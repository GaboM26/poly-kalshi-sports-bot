//! Kalshi platform client
//!
//! Handles Kalshi API interactions including:
//! - RSA-PSS authentication
//! - Market data retrieval
//! - WebSocket order book subscription
//! - Order placement

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use parking_lot::RwLock;
use reqwest::Client;
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::pss::{BlindedSigningKey, Signature};
use rsa::sha2::Sha256;
use rsa::signature::{RandomizedSigner, SignatureEncoding};
use rsa::RsaPrivateKey;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

use crate::config::KalshiConfig;
use crate::models::{KalshiEvent, KalshiMarket, Platform, PriceUpdate};

const KALSHI_WS_URL: &str = "wss://external-api-ws.kalshi.com/trade-api/ws/v2";

/// Subscription command for Kalshi WebSocket
#[derive(Debug, Clone)]
pub enum KalshiWsCommand {
    Subscribe(Vec<String>),
    Unsubscribe(Vec<String>),
}

/// Kalshi API client
#[derive(Clone)]
pub struct KalshiClient {
    pub config: KalshiConfig,
    http: Client,
    signing_key: Arc<BlindedSigningKey<Sha256>>,
    /// Order book cache: market_ticker -> { "yes": [[price, qty], ...], "no": [[price, qty], ...] }
    orderbook_cache: Arc<RwLock<HashMap<String, OrderBook>>>,
    /// Channel sender for dynamic subscriptions/unsubscriptions
    command_tx: Arc<RwLock<Option<mpsc::Sender<KalshiWsCommand>>>>,
}

/// Order book structure
#[derive(Debug, Clone, Default)]
pub struct OrderBook {
    pub yes: Vec<(i32, i32)>, // (price_cents, quantity)
    pub no: Vec<(i32, i32)>,
}

impl OrderBook {
    /// 计算 yes 侧的 ask 深度（买入 yes 时使用）
    /// Kalshi: yes_ask = 1 - no_bid，所以买 yes 的深度看 no 侧的 bid
    /// no 按价格升序排列，last 是最高买价（best no_bid）
    /// 只看最优档位的深度，避免累加多档位导致过于乐观的深度估计
    pub fn yes_ask_depth(&self, _max_contracts: i32) -> i32 {
        // 只返回最优档位（No 侧最高 bid）的深度
        self.no.last().map(|(_, qty)| *qty).unwrap_or(0)
    }

    /// 计算 no 侧的 ask 深度（买入 no 时使用）
    /// no_ask = 1 - yes_bid，所以买 no 的深度看 yes 侧的 bid
    /// 只看最优档位的深度，避免累加多档位导致过于乐观的深度估计
    pub fn no_ask_depth(&self, _max_contracts: i32) -> i32 {
        // 只返回最优档位（Yes 侧最高 bid）的深度
        self.yes.last().map(|(_, qty)| *qty).unwrap_or(0)
    }

    /// 根据 side 获取对应的 ask 深度
    pub fn ask_depth_for_side(&self, side: &str, max_contracts: i32) -> i32 {
        match side.to_lowercase().as_str() {
            "yes" => self.yes_ask_depth(max_contracts),
            "no" => self.no_ask_depth(max_contracts),
            _ => 0,
        }
    }
}

impl KalshiClient {
    /// Create a new Kalshi client
    pub fn new(config: KalshiConfig) -> Result<Self> {
        // Parse RSA private key - 支持 PKCS#8 和 PKCS#1 两种格式
        let private_key = RsaPrivateKey::from_pkcs8_pem(&config.api_secret)
            .or_else(|_| RsaPrivateKey::from_pkcs1_pem(&config.api_secret))
            .with_context(|| {
                "Failed to parse Kalshi RSA private key (tried both PKCS#8 and PKCS#1 formats)"
            })?;
        let signing_key = Arc::new(BlindedSigningKey::<Sha256>::new(private_key));

        Ok(Self {
            config,
            http: Client::new(),
            signing_key,
            orderbook_cache: Arc::new(RwLock::new(HashMap::new())),
            command_tx: Arc::new(RwLock::new(None)),
        })
    }

    /// Sign a request using RSA-PSS
    fn sign_request(&self, timestamp: i64, method: &str, path: &str) -> String {
        let message = format!("{}{}{}", timestamp, method, path);
        let mut rng = rand::thread_rng();
        let signature: Signature = self.signing_key.sign_with_rng(&mut rng, message.as_bytes());
        BASE64.encode(signature.to_bytes())
    }

