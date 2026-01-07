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

/// Kalshi API client
#[derive(Clone)]
pub struct KalshiClient {
    pub config: KalshiConfig,
    http: Client,
    signing_key: Arc<BlindedSigningKey<Sha256>>,
    /// Order book cache: market_ticker -> { "yes": [[price, qty], ...], "no": [[price, qty], ...] }
    orderbook_cache: Arc<RwLock<HashMap<String, OrderBook>>>,
}

/// Order book structure
#[derive(Debug, Clone, Default)]
pub struct OrderBook {
    pub yes: Vec<(i32, i32)>, // (price_cents, quantity)
    pub no: Vec<(i32, i32)>,
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
        let signature = self.sign_request(timestamp, "GET", path);

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
        let signature = self.sign_request(timestamp, "POST", path);

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
            let title = event_data["title"].as_str().unwrap_or("");

            // Parse team names from title (e.g., "MEM vs LAL")
            let team_names = parse_team_names(title);
            if team_names.is_none() {
                continue;
            }
            let (team_a, team_b) = team_names.unwrap();

            // Standardize event name (alphabetical order)
            let event_name = if team_a < team_b {
                format!("{}-{}", team_a, team_b)
            } else {
                format!("{}-{}", team_b, team_a)
            };

            // Parse start time
            let start_time = event_data["expected_expiration_time"]
                .as_str()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc));

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
                    let subtitle = market_data["subtitle"].as_str().unwrap_or("");

                    // Determine which team this market is for
                    let team_name = if subtitle.contains(&team_a) {
                        team_a.clone()
                    } else if subtitle.contains(&team_b) {
                        team_b.clone()
                    } else {
                        // Try to extract from ticker
                        ticker
                            .split('-')
                            .last()
                            .map(|s| s.to_string())
                            .unwrap_or_default()
                    };

                    let opponent_name = if team_name == team_a {
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
            "Loaded {} Kalshi events with {} markets",
            events.len(),
            markets.len()
        );

        Ok((events, markets))
    }

    /// Place an order
    pub async fn place_order(
        &self,
        ticker: &str,
        side: &str,
        outcome: &str,
        count: i32,
        price: i32, // in cents
    ) -> Result<Value> {
        let action = if side == "buy" { "buy" } else { "sell" };
        let body = json!({
            "ticker": ticker,
            "action": action,
            "side": outcome,
            "count": count,
            "type": "limit",
            "yes_price": if outcome == "yes" { price } else { 100 - price },
        });

        self.post("/portfolio/orders", &body).await
    }

    /// Connect to WebSocket for real-time updates
    pub async fn connect_websocket(
        &self,
        tickers: Vec<String>,
        price_tx: mpsc::Sender<PriceUpdate>,
    ) -> Result<()> {
        let timestamp = Self::get_timestamp_ms();
        let signature = self.sign_request(timestamp, "GET", "/trade-api/ws/v2");

        let ws_url = format!(
            "{}?api_key={}&timestamp={}&signature={}",
            KALSHI_WS_URL, self.config.api_key, timestamp, signature
        );

        info!("Connecting to Kalshi WebSocket...");

        let (ws_stream, _) = connect_async(&ws_url)
            .await
            .with_context(|| "Failed to connect to Kalshi WebSocket")?;

        let (mut write, mut read) = ws_stream.split();

        // Subscribe to order books
        let subscribe_msg = json!({
            "id": 1,
            "cmd": "subscribe",
            "params": {
                "channels": ["orderbook_delta"],
                "market_tickers": tickers
            }
        });

        write
            .send(Message::Text(subscribe_msg.to_string()))
            .await?;

        info!("Subscribed to {} Kalshi markets", tickers.len());

        let orderbook_cache = self.orderbook_cache.clone();

        // Process messages
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Some(update) =
                        Self::parse_ws_message(&text, &orderbook_cache)
                    {
                        if price_tx.send(update).await.is_err() {
                            warn!("Price update channel closed");
                            break;
                        }
                    }
                }
                Ok(Message::Close(_)) => {
                    info!("Kalshi WebSocket closed");
                    break;
                }
                Err(e) => {
                    error!("Kalshi WebSocket error: {}", e);
                    break;
                }
                _ => {}
            }
        }

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
fn parse_team_names(title: &str) -> Option<(String, String)> {
    // Common patterns: "MEM vs LAL", "MEM @ LAL", "MEM at LAL"
    let separators = ["vs", "@", "at", "vs.", " - "];

    for sep in separators {
        if let Some(pos) = title.to_lowercase().find(sep) {
            let left = title[..pos].trim();
            let right = title[pos + sep.len()..].trim();

            // Extract abbreviations (last word or 3-letter code)
            let team_a = extract_team_abbr(left)?;
            let team_b = extract_team_abbr(right)?;

            return Some((team_a.to_uppercase(), team_b.to_uppercase()));
        }
    }

    None
}

/// Extract team abbreviation from text
fn extract_team_abbr(text: &str) -> Option<String> {
    // Try to find 3-letter abbreviation
    let words: Vec<&str> = text.split_whitespace().collect();

    for word in words.iter().rev() {
        let clean = word.trim_matches(|c: char| !c.is_alphanumeric());
        if clean.len() == 3 && clean.chars().all(|c| c.is_alphabetic()) {
            return Some(clean.to_uppercase());
        }
    }

    // Fall back to last word
    words.last().map(|w| w.to_uppercase())
}
