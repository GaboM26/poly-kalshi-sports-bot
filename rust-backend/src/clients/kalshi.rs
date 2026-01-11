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
use chrono::{DateTime, TimeZone, Utc};
use futures_util::{SinkExt, StreamExt};
use parking_lot::RwLock;
use reqwest::Client;
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::pss::{BlindedSigningKey, Signature};
use rsa::sha2::Sha256;
use rsa::signature::{RandomizedSigner, SignatureEncoding};
use rsa::RsaPrivateKey;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

use crate::config::KalshiConfig;
use crate::models::{KalshiEvent, KalshiMarket, Platform, PriceUpdate};

const KALSHI_WS_URL: &str = "wss://api.elections.kalshi.com/trade-api/ws/v2";

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
    pub fn yes_ask_depth(&self, max_contracts: i32) -> i32 {
        let mut depth = 0;
        for (_, qty) in self.no.iter().rev() {
            depth += qty;
            if depth >= max_contracts {
                return max_contracts;
            }
        }
        depth
    }

    /// 计算 no 侧的 ask 深度（买入 no 时使用）
    /// no_ask = 1 - yes_bid，所以买 no 的深度看 yes 侧的 bid
    pub fn no_ask_depth(&self, max_contracts: i32) -> i32 {
        let mut depth = 0;
        for (_, qty) in self.yes.iter().rev() {
            depth += qty;
            if depth >= max_contracts {
                return max_contracts;
            }
        }
        depth
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
            .with_context(|| "Failed to parse Kalshi RSA private key (tried both PKCS#8 and PKCS#1 formats)")?;
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
    pub async fn get_nba_events_and_markets(&self) -> Result<(Vec<KalshiEvent>, Vec<KalshiMarket>)>
    {
        let mut events = Vec::new();
        let mut markets = Vec::new();

        // Get NBA events
        let response = self
            .get("/events?series_ticker=KXNBAGAME&status=open&with_nested_markets=true")
            .await?;

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

        info!(
            "已加载 {} 个 Kalshi 事件和 {} 个市场",
            events.len(),
            markets.len()
        );

        Ok((events, markets))
    }

    /// Place an order (market order using FOK - Fill or Kill)
    pub async fn place_order(
        &self,
        ticker: &str,
        side: &str,
        outcome: &str,
        count: i32,
        price: i32, // in cents - used as max price for FOK
    ) -> Result<Value> {
        let action = if side == "buy" { "buy" } else { "sell" };
        let yes_price = if outcome == "yes" { price } else { 100 - price };
        
        // 使用 FOK (Fill or Kill) 订单类型，保证立即成交或取消
        // yes_price 作为最高可接受价格（市价吃单）
        let body = json!({
            "ticker": ticker,
            "action": action,
            "side": outcome,
            "count": count,
            "type": "market",
            "yes_price": yes_price,
        });

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
                    info!("🔌 [Kalshi] 发送热订阅请求: {} 个市场", tickers.len());
                    Ok(true)
                }
                Err(e) => {
                    warn!("⚠️ [Kalshi] 热订阅请求发送失败: {}", e);
                    Ok(false)
                }
            }
        } else {
            warn!("⚠️ [Kalshi] WebSocket 未连接，无法热订阅");
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
                    info!("🔌 [Kalshi] 发送取消订阅请求: {} 个市场", tickers.len());
                    Ok(true)
                }
                Err(e) => {
                    warn!("⚠️ [Kalshi] 取消订阅请求发送失败: {}", e);
                    Ok(false)
                }
            }
        } else {
            warn!("⚠️ [Kalshi] WebSocket 未连接，无法取消订阅");
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
        request.headers_mut().insert(
            "KALSHI-ACCESS-KEY",
            self.config.api_key.parse().unwrap(),
        );
        request.headers_mut().insert(
            "KALSHI-ACCESS-SIGNATURE",
            signature.parse().unwrap(),
        );
        request.headers_mut().insert(
            "KALSHI-ACCESS-TIMESTAMP",
            timestamp.to_string().parse().unwrap(),
        );

        info!("正在连接 Kalshi WebSocket...");

        let (ws_stream, _) = connect_async(request)
            .await
            .with_context(|| "连接 Kalshi WebSocket 失败")?;

        let (mut write, mut read) = ws_stream.split();

        // Create channel for dynamic subscriptions/unsubscriptions
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<KalshiWsCommand>(100);
        *self.command_tx.write() = Some(cmd_tx);

        // Subscribe to initial order books - 逐个订阅（与 Python 版本一致）
        let mut next_msg_id = 1;
        for ticker in tickers.iter() {
            let subscribe_msg = json!({
                "id": next_msg_id,
                "cmd": "subscribe",
                "params": {
                    "channels": ["orderbook_delta"],
                    "market_ticker": ticker  // 单个 ticker，不是数组
                }
            });
            next_msg_id += 1;

            write
                .send(Message::Text(subscribe_msg.to_string()))
                .await?;
        }

        info!("已订阅 {} 个 Kalshi 市场", tickers.len());

        let orderbook_cache = self.orderbook_cache.clone();

        // Process messages with dynamic subscription/unsubscription support
        loop {
            tokio::select! {
                // Handle incoming WebSocket messages
                msg = read.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            if let Some(update) = Self::parse_ws_message(&text, &orderbook_cache) {
                                if price_tx.send(update).await.is_err() {
                                    warn!("价格更新通道已关闭");
                                    break;
                                }
                            }
                        }
                        Some(Ok(Message::Close(_))) => {
                            info!("Kalshi WebSocket 已关闭");
                            break;
                        }
                        Some(Err(e)) => {
                            error!("Kalshi WebSocket 错误: {}", e);
                            break;
                        }
                        None => {
                            info!("Kalshi WebSocket 流结束");
                            break;
                        }
                        _ => {}
                    }
                }
                // Handle dynamic subscription/unsubscription requests
                Some(command) = cmd_rx.recv() => {
                    match command {
                        KalshiWsCommand::Subscribe(new_tickers) => {
                            info!("🔌 [Kalshi] 处理热订阅: {} 个新市场", new_tickers.len());
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
                                    error!("❌ [Kalshi] 热订阅发送失败: {}", e);
                                }
                            }
                            info!("✅ [Kalshi] 热订阅完成: {} 个市场", new_tickers.len());
                        }
                        KalshiWsCommand::Unsubscribe(tickers_to_unsub) => {
                            info!("🔌 [Kalshi] 处理取消订阅: {} 个市场", tickers_to_unsub.len());
                            for ticker in tickers_to_unsub.iter() {
                                let unsubscribe_msg = json!({
                                    "id": next_msg_id,
                                    "cmd": "unsubscribe",
                                    "params": {
                                        "channels": ["orderbook_delta"],
                                        "market_ticker": ticker
                                    }
                                });
                                next_msg_id += 1;

                                if let Err(e) = write.send(Message::Text(unsubscribe_msg.to_string())).await {
                                    error!("❌ [Kalshi] 取消订阅发送失败: {}", e);
                                }
                                
                                // Also remove from orderbook cache
                                orderbook_cache.write().remove(ticker);
                            }
                            info!("✅ [Kalshi] 取消订阅完成: {} 个市场", tickers_to_unsub.len());
                        }
                    }
                }
            }
        }

        // Clear command channel on disconnect
        *self.command_tx.write() = None;

        Ok(())
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

                let yes_data = msg.get("yes")?.as_array()?;
                let no_data = msg.get("no")?.as_array()?;

                let mut book = OrderBook::default();

                for entry in yes_data {
                    if let (Some(price), Some(qty)) = (entry.get(0), entry.get(1)) {
                        book.yes.push((
                            price.as_i64().unwrap_or(0) as i32,
                            qty.as_i64().unwrap_or(0) as i32,
                        ));
                    }
                }
                for entry in no_data {
                    if let (Some(price), Some(qty)) = (entry.get(0), entry.get(1)) {
                        book.no.push((
                            price.as_i64().unwrap_or(0) as i32,
                            qty.as_i64().unwrap_or(0) as i32,
                        ));
                    }
                }

                // Sort by price
                book.yes.sort_by_key(|(p, _)| *p);
                book.no.sort_by_key(|(p, _)| *p);

                orderbook_cache.write().insert(ticker.to_string(), book.clone());

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
                let price = msg.get("price")?.as_i64()? as i32;
                let delta = msg.get("delta")?.as_i64()? as i32;
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
        return Some((
            teams_str[..3].to_uppercase(),
            teams_str[3..].to_uppercase(),
        ));
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
        "JAN" => 1, "FEB" => 2, "MAR" => 3, "APR" => 4,
        "MAY" => 5, "JUN" => 6, "JUL" => 7, "AUG" => 8,
        "SEP" => 9, "OCT" => 10, "NOV" => 11, "DEC" => 12,
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
