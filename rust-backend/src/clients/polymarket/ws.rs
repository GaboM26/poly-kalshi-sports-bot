//! Polymarket US markets WebSocket (`wss://api.polymarket.us/v1/ws/markets`).
//!
//! Push-based replacement for the REST gateway, whose responses are
//! Cloudflare-cached for up to 30s. Subscribes to
//! `SUBSCRIPTION_TYPE_MARKET_DATA_LITE`, whose `longQuote`/`shortQuote` are
//! the same per-side quotes the gateway feed exposes as
//! `marketSides[].quote` (verified live), and to `SUBSCRIPTION_TYPE_MARKET_DATA`
//! full books. Execution sizes from those books only while the connection
//! that delivered them is provably alive (`WebSocketManager::
//! fresh_polymarket_book`), falling back to a cache-bypassing REST fetch.
//!
//! Auth is the same Ed25519 scheme as REST: sign `{timestamp}GET{path}`.

use std::collections::HashSet;
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::{
    connect_async, tungstenite::client::IntoClientRequest, tungstenite::Message,
};
use tracing::{debug, info, warn};

use super::{build_market_book, PolymarketMarketBook};
use super::auth;
use crate::config::PolymarketWsConfig;

const WS_URL: &str = "wss://api.polymarket.us/v1/ws/markets";
const WS_PATH: &str = "/v1/ws/markets";
/// Documented cap: 100 markets per subscription.
const MAX_SLUGS_PER_SUBSCRIPTION: usize = 100;
/// Server limit, observed live 2026-10-27: the 11th subscribe on one
/// connection is answered `"error":"max subscriptions per connection
/// reached"` and never streams. Every hot-add uses 2 (lite + book), so
/// without this the markets added after the first few (every new crypto
/// window) silently got no WS data and detection ran on 30s-stale gateway
/// quotes.
const MAX_SUBSCRIPTIONS_PER_CONNECTION: usize = 10;
/// The server sends heartbeats; silence beyond this means a dead socket.

/// A per-side quote update for one market slug.
#[derive(Debug, Clone, PartialEq)]
pub struct PolyWsQuote {
    pub slug: String,
    pub long_quote: f64,
    pub short_quote: f64,
}

/// One decoded stream message, or a connection lifecycle change.
#[derive(Debug, Clone)]
pub enum PolyWsEvent {
    /// Lightweight per-side quotes (detection prices).
    Quote(PolyWsQuote),
    /// Full order book snapshot. Each one replaces the stored book; the
    /// server sends one on subscribe and again on every change, so on a
    /// live connection the latest snapshot is the current book.
    Book(PolymarketMarketBook),
    /// A new connection is up. Everything stored from an earlier connection
    /// is no longer provably current until this one re-sends it.
    Connected,
    /// Any other frame (heartbeat, subscription ack, ping): proof the
    /// connection is still alive even when no subscribed market changes.
    Heartbeat,
    /// The connection died; a reconnect attempt follows.
    Disconnected,
}

fn price(value: &Value) -> Option<f64> {
    let v = value.get("value").unwrap_or(value);
    v.as_f64().or_else(|| v.as_str()?.parse().ok())
}

/// Parse a raw WS text frame. Returns `None` for heartbeats, acks, full
/// order-book messages, closed/expired markets, or anything missing a
/// quote - the caller then simply leaves the previous quote in place.
pub fn parse_lite_message(text: &str) -> Option<PolyWsQuote> {
    let root: Value = serde_json::from_str(text).ok()?;
    let lite = root.get("marketDataLite")?;
    if lite.get("state").and_then(Value::as_str) != Some("MARKET_STATE_OPEN") {
        return None;
    }
    let long_quote = price(lite.get("longQuote")?)?;
    let short_quote = price(lite.get("shortQuote")?)?;
    // Mirror the REST parser's validity window.
    if !(0.01..=0.99).contains(&long_quote) || !(0.01..=0.99).contains(&short_quote) {
        return None;
    }
    Some(PolyWsQuote {
        slug: lite.get("marketSlug")?.as_str()?.to_string(),
        long_quote,
        short_quote,
    })
}