    /// Get current timestamp in milliseconds
    fn get_timestamp_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
    }

    /// Make an authenticated GET request
    async fn get(&self, path: &str) -> Result<Value> {
        let timestamp = Self::get_timestamp_ms();
        // 签名需要完整 API 路径 (与 Python 版本一致)
        let sign_path = format!("/trade-api/v2{}", path);
        let signature = self.sign_request(timestamp, "GET", &sign_path);

        let url = format!("{}{}", self.config.base_url, path);

        let response = self
            .http
            .get(&url)
            .header("KALSHI-ACCESS-KEY", &self.config.api_key)
            .header("KALSHI-ACCESS-SIGNATURE", &signature)
            .header("KALSHI-ACCESS-TIMESTAMP", timestamp.to_string())
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() {
            bail!("Kalshi API error {}: {}", status, body);
        }

        serde_json::from_str(&body).with_context(|| format!("Failed to parse response: {}", body))
    }

    /// Make an authenticated POST request
    async fn post(&self, path: &str, body: &Value) -> Result<Value> {
        let timestamp = Self::get_timestamp_ms();
        // 签名需要完整 API 路径 (与 Python 版本一致)
        let sign_path = format!("/trade-api/v2{}", path);
        let signature = self.sign_request(timestamp, "POST", &sign_path);

        let url = format!("{}{}", self.config.base_url, path);

        let response = self
            .http
            .post(&url)
            .header("KALSHI-ACCESS-KEY", &self.config.api_key)
            .header("KALSHI-ACCESS-SIGNATURE", &signature)
            .header("KALSHI-ACCESS-TIMESTAMP", timestamp.to_string())
            .header("Content-Type", "application/json")
            .json(body)
            .send()
            .await?;

        let status = response.status();
        let resp_body = response.text().await?;

        if !status.is_success() {
            bail!("Kalshi API error {}: {}", status, resp_body);
        }

        serde_json::from_str(&resp_body)
            .with_context(|| format!("Failed to parse response: {}", resp_body))
    }

    /// Get account balance
    pub async fn get_balance(&self) -> Result<f64> {
        let response = self.get("/portfolio/balance").await?;
        let balance = response["balance"]
            .as_f64()
            .ok_or_else(|| anyhow::anyhow!("Invalid balance response"))?;
        Ok(balance / 100.0) // Convert cents to dollars
    }

    /// Get order book from cache
    pub fn get_orderbook(&self, ticker: &str) -> Option<OrderBook> {
        self.orderbook_cache.read().get(ticker).cloned()
    }

    /// Get NBA events and markets
    pub async fn get_nba_events_and_markets(
        &self,
    ) -> Result<(Vec<KalshiEvent>, Vec<KalshiMarket>)> {
        let mut events = Vec::new();
        let mut markets = Vec::new();

        // Kalshi paginates event results. Fetch every open NBA event before matching.
        let mut cursor = None;
        loop {
            let path = {
                let mut query = url::form_urlencoded::Serializer::new(String::new());
                query.append_pair("series_ticker", "KXNBAGAME");
                query.append_pair("status", "open");
                query.append_pair("with_nested_markets", "true");
                query.append_pair("limit", "200");
                if let Some(cursor) = cursor.as_deref() {
                    query.append_pair("cursor", cursor);
                }
                format!("/events?{}", query.finish())
            };
            let response = self.get(&path).await?;
            let event_array = response["events"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("Invalid events response"))?;

            for event_data in event_array {
                let event_ticker = event_data["event_ticker"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();

                // Extract team names from event_ticker (e.g., "KXNBAGAME-26JAN07CLELAL" -> "CLE", "LAL")
                let team_names = extract_teams_from_ticker(&event_ticker);
                if team_names.is_none() {
                    continue;
                }
                let (mut team_a, mut team_b) = team_names.unwrap();

                // Standardize event name (alphabetical order)
                if team_a > team_b {
                    std::mem::swap(&mut team_a, &mut team_b);
                }
                let event_name = format!("{}-{}", team_a, team_b);

                // Parse start time from event ticker (Python-compatible approach)
                // Format: KXNBA-26JAN08-DAL-UTA -> 2026-01-08
                let start_time = extract_game_date_from_ticker(&event_ticker);

                let mut event = KalshiEvent {
                    event_id: event_ticker.clone(),
                    name: event_name.clone(),
                    team_a: team_a.clone(),
                    team_b: team_b.clone(),
                    start_time,
                    category: "NBA".to_string(),
                    markets: Vec::new(),
                };

                // Parse markets
                if let Some(market_array) = event_data["markets"].as_array() {
                    for market_data in market_array {
                        let ticker = market_data["ticker"].as_str().unwrap_or("").to_string();

                        // Extract team from ticker (e.g., "KXNBAGAME-26JAN07CLELAL-CLE" -> "CLE")
                        let team_name = match extract_team_from_ticker(&ticker) {
                            Some(t) => t,
                            None => continue,
                        };

                        let opponent_name = if team_name.to_uppercase() == team_a.to_uppercase() {
                            team_b.clone()
                        } else {
                            team_a.clone()
                        };

                        let yes_price = market_data["yes_ask"]
                            .as_f64()
                            .or_else(|| market_data["last_price"].as_f64())
                            .unwrap_or(0.5)
                            / 100.0;
                        let no_price = 1.0 - yes_price;

                        let market = KalshiMarket {
                            market_id: ticker.clone(),
                            event_id: event_ticker.clone(),
                            event_name: event_name.clone(),
                            team_name: team_name.clone(),
                            opponent_name,
                            yes_price,
                            no_price,
                            start_time,
                            volume: market_data["volume"].as_f64(),
                            liquidity: market_data["open_interest"].as_f64(),
                        };

                        event.markets.push(market.clone());
                        markets.push(market);
                    }
                }

                events.push(event);
            }

            let next_cursor = response["cursor"].as_str().unwrap_or_default();
            if next_cursor.is_empty() {
                break;
            }
            cursor = Some(next_cursor.to_string());
        }

        info!(
            "Loaded {} Kalshi events and {} markets",
            events.len(),
            markets.len()
        );

        Ok((events, markets))
    }

    /// Place a market order - fill immediately at best available prices
    pub async fn place_order(
        &self,
        ticker: &str,
        side: &str,
        outcome: &str,
        count: i32,
        price: i32, // 当前市场价格（美分），会在此基础上+1美分以保证成交
    ) -> Result<Value> {
        let action = if side == "buy" { "buy" } else { "sell" };

        // 在当前价格基础上加1美分以保证成交，但不超过99美分
        let adjusted_price = (price + 1).min(99);

        // 根据 outcome 决定使用 yes_price 还是 no_price
        // Kalshi API 要求市价单必须提供价格参数
        let mut body = json!({
            "ticker": ticker,
            "action": action,
            "side": outcome,
            "count": count,
            "type": "market",
        });

        if outcome == "yes" {
            body["yes_price"] = json!(adjusted_price);
        } else {
            body["no_price"] = json!(adjusted_price);
        }

        self.post("/portfolio/orders", &body).await
    }

    /// Get orders with optional status filter
    pub async fn get_orders(&self, status: Option<&str>) -> Result<Value> {
        let path = if let Some(s) = status {
            format!("/portfolio/orders?status={}", s)
        } else {
            "/portfolio/orders".to_string()
        };

        self.get(&path).await
    }

    /// Get positions
    pub async fn get_positions(&self) -> Result<Value> {
        self.get("/portfolio/positions").await
    }

    /// Cancel an order
    pub async fn cancel_order(&self, order_id: &str) -> Result<Value> {
        let timestamp = Self::get_timestamp_ms();
        let path = format!("/portfolio/orders/{}", order_id);
        // 签名需要完整 API 路径 (与 Python 版本一致)
        let sign_path = format!("/trade-api/v2{}", path);
        let signature = self.sign_request(timestamp, "DELETE", &sign_path);

        let url = format!("{}{}", self.config.base_url, path);

        let response = self
            .http
            .delete(&url)
            .header("KALSHI-ACCESS-KEY", &self.config.api_key)
            .header("KALSHI-ACCESS-SIGNATURE", &signature)
            .header("KALSHI-ACCESS-TIMESTAMP", timestamp.to_string())
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() {
            bail!("Kalshi API error {}: {}", status, body);
        }

        serde_json::from_str(&body).with_context(|| format!("Failed to parse response: {}", body))
    }

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

    /// Connect to WebSocket for real-time updates
    pub async fn connect_websocket(
        &self,
        tickers: Vec<String>,
        price_tx: mpsc::Sender<PriceUpdate>,
    ) -> Result<()> {
        let timestamp = Self::get_timestamp_ms();
        let signature = self.sign_request(timestamp, "GET", "/trade-api/ws/v2");

        // 使用 HTTP headers 传递认证信息（与 Python 版本一致）
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

        info!("Connecting to the Kalshi WebSocket...");

        let (ws_stream, _) = connect_async(request)
            .await
            .with_context(|| "连接 Kalshi WebSocket 失败")?;

        let (mut write, mut read) = ws_stream.split();
        let mut pending_subscription_ids = HashMap::new();
        let mut subscription_ids = HashMap::new();

        // Create channel for dynamic subscriptions/unsubscriptions
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<KalshiWsCommand>(100);
        *self.command_tx.write() = Some(cmd_tx);

        // Subscribe to initial order books - 逐个订阅（与 Python 版本一致）
        let mut next_msg_id = 1;
        for ticker in tickers.iter() {
            pending_subscription_ids.insert(next_msg_id, ticker.clone());
            let subscribe_msg = json!({
                "id": next_msg_id,
                "cmd": "subscribe",
                "params": {
                    "channels": ["orderbook_delta"],
                    "market_ticker": ticker  // 单个 ticker，不是数组
                }
            });
            next_msg_id += 1;

            write.send(Message::Text(subscribe_msg.to_string())).await?;
        }

        info!("Subscribed to {} Kalshi markets", tickers.len());

        let orderbook_cache = self.orderbook_cache.clone();

        // Process messages with dynamic subscription/unsubscription support
        loop {
            tokio::select! {
                // Handle incoming WebSocket messages
                msg = read.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            Self::record_subscription(
                                &text,
                                &mut pending_subscription_ids,
                                &mut subscription_ids,
                            );
                            if let Some(update) = Self::parse_ws_message(&text, &orderbook_cache) {
                                if price_tx.send(update).await.is_err() {
                                    warn!("Price update channel has closed");
                                    break;
                                }
                            }
                        }
                        Some(Ok(Message::Close(_))) => {
                            info!("Kalshi WebSocket closed");
                            break;
                        }
                        Some(Err(e)) => {
                            error!("Kalshi WebSocket error: {}", e);
                            break;
                        }
                        None => {
                            info!("Kalshi WebSocket stream ended");
                            break;
                        }
                        _ => {}
                    }
                }
                // Handle dynamic subscription/unsubscription requests
                Some(command) = cmd_rx.recv() => {
                    match command {
                        KalshiWsCommand::Subscribe(new_tickers) => {
                            info!("🔌 [Kalshi] Processing hot subscription for {} new markets", new_tickers.len());
                            for ticker in new_tickers.iter() {
                                pending_subscription_ids.insert(next_msg_id, ticker.clone());
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
                                    error!("❌ [Kalshi] Hot subscription send failed: {}", e);
                                }
                            }
                            info!("✅ [Kalshi] Hot subscription completed for {} markets", new_tickers.len());
                        }
                        KalshiWsCommand::Unsubscribe(tickers_to_unsub) => {
                            let ticker_count = tickers_to_unsub.len();
                            info!("🔌 [Kalshi] Processing unsubscribe for {} markets", ticker_count);
                            let subscription_ids_to_remove: Vec<u64> = tickers_to_unsub
                                .iter()
                                .filter_map(|ticker| {
                                    let sid = subscription_ids.remove(ticker);
                                    if sid.is_none() {
                                        warn!("Kalshi subscription for {} is not active", ticker);
                                    }
                                    sid
                                })
                                .collect();

                            if !subscription_ids_to_remove.is_empty() {
                                let unsubscribe_msg = json!({
                                    "id": next_msg_id,
                                    "cmd": "unsubscribe",
                                    "params": {
                                        "sids": subscription_ids_to_remove
                                    }
                                });
                                next_msg_id += 1;

                                if let Err(e) = write.send(Message::Text(unsubscribe_msg.to_string())).await {
                                    error!("❌ [Kalshi] Unsubscribe send failed: {}", e);
                                }
                            }
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
    fn record_subscription(
        text: &str,
        pending_subscription_ids: &mut HashMap<u64, String>,
        subscription_ids: &mut HashMap<String, u64>,
    ) {
        let Ok(data) = serde_json::from_str::<Value>(text) else {
            return;
        };
        if data["type"].as_str() != Some("subscribed") {
            return;
        }

        let (Some(request_id), Some(subscription_id)) =
            (data["id"].as_u64(), data["msg"]["sid"].as_u64())
        else {
            warn!("Kalshi returned an invalid subscription confirmation");
            return;
        };

        if let Some(ticker) = pending_subscription_ids.remove(&request_id) {
            subscription_ids.insert(ticker, subscription_id);
        } else {
            warn!(
                "Kalshi returned an unexpected subscription confirmation: {}",
                request_id
            );
        }
    }

    /// Parse WebSocket message
    fn parse_ws_message(
        text: &str,
        orderbook_cache: &Arc<RwLock<HashMap<String, OrderBook>>>,
    ) -> Option<PriceUpdate> {
        let data: Value = serde_json::from_str(text).ok()?;
        let msg_type = data.get("type")?.as_str()?;

        match msg_type {
            "orderbook_snapshot" => {
                let msg = data.get("msg")?;
                let ticker = msg.get("market_ticker")?.as_str()?;

                let mut book = OrderBook::default();
                book.yes = parse_price_levels(msg, "yes_dollars_fp", "yes");
                book.no = parse_price_levels(msg, "no_dollars_fp", "no");

                // Sort by price
                book.yes.sort_by_key(|(p, _)| *p);
                book.no.sort_by_key(|(p, _)| *p);

                orderbook_cache
                    .write()
                    .insert(ticker.to_string(), book.clone());

                // Calculate prices
                let yes_bid = book.yes.last().map(|(p, _)| *p as f64 / 100.0);
                let no_bid = book.no.last().map(|(p, _)| *p as f64 / 100.0);

                if let (Some(yb), Some(nb)) = (yes_bid, no_bid) {
                    Some(PriceUpdate {
                        platform: Platform::Kalshi,
                        market_id: ticker.to_string(),
                        yes_bid: Some(yb),
                        yes_ask: Some(1.0 - nb),
                        no_bid: Some(nb),
                        no_ask: Some(1.0 - yb),
                        timestamp: Utc::now(),
                    })
                } else {
                    None
                }
            }
            "orderbook_delta" => {
                let msg = data.get("msg")?;
                let ticker = msg.get("market_ticker")?.as_str()?;
                let price = msg
                    .get("price_dollars")
                    .and_then(dollars_to_cents)
                    .or_else(|| {
                        msg.get("price")
                            .and_then(Value::as_i64)
                            .map(|price| price as i32)
                    })?;
                let delta = msg
                    .get("delta_fp")
                    .and_then(fixed_point_to_i32)
                    .or_else(|| {
                        msg.get("delta")
                            .and_then(Value::as_i64)
                            .map(|delta| delta as i32)
                    })?;
                let side = msg.get("side")?.as_str()?;

                // Apply delta
                let mut cache = orderbook_cache.write();
                let book = cache.get_mut(ticker)?;

                let book_side = if side == "yes" {
                    &mut book.yes
                } else {
                    &mut book.no
                };

                // Find and update or insert
                if let Some(pos) = book_side.iter().position(|(p, _)| *p == price) {
                    let new_qty = book_side[pos].1 + delta;
                    if new_qty <= 0 {
                        book_side.remove(pos);
                    } else {
                        book_side[pos].1 = new_qty;
                    }
                } else if delta > 0 {
                    book_side.push((price, delta));
                    book_side.sort_by_key(|(p, _)| *p);
                }

                // Recalculate prices
                let yes_bid = book.yes.last().map(|(p, _)| *p as f64 / 100.0);
                let no_bid = book.no.last().map(|(p, _)| *p as f64 / 100.0);

                drop(cache);

                if let (Some(yb), Some(nb)) = (yes_bid, no_bid) {
                    Some(PriceUpdate {
                        platform: Platform::Kalshi,
                        market_id: ticker.to_string(),
                        yes_bid: Some(yb),
                        yes_ask: Some(1.0 - nb),
                        no_bid: Some(nb),
                        no_ask: Some(1.0 - yb),
                        timestamp: Utc::now(),
                    })
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

/// Parse current fixed-point levels and the legacy integer-cent representation.
fn parse_price_levels(message: &Value, fixed_point_key: &str, legacy_key: &str) -> Vec<(i32, i32)> {
    let Some(levels) = message
        .get(fixed_point_key)
        .or_else(|| message.get(legacy_key))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };

    let fixed_point = message.get(fixed_point_key).is_some();
    levels
        .iter()
        .filter_map(|entry| {
            let price = entry.get(0)?;
            let quantity = entry.get(1).and_then(quantity_to_i32)?;
            let price = if fixed_point {
                dollars_to_cents(price)?
            } else {
                price.as_i64()? as i32
            };
            Some((price, quantity))
        })
        .collect()
}

/// Convert Kalshi's fixed-point dollar price to an exact whole-cent price.
fn dollars_to_cents(value: &Value) -> Option<i32> {
    let price = value
        .as_str()
        .map(ToOwned::to_owned)
        .or_else(|| value.as_number().map(ToString::to_string))?;
    let (dollars, fractional) = price.split_once('.').unwrap_or((&price, ""));
    let dollars = dollars.parse::<i32>().ok()?;
    let mut cents = fractional.chars().take(2).collect::<String>();
    while cents.len() < 2 {
        cents.push('0');
    }
    if fractional.chars().skip(2).any(|digit| digit != '0') {
        return None;
    }

    let cents = dollars
        .checked_mul(100)?
        .checked_add(cents.parse::<i32>().ok()?)?;
    (0..=100).contains(&cents).then_some(cents)
}

fn quantity_to_i32(value: &Value) -> Option<i32> {
    fixed_point_to_i32(value).filter(|quantity| *quantity >= 0)
}

fn fixed_point_to_i32(value: &Value) -> Option<i32> {
    let quantity = value
        .as_f64()
        .or_else(|| {
            value
                .as_str()
                .and_then(|quantity| quantity.parse::<f64>().ok())
        })?
        .round();
    (quantity.is_finite() && quantity >= i32::MIN as f64 && quantity <= i32::MAX as f64)
        .then_some(quantity as i32)
}

/// Parse team names from event title
/// Extract team names from event_ticker
/// Example: "KXNBAGAME-26JAN07CLELAL" -> ("CLE", "LAL")
fn extract_teams_from_ticker(event_ticker: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = event_ticker.split('-').collect();
    if parts.len() < 2 {
        return None;
    }

    let last_part = parts.last()?;
    if last_part.len() <= 7 {
        return None;
    }

    // Skip the date part (first 7 chars like "26JAN07")
    let teams_str = &last_part[7..];

    // Most common: 6 characters (3 + 3)
    if teams_str.len() == 6 {
        return Some((teams_str[..3].to_uppercase(), teams_str[3..].to_uppercase()));
    }

    // Handle 7+ characters by splitting in the middle
    if teams_str.len() >= 4 {
        let mid = teams_str.len() / 2;
        return Some((
            teams_str[..mid].to_uppercase(),
            teams_str[mid..].to_uppercase(),
        ));
    }

    None
}

/// Extract team from market ticker
/// Example: "KXNBAGAME-26JAN07CLELAL-CLE" -> "CLE"
fn extract_team_from_ticker(ticker: &str) -> Option<String> {
    let parts: Vec<&str> = ticker.split('-').collect();
    if parts.len() < 3 {
        return None;
    }

    // Last part is the team abbreviation
    Some(parts.last()?.to_uppercase())
}

/// Extract game date from event ticker (e.g., "KXNBA-26JAN08-DAL-UTA" -> 2026-01-08)
///
/// Format: The second part contains the date as "YYMMMDD" where:
/// - YY: two-digit year (e.g., "26" for 2026)
/// - MMM: three-letter month abbreviation (e.g., "JAN")
/// - DD: two-digit day (e.g., "08")
fn extract_game_date_from_ticker(event_ticker: &str) -> Option<DateTime<Utc>> {
    let parts: Vec<&str> = event_ticker.split('-').collect();
    if parts.len() < 2 {
        return None;
    }

    let date_part = parts[1];
    if date_part.len() < 7 {
        return None;
    }

    // Parse year (first 2 characters)
    let year_str = &date_part[..2];
    let year: i32 = match year_str.parse::<i32>() {
        Ok(y) => 2000 + y,
        Err(_) => return None,
    };

    // Parse month (characters 2-5, e.g., "JAN")
    let month_str = &date_part[2..5];
    let month: u32 = match month_str.to_uppercase().as_str() {
        "JAN" => 1,
        "FEB" => 2,
        "MAR" => 3,
        "APR" => 4,
        "MAY" => 5,
        "JUN" => 6,
        "JUL" => 7,
        "AUG" => 8,
        "SEP" => 9,
        "OCT" => 10,
        "NOV" => 11,
        "DEC" => 12,
        _ => return None,
    };

    // Parse day (characters 5-7)
    let day_str = &date_part[5..7];
    let day: u32 = match day_str.parse() {
        Ok(d) => d,
        Err(_) => return None,
    };

    use chrono::NaiveDate;
    let naive_date = NaiveDate::from_ymd_opt(year, month, day)?;
    let naive_datetime = naive_date.and_hms_opt(12, 0, 0)?;

    Some(DateTime::from_naive_utc_and_offset(naive_datetime, Utc))
}
