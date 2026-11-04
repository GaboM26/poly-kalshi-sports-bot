//! Kalshi WebSocket: signed connect, subscriptions, order book deltas and sequence checks.

use super::*;

pub(super) const KALSHI_WS_URL: &str = "wss://external-api-ws.kalshi.com/trade-api/ws/v2";

/// Subscription command for Kalshi WebSocket
#[derive(Debug, Clone)]
pub enum KalshiWsCommand {
    Subscribe(Vec<String>),
    Unsubscribe(Vec<String>),
}

/// A missing `orderbook_delta` (non-consecutive `seq`) for one ticker.
#[derive(Debug, PartialEq)]
pub struct SeqGap {
    pub ticker: String,
    pub expected: u64,
    pub got: u64,
}

/// The server dropped our book subscription while keeping the socket open:
/// `{"type":"error","msg":{"code":25,"msg":"Subscription buffer overflow"}}`
/// (we fell behind), followed by `{"type":"unsubscribed","sid":1}`. Seen live
/// 2026-10-27, when book updates collapsed from ~330 markets to ~4 and the
/// rest froze. This client never sends an unsubscribe, so any `unsubscribed`
/// is server-initiated.
pub(super) fn is_subscription_dropped(frame: &Value) -> bool {
    match frame["type"].as_str() {
        Some("error") => frame["msg"]["code"].as_u64() == Some(25),
        Some("unsubscribed") => true,
        _ => false,
    }
}

impl KalshiClient {
    /// Subscribe to additional markets dynamically (hot subscription)
    ///
    /// This can be called after the WebSocket connection is established
    /// to add new market subscriptions.
    pub async fn subscribe_markets(&self, tickers: Vec<String>) -> Result<bool> {
        if tickers.is_empty() {
            return Ok(true);
        }

        let tx = self.command_tx.read().clone();
        if let Some(tx) = tx {
            match tx.send(KalshiWsCommand::Subscribe(tickers.clone())).await {
                Ok(_) => {
                    info!(
                        "🔌 [Kalshi] Sending hot-subscription request for {} markets",
                        tickers.len()
                    );
                    Ok(true)
                }
                Err(e) => {
                    warn!("⚠️ [Kalshi] Hot-subscription request failed: {}", e);
                    Ok(false)
                }
            }
        } else {
            warn!("⚠️ [Kalshi] WebSocket is not connected; cannot hot-subscribe");
            Ok(false)
        }
    }

    /// Unsubscribe from markets dynamically
    ///
    /// This can be called after the WebSocket connection is established
    /// to remove market subscriptions for ended games.
    pub async fn unsubscribe_markets(&self, tickers: Vec<String>) -> Result<bool> {
        if tickers.is_empty() {
            return Ok(true);
        }

        let tx = self.command_tx.read().clone();
        if let Some(tx) = tx {
            match tx.send(KalshiWsCommand::Unsubscribe(tickers.clone())).await {
                Ok(_) => {
                    info!(
                        "🔌 [Kalshi] Sending unsubscribe request for {} markets",
                        tickers.len()
                    );
                    Ok(true)
                }
                Err(e) => {
                    warn!("⚠️ [Kalshi] Unsubscribe request failed: {}", e);
                    Ok(false)
                }
            }
        } else {
            warn!("⚠️ [Kalshi] WebSocket is not connected; cannot unsubscribe");
            Ok(false)
        }
    }

    /// Directional (mention) variant of `subscribe_markets`. Records the tickers so they
    /// are re-subscribed after a reconnect, then delegates to the shared hot-subscribe.
    pub async fn subscribe_directional_markets(&self, tickers: Vec<String>) -> Result<bool> {
        self.directional_subscriptions
            .write()
            .extend(tickers.iter().cloned());
        self.subscribe_markets(tickers).await
    }

    /// Directional (mention) variant of `unsubscribe_markets`.
    pub async fn unsubscribe_directional_markets(&self, tickers: Vec<String>) -> Result<bool> {
        {
            let mut subscriptions = self.directional_subscriptions.write();
            for ticker in &tickers {
                subscriptions.remove(ticker);
            }
        }
        self.unsubscribe_markets(tickers).await
    }