/// Parse a full `marketData` book message. Each message is a complete
/// snapshot (verified live), so the caller replaces its stored book.
pub fn parse_book_message(text: &str) -> Option<PolymarketMarketBook> {
    let root: Value = serde_json::from_str(text).ok()?;
    let data = root.get("marketData")?;
    let slug = data.get("marketSlug")?.as_str()?;
    build_market_book(&json!({ "marketData": data }), slug).ok()
}

fn parse_event(text: &str) -> Option<PolyWsEvent> {
    parse_lite_message(text)
        .map(PolyWsEvent::Quote)
        .or_else(|| parse_book_message(text).map(PolyWsEvent::Book))
}

const SUBSCRIPTION_TYPES: [(&str, &str); 2] = [
    ("lite", "SUBSCRIPTION_TYPE_MARKET_DATA_LITE"),
    ("book", "SUBSCRIPTION_TYPE_MARKET_DATA"),
];

fn subscribe_messages(slugs: &[String], next_id: &mut u64) -> Vec<Message> {
    let mut messages = Vec::new();
    for chunk in slugs.chunks(MAX_SLUGS_PER_SUBSCRIPTION) {
        for (label, subscription_type) in SUBSCRIPTION_TYPES {
            *next_id += 1;
            messages.push(Message::Text(
                json!({"subscribe": {
                    "requestId": format!("{}-{}", label, next_id),
                    "subscriptionType": subscription_type,
                    "marketSlugs": chunk,
                }})
                .to_string(),
            ));
        }
    }
    messages
}

/// Subscriptions (lite + book per 100-slug chunk) needed for `slugs` markets.
fn subscription_count(slugs: usize) -> usize {
    slugs.div_ceil(MAX_SLUGS_PER_SUBSCRIPTION) * SUBSCRIPTION_TYPES.len()
}

/// The server's error for a rejected request, e.g. a subscribe over the cap.
fn subscription_error(text: &str) -> Option<String> {
    let root: Value = serde_json::from_str(text).ok()?;
    let error = root.get("error")?.as_str()?;
    Some(format!(
        "{} (request {}, {})",
        error,
        root.get("requestId").and_then(Value::as_str).unwrap_or("?"),
        root.get("subscriptionType").and_then(Value::as_str).unwrap_or("?")
    ))
}