    /// Signed WebSocket upgrade request (auth goes in HTTP headers).
    pub fn websocket_request(
        &self,
    ) -> Result<tokio_tungstenite::tungstenite::handshake::client::Request> {
        let timestamp = Self::get_timestamp_ms();
        let signature = self.sign_request(timestamp, "GET", "/trade-api/ws/v2");

        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let mut request = KALSHI_WS_URL.into_client_request()?;
        request
            .headers_mut()
            .insert("KALSHI-ACCESS-KEY", self.config.api_key.parse().unwrap());
        request
            .headers_mut()
            .insert("KALSHI-ACCESS-SIGNATURE", signature.parse().unwrap());
        request.headers_mut().insert(
            "KALSHI-ACCESS-TIMESTAMP",
            timestamp.to_string().parse().unwrap(),
        );
        Ok(request)
    }

    /// Connect to WebSocket for real-time updates
    pub async fn connect_websocket(
        &self,
        tickers: Vec<String>,
        price_tx: mpsc::Sender<PriceUpdate>,
    ) -> Result<()> {
        let mut tickers = tickers;
        {
            // Re-subscribe directional (mention) tickers after a reconnect. The set is
            // empty unless the directional service is running, leaving the paired
            // path's ticker list exactly as the caller supplied it.
            let directional = self.directional_subscriptions.read();
            for ticker in directional.iter() {
                if !tickers.contains(ticker) {
                    tickers.push(ticker.clone());
                }
            }
        }
        let request = self.websocket_request()?;

        info!("Connecting to the Kalshi WebSocket...");

        let (ws_stream, _) = connect_async(request)
            .await
            .with_context(|| "连接 Kalshi WebSocket 失败")?;

        // A fresh connection means no book we've cached so far can be
        // vouched for until it's rewritten under this epoch - see
        // `get_fresh_orderbook`.
        let current_epoch = self.connection_epoch.fetch_add(1, Ordering::SeqCst) + 1;
        *self.last_message_at.write() = Some(Instant::now());

        let (mut write, mut read) = ws_stream.split();

        // Create channel for dynamic subscriptions/unsubscriptions
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<KalshiWsCommand>(100);
        *self.command_tx.write() = Some(cmd_tx);

        // One subscribe for every initial ticker. Subscribing one ticker per
        // message made Kalshi answer each with an `ok` listing the whole
        // growing ticker set (~3 MB for ~400 tickers, measured 2026-10-27),
        // arriving with the snapshots and pushing the reader behind. An
        // empty list is never sent: it could mean "every market".
        let mut next_msg_id = 1;
        if !tickers.is_empty() {
            let subscribe_msg = json!({
                "id": next_msg_id,
                "cmd": "subscribe",
                "params": {
                    "channels": ["orderbook_delta"],
                    "market_tickers": tickers
                }
            });
            next_msg_id += 1;
            write.send(Message::Text(subscribe_msg.to_string())).await?;
        }

        info!("Subscribed to {} Kalshi markets", tickers.len());

        let orderbook_cache = self.orderbook_cache.clone();
        // Last `seq` seen per subscription id, scoped to this connection.
        let mut last_seq_by_sid: HashMap<u64, u64> = HashMap::new();
        let mut gaps_logged = 0u32;
        let mut headers_logged = 0u32;
        // Observe-only feed lag: exchange `ts_ms` on deltas vs local receipt,
        // and how full the price channel is (a full channel blocks this loop
        // and every cached book falls behind).
        let mut lag_window_start = Instant::now();
        let mut lag_samples: Vec<i64> = Vec::new();
        let mut lag_crypto: Vec<i64> = Vec::new();

        // Process messages with dynamic subscription/unsubscription support
        loop {
            tokio::select! {
                // Handle incoming WebSocket messages
                msg = read.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            *self.last_message_at.write() = Some(Instant::now());
                            if let Some(ts_ms) = text
                                .find("\"ts_ms\":")
                                .and_then(|at| text[at + 8..].split(|c: char| !c.is_ascii_digit()).next())
                                .and_then(|digits| digits.parse::<i64>().ok())
                            {
                                let lag = Utc::now().timestamp_millis() - ts_ms;
                                lag_samples.push(lag);
                                if text.contains("\"market_ticker\":\"KXBTC15M")
                                    || text.contains("\"market_ticker\":\"KXETH15M")
                                    || text.contains("\"market_ticker\":\"KXSOL15M")
                                {
                                    lag_crypto.push(lag);
                                }
                            }
                            if lag_window_start.elapsed() >= Duration::from_secs(30) {
                                if !lag_samples.is_empty() {
                                    lag_samples.sort_unstable();
                                    let pick = |q: f64| lag_samples[((lag_samples.len() - 1) as f64 * q) as usize];
                                    info!(
                                        "Kalshi WS lag over 30s: {} deltas, p50 {}ms, p95 {}ms, max {}ms; price channel free {}/{}",
                                        lag_samples.len(),
                                        pick(0.5),
                                        pick(0.95),
                                        pick(1.0),
                                        price_tx.capacity(),
                                        price_tx.max_capacity()
                                    );
                                }
                                if !lag_crypto.is_empty() {
                                    lag_crypto.sort_unstable();
                                    let pick = |q: f64| lag_crypto[((lag_crypto.len() - 1) as f64 * q) as usize];
                                    info!(
                                        "Kalshi WS lag (crypto only): {} deltas, p50 {}ms, p95 {}ms",
                                        lag_crypto.len(),
                                        pick(0.5),
                                        pick(0.95)
                                    );
                                }
                                lag_crypto.clear();
                                lag_samples.clear();
                                lag_window_start = Instant::now();
                            }
                            if headers_logged < 25 {
                                headers_logged += 1;
                                if let Ok(v) = serde_json::from_str::<Value>(&text) {
                                    info!(
                                        "Kalshi ws header: type={} sid={} seq={} ticker={}",
                                        v["type"], v["sid"], v["seq"], v["msg"]["market_ticker"]
                                    );
                                }
                            }
                            // Every non-book frame (acks, errors, unsubscribes)
                            // is rare and explains a change in what the server
                            // sends; on 2026-10-27 book updates collapsed from
                            // ~330 to ~4 markets on a live connection with no
                            // visible cause. An `ok` carries the subscription's
                            // whole ticker list, so log only its size.
                            if !text.contains("\"type\":\"orderbook_delta\"")
                                && !text.contains("\"type\":\"orderbook_snapshot\"")
                            {
                                if let Ok(v) = serde_json::from_str::<Value>(&text) {
                                    match v["msg"]["market_tickers"].as_array() {
                                        Some(list) => info!(
                                            "Kalshi ws control: type={} id={} sid={} subscribed_tickers={}",
                                            v["type"], v["id"], v["sid"], list.len()
                                        ),
                                        None => info!(
                                            "Kalshi ws control: {}",
                                            text.chars().take(300).collect::<String>()
                                        ),
                                    }
                                    if is_subscription_dropped(&v) {
                                        // The server dropped our subscription but
                                        // keeps the socket open: every book would
                                        // freeze while looking current. Reconnect
                                        // for a fresh subscription and snapshots.
                                        error!("Kalshi WS subscription dropped by the server; reconnecting");
                                        break;
                                    }
                                }
                            }
                            match Self::parse_ws_message(&text, &orderbook_cache, current_epoch, &mut last_seq_by_sid) {
                                Some(Ok(update)) => {
                                    if price_tx.send(update).await.is_err() {
                                        warn!("Price update channel has closed");
                                        break;
                                    }
                                }
                                Some(Err(gap)) => {
                                    // Observation only: the live feed's `seq`
                                    // semantics are not yet understood (false
                                    // gaps on every connect), so a gap must not
                                    // reconnect or untrust books. Log the first
                                    // few per connection for diagnosis.
                                    gaps_logged += 1;
                                    if gaps_logged <= 5 {
                                        warn!(
                                            "Kalshi seq discontinuity (not acted on): {} expected {}, got {}",
                                            gap.ticker, gap.expected, gap.got
                                        );
                                    }
                                }
                                None => {}
                            }
                        }
                        Some(Ok(Message::Close(_))) => {
                            info!("Kalshi WebSocket closed");
                            break;
                        }
                        Some(Err(e)) => {
                            error!("Kalshi WebSocket error: {:#}", e);
                            break;
                        }
                        None => {
                            info!("Kalshi WebSocket stream ended");
                            break;
                        }
                        Some(Ok(_)) => {
                            // Ping/Pong/Binary frames still confirm the
                            // connection is alive.
                            *self.last_message_at.write() = Some(Instant::now());
                        }
                    }
                }
                // Handle dynamic subscription/unsubscription requests
                Some(command) = cmd_rx.recv() => {
                    match command {
                        KalshiWsCommand::Subscribe(new_tickers) => {
                            info!("🔌 [Kalshi] Processing hot subscription for {} new markets", new_tickers.len());
                            for ticker in new_tickers.iter() {
                                let subscribe_msg = json!({
                                    "id": next_msg_id,
                                    "cmd": "subscribe",
                                    "params": {
                                        "channels": ["orderbook_delta"],
                                        "market_ticker": ticker
                                    }
                                });
                                next_msg_id += 1;

                                if let Err(e) = write.send(Message::Text(subscribe_msg.to_string())).await {
                                    error!("❌ [Kalshi] Hot subscription send failed: {:#}", e);
                                }
                            }
                            info!("✅ [Kalshi] Hot subscription completed for {} markets", new_tickers.len());
                        }
                        KalshiWsCommand::Unsubscribe(tickers_to_unsub) => {
                            let ticker_count = tickers_to_unsub.len();
                            info!("🔌 [Kalshi] Processing unsubscribe for {} markets", ticker_count);
                            // Every ticker on the live feed shares one sid
                            // (only the first subscribe is acked `subscribed`;
                            // the rest are `ok` on sid 1), so a sid unsubscribe
                            // for an ended market froze every other book while
                            // the connection stayed alive. An ended market is
                            // closed and sends nothing, so dropping its cached
                            // book is enough; no unsubscribe is sent.
                            for ticker in tickers_to_unsub {
                                orderbook_cache.write().remove(&ticker);
                            }
                            info!("✅ [Kalshi] Unsubscribe completed for {} markets", ticker_count);
                        }
                    }
                }
            }
        }

        // Clear command channel on disconnect
        *self.command_tx.write() = None;

        Ok(())
    }

    /// Associate Kalshi's server subscription ID with the market that initiated it.
    /// Parse WebSocket message. `epoch` is the current connection's epoch
    /// (see `KalshiClient::connection_epoch`) and is stamped onto any book
    /// this message writes, so `get_fresh_orderbook` can tell it was built
    /// under the connection that's still active.
    ///
    /// `Some(Err(SeqGap))` means a delta was lost: the book is marked
    /// untrusted (so `get_fresh_orderbook` refuses it) and the caller must
    /// reconnect, which re-subscribes everything and brings fresh snapshots.
    pub(super) fn parse_ws_message(
        text: &str,
        orderbook_cache: &Arc<RwLock<HashMap<String, OrderBook>>>,
        epoch: u64,
        last_seq_by_sid: &mut HashMap<u64, u64>,
    ) -> Option<Result<PriceUpdate, SeqGap>> {
        let data: Value = serde_json::from_str(text).ok()?;
        let msg_type = data.get("type")?.as_str()?;

        // `seq` counts EVERY message on a subscription id (`sid`) - snapshots,
        // deltas and the `ok` acknowledgements alike - and one sid carries all
        // tickers. Verified on the live feed: sid=1, seq 1..N across mixed
        // `orderbook_snapshot` / `ok` messages. Counting only book messages
        // (or per ticker) reported false gaps on every connect. A skipped
        // number on a sid means a message was lost.
        {
            if let (Some(sid), Some(got)) = (
                data.get("sid").and_then(Value::as_u64),
                data.get("seq").and_then(Value::as_u64),
            ) {
                let previous = last_seq_by_sid.insert(sid, got);
                if let Some(last) = previous {
                    if got != last + 1 {
                        let ticker = data
                            .get("msg")
                            .and_then(|m| m.get("market_ticker"))
                            .and_then(Value::as_str)
                            .unwrap_or("?")
                            .to_string();
                        return Some(Err(SeqGap {
                            ticker,
                            expected: last + 1,
                            got,
                        }));
                    }
                }
            }
        }

        match msg_type {
            "orderbook_snapshot" => {
                let msg = data.get("msg")?;
                let ticker = msg.get("market_ticker")?.as_str()?;

                let mut book = OrderBook::from_exact(
                    parse_price_levels(msg, "yes_dollars_fp", "yes"),
                    parse_price_levels(msg, "no_dollars_fp", "no"),
                );
                book.updated_at = Some(Instant::now());
                book.epoch = epoch;

                orderbook_cache
                    .write()
                    .insert(ticker.to_string(), book.clone());

                // Calculate prices
                let yes_bid = book.yes.last().map(|(p, _)| *p as f64 / 100.0);
                let no_bid = book.no.last().map(|(p, _)| *p as f64 / 100.0);

                if let (Some(yb), Some(nb)) = (yes_bid, no_bid) {
                    Some(Ok(PriceUpdate {
                        platform: Platform::Kalshi,
                        market_id: ticker.to_string(),
                        yes_bid: Some(yb),
                        yes_ask: Some(1.0 - nb),
                        no_bid: Some(nb),
                        no_ask: Some(1.0 - yb),
                        timestamp: Utc::now(),
                    }))
                } else {
                    None
                }
            }
            "orderbook_delta" => {
                let msg = data.get("msg")?;
                let ticker = msg.get("market_ticker")?.as_str()?;
                let price = msg
                    .get("price_dollars")
                    .and_then(book_bid_cents)
                    .or_else(|| {
                        msg.get("price")
                            .and_then(Value::as_i64)
                            .map(|price| price as i32)
                    });
                let delta = msg
                    .get("delta_fp")
                    .and_then(fixed_point_to_centi)
                    .or_else(|| {
                        msg.get("delta")
                            .and_then(Value::as_i64)
                            .map(|delta| delta * 100)
                    });
                let side = msg.get("side").and_then(Value::as_str);
                let (Some(price), Some(delta), Some(side)) = (price, delta, side) else {
                    // A dropped delta leaves a removed level in the cached
                    // book forever, so it must never be silent.
                    warn!("Kalshi orderbook_delta dropped (unparsable) for {}: {}", ticker, msg);
                    return None;
                };

                // Apply delta
                let mut cache = orderbook_cache.write();
                let Some(book) = cache.get_mut(ticker) else {
                    warn!("Kalshi orderbook_delta for {} dropped: no snapshot cached", ticker);
                    return None;
                };
                // Never build on a book left over from a previous connection
                // - wait for the fresh snapshot this epoch's subscribe will
                // bring instead of layering a delta on a stale base.
                if book.epoch != epoch {
                    return None;
                }
                book.apply_delta(side, price, delta);
                book.updated_at = Some(Instant::now());
                book.epoch = epoch;

                // Recalculate prices
                let yes_bid = book.yes.last().map(|(p, _)| *p as f64 / 100.0);
                let no_bid = book.no.last().map(|(p, _)| *p as f64 / 100.0);

                drop(cache);

                if let (Some(yb), Some(nb)) = (yes_bid, no_bid) {
                    Some(Ok(PriceUpdate {
                        platform: Platform::Kalshi,
                        market_id: ticker.to_string(),
                        yes_bid: Some(yb),
                        yes_ask: Some(1.0 - nb),
                        no_bid: Some(nb),
                        no_ask: Some(1.0 - yb),
                        timestamp: Utc::now(),
                    }))
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_a_server_dropped_subscription_only() {
        let overflow: Value = serde_json::from_str(
            r#"{"type":"error","sid":1,"seq":4912141,"msg":{"code":25,"msg":"Subscription buffer overflow"},"sending_ts_ms":1793122937806}"#,
        )
        .unwrap();
        assert!(is_subscription_dropped(&overflow));
        let unsubscribed: Value =
            serde_json::from_str(r#"{"type":"unsubscribed","sid":1,"seq":4912142,"sending_ts_ms":1793122937806}"#).unwrap();
        assert!(is_subscription_dropped(&unsubscribed));
        let other_error: Value =
            serde_json::from_str(r#"{"type":"error","id":7,"msg":{"code":8,"msg":"Unknown market"}}"#).unwrap();
        assert!(!is_subscription_dropped(&other_error));
        let ok: Value = serde_json::from_str(r#"{"type":"ok","id":2,"sid":1,"seq":2,"msg":{"market_tickers":[]}}"#).unwrap();
        assert!(!is_subscription_dropped(&ok));
    }

    fn ws_cache() -> Arc<RwLock<HashMap<String, OrderBook>>> {
        Arc::new(RwLock::new(HashMap::new()))
    }

    fn snapshot(sid: u64, seq: u64, ticker: &str) -> String {
        format!(
            r#"{{"type":"orderbook_snapshot","sid":{sid},"seq":{seq},"msg":{{"market_ticker":"{ticker}","yes_dollars_fp":[["0.40","10.00"]],"no_dollars_fp":[["0.55","10.00"]]}}}}"#
        )
    }

    fn delta(sid: u64, seq: u64, ticker: &str) -> String {
        format!(
            r#"{{"type":"orderbook_delta","sid":{sid},"seq":{seq},"msg":{{"market_ticker":"{ticker}","price_dollars":"0.41","delta_fp":"5.00","side":"yes"}}}}"#
        )
    }

    fn parse(
        text: &str,
        cache: &Arc<RwLock<HashMap<String, OrderBook>>>,
        seqs: &mut HashMap<u64, u64>,
    ) -> Option<Result<PriceUpdate, SeqGap>> {
        KalshiClient::parse_ws_message(text, cache, 1, seqs)
    }

    #[test]
    fn seq_is_consecutive_per_subscription_across_tickers() {
        // One sid carries several tickers; its seq advances on every message
        // regardless of ticker. This is what the live feed does.
        let cache = ws_cache();
        let mut seqs = HashMap::new();
        assert!(matches!(parse(&snapshot(1, 5, "A"), &cache, &mut seqs), Some(Ok(_))));
        assert!(matches!(parse(&snapshot(1, 6, "B"), &cache, &mut seqs), Some(Ok(_))));
        assert!(matches!(parse(&delta(1, 7, "A"), &cache, &mut seqs), Some(Ok(_))));
        assert!(matches!(parse(&delta(1, 8, "B"), &cache, &mut seqs), Some(Ok(_))));
        assert_eq!(cache.read().get("A").unwrap().epoch, 1);
    }

    #[test]
    fn ok_acknowledgements_consume_sequence_numbers() {
        // Live feed: snapshot seq=1, then `ok` acks seq=2..3, then the next
        // snapshot is seq=4. The acks must count or this looks like a gap.
        let cache = ws_cache();
        let mut seqs = HashMap::new();
        let ok = |seq: u64| format!(r#"{{"type":"ok","sid":1,"seq":{seq},"msg":{{}}}}"#);
        assert!(matches!(parse(&snapshot(1, 1, "A"), &cache, &mut seqs), Some(Ok(_))));
        parse(&ok(2), &cache, &mut seqs);
        parse(&ok(3), &cache, &mut seqs);
        assert!(matches!(parse(&snapshot(1, 4, "B"), &cache, &mut seqs), Some(Ok(_))));
        // A skipped ack is a real gap.
        let Some(Err(gap)) = parse(&snapshot(1, 6, "B"), &cache, &mut seqs) else {
            panic!("expected a gap");
        };
        assert_eq!((gap.expected, gap.got), (5, 6));
    }

    #[test]
    fn separate_subscriptions_keep_separate_sequences() {
        let cache = ws_cache();
        let mut seqs = HashMap::new();
        assert!(matches!(parse(&snapshot(1, 1, "A"), &cache, &mut seqs), Some(Ok(_))));
        assert!(matches!(parse(&snapshot(2, 1, "B"), &cache, &mut seqs), Some(Ok(_))));
        assert!(matches!(parse(&delta(1, 2, "A"), &cache, &mut seqs), Some(Ok(_))));
        assert!(matches!(parse(&delta(2, 2, "B"), &cache, &mut seqs), Some(Ok(_))));
    }

    #[test]
    fn a_skipped_seq_is_reported_as_a_gap() {
        let cache = ws_cache();
        let mut seqs = HashMap::new();
        parse(&snapshot(1, 5, "T"), &cache, &mut seqs);
        parse(&delta(1, 6, "T"), &cache, &mut seqs);
        let Some(Err(gap)) = parse(&delta(1, 8, "T"), &cache, &mut seqs) else {
            panic!("expected a sequence gap");
        };
        assert_eq!(
            gap,
            SeqGap {
                ticker: "T".to_string(),
                expected: 7,
                got: 8
            }
        );
        // Observation only for now: the book stays trusted.
        assert_eq!(cache.read().get("T").unwrap().epoch, 1);
    }
}