/// One connection attempt. Returns when the socket dies or goes silent.
async fn run_once<S, Q>(
    timing: &PolymarketWsConfig,
    key_id: &str,
    secret_key: &str,
    slugs: &S,
    on_quote: &Q,
) -> Result<()>
where
    S: Fn() -> Vec<String>,
    Q: Fn(PolyWsEvent),
{
    let headers = auth::sign_request(key_id, secret_key, "GET", WS_PATH)?;
    let mut request = WS_URL.into_client_request()?;
    let h = request.headers_mut();
    h.insert("X-PM-Access-Key", headers.access_key.parse()?);
    h.insert("X-PM-Timestamp", headers.timestamp.parse()?);
    h.insert("X-PM-Signature", headers.signature.parse()?);

    let (stream, _) = connect_async(request)
        .await
        .context("Failed to connect to the Polymarket US WebSocket")?;
    let (mut write, mut read) = stream.split();
    on_quote(PolyWsEvent::Connected);

    let mut subscribed: HashSet<String> = HashSet::new();
    let mut next_id = 0u64;
    let mut subscriptions = 0usize;
    let mut refresh = tokio::time::interval(timing.slug_refresh());

    loop {
        tokio::select! {
            _ = refresh.tick() => {
                // Subscribe to newly matched slugs. There is no documented
                // unsubscribe; slugs that drop out stay subscribed until the
                // next reconnect, which is harmless (their updates are
                // ignored once no matched market carries the slug).
                let new: Vec<String> = slugs()
                    .into_iter()
                    .filter(|s| !subscribed.contains(s))
                    .collect();
                if !new.is_empty() {
                    let messages = subscribe_messages(&new, &mut next_id);
                    if subscriptions + messages.len() > MAX_SUBSCRIPTIONS_PER_CONNECTION {
                        let all = subscribed.len() + new.len();
                        if subscription_count(all) <= MAX_SUBSCRIPTIONS_PER_CONNECTION {
                            // A fresh connection packs every slug into full
                            // 100-slug chunks, so all of them fit again.
                            anyhow::bail!(
                                "subscription cap reached with {} new markets; reconnecting to resubscribe all {}",
                                new.len(), all
                            );
                        }
                        // Even a fresh connection cannot carry every slug:
                        // fill the remaining slots (whole chunks, lite +
                        // book together) and leave the rest on REST.
                        let free = MAX_SUBSCRIPTIONS_PER_CONNECTION - subscriptions;
                        warn!(
                            "📡 Polymarket WS covers only part of {} new markets: {} markets need more than {} subscriptions on one connection",
                            new.len(), all, MAX_SUBSCRIPTIONS_PER_CONNECTION
                        );
                        subscriptions += free;
                        for msg in messages.into_iter().take(free) {
                            write.send(msg).await.context("WS subscribe send failed")?;
                        }
                        subscribed.extend(new);
                        continue;
                    }
                    subscriptions += messages.len();
                    for msg in messages {
                        write.send(msg).await.context("WS subscribe send failed")?;
                    }
                    info!("📡 Polymarket WS subscribed to {} new markets ({} total, {} subscriptions)",
                        new.len(), subscribed.len() + new.len(), subscriptions);
                    subscribed.extend(new);
                }
            }
            frame = tokio::time::timeout(timing.silence_timeout(), read.next()) => {
                let frame = frame.context("Polymarket WS silent for 30s")?;
                match frame {
                    Some(Ok(Message::Text(text))) => {
                        if let Some(error) = subscription_error(&text) {
                            warn!("Polymarket WS subscription error: {}", error);
                        }
                        on_quote(parse_event(&text).unwrap_or(PolyWsEvent::Heartbeat));
                    }
                    Some(Ok(Message::Close(frame))) => {
                        anyhow::bail!("Polymarket WS closed by server: {:?}", frame);
                    }
                    Some(Ok(_)) => on_quote(PolyWsEvent::Heartbeat),
                    Some(Err(e)) => return Err(e).context("Polymarket WS read error"),
                    None => anyhow::bail!("Polymarket WS stream ended"),
                }
            }
        }
    }
}

/// Run forever, reconnecting with exponential backoff (1s up to
/// `max_backoff_secs`). `slugs`
/// supplies the current desired subscription set; `on_quote` receives each
/// decoded quote or book event.
pub async fn run_markets_stream<S, Q>(
    timing: PolymarketWsConfig,
    key_id: String,
    secret_key: String,
    slugs: S,
    on_quote: Q,
)
where
    S: Fn() -> Vec<String>,
    Q: Fn(PolyWsEvent),
{
    let mut backoff = Duration::from_secs(1);
    loop {
        let started = std::time::Instant::now();
        match run_once(&timing, &key_id, &secret_key, &slugs, &on_quote).await {
            Ok(()) => debug!("Polymarket WS loop returned"),
            Err(e) => warn!("Polymarket WS disconnected: {:#}. Reconnecting in {:?}", e, backoff),
        }
        on_quote(PolyWsEvent::Disconnected);
        // A connection that stayed up a while earns a fresh backoff.
        if started.elapsed() > Duration::from_secs(60) {
            backoff = Duration::from_secs(1);
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(timing.max_backoff());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPEN: &str = r#"{"requestId":"r0","subscriptionType":"SUBSCRIPTION_TYPE_MARKET_DATA_LITE","marketDataLite":{"marketSlug":"s1","currentPx":{"value":"0.1050","currency":"USD"},"bestAsk":{"value":"0.1100","currency":"USD"},"bestBid":{"value":"0.1000","currency":"USD"},"longQuote":{"value":"0.1100","currency":"USD"},"shortQuote":{"value":"0.9","currency":"USD"},"state":"MARKET_STATE_OPEN"}}"#;

    #[test]
    fn parses_an_open_market_quote() {
        let q = parse_lite_message(OPEN).unwrap();
        assert_eq!(q.slug, "s1");
        assert!((q.long_quote - 0.11).abs() < 1e-9);
        assert!((q.short_quote - 0.9).abs() < 1e-9);
    }

    #[test]
    fn ignores_non_open_markets() {
        let expired = OPEN.replace("MARKET_STATE_OPEN", "MARKET_STATE_EXPIRED");
        assert!(parse_lite_message(&expired).is_none());
    }

    #[test]
    fn ignores_out_of_range_quotes() {
        assert!(parse_lite_message(&OPEN.replace(r#""0.9""#, r#""1.00""#)).is_none());
        assert!(parse_lite_message(&OPEN.replace(r#""0.1100","currency":"USD"},"shortQuote""#, r#""0.00","currency":"USD"},"shortQuote""#)).is_none());
    }

    #[test]
    fn ignores_heartbeats_acks_and_full_book_messages() {
        assert!(parse_lite_message(r#"{"heartbeat":{}}"#).is_none());
        assert!(parse_lite_message(r#"{"marketData":{"marketSlug":"s1","bids":[],"offers":[]}}"#).is_none());
        assert!(parse_lite_message("not json").is_none());
    }

    #[test]
    fn ignores_missing_quotes() {
        let no_quote = OPEN.replace("longQuote", "somethingElse");
        assert!(parse_lite_message(&no_quote).is_none());
    }

    #[test]
    fn chunks_subscriptions_at_100_slugs() {
        let slugs: Vec<String> = (0..250).map(|i| format!("s{i}")).collect();
        let mut id = 0;
        // Two subscription types (lite quotes + full book) per chunk.
        assert_eq!(subscribe_messages(&slugs, &mut id).len(), 6);
        assert_eq!(id, 6);
    }

    #[test]
    fn counts_subscriptions_and_reads_server_errors() {
        assert_eq!(subscription_count(0), 0);
        assert_eq!(subscription_count(100), 2);
        assert_eq!(subscription_count(320), 8);
        assert_eq!(subscription_count(501), 12);
        let err = r#"{"requestId":"r11", "error":"max subscriptions per connection reached", "subscriptionType":"SUBSCRIPTION_TYPE_MARKET_DATA_LITE"}"#;
        assert_eq!(
            subscription_error(err).as_deref(),
            Some("max subscriptions per connection reached (request r11, SUBSCRIPTION_TYPE_MARKET_DATA_LITE)")
        );
        assert!(subscription_error(OPEN).is_none());
    }

    #[test]
    fn parses_a_full_book_snapshot_and_rejects_bad_levels() {
        let book = r#"{"marketData":{"marketSlug":"s1","bids":[{"px":{"value":"0.10"},"qty":"5"},{"px":{"value":"0.20"},"qty":"3"}],"offers":[{"px":{"value":"0.30"},"qty":"7"}],"state":"MARKET_STATE_OPEN"}}"#;
        let b = parse_book_message(book).unwrap();
        assert_eq!(b.market_slug, "s1");
        assert_eq!(b.bids[0].price, 0.20); // sorted best-first
        assert_eq!(b.offers[0].quantity, 7.0);
        // A lite message is not a book, and vice versa.
        assert!(parse_book_message(OPEN).is_none());
        assert!(parse_lite_message(book).is_none());
        let bad = book.replace(r#""qty":"7""#, r#""qty":"0""#);
        assert!(parse_book_message(&bad).is_none());
    }
}
