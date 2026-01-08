//! Polymarket platform client
//!
//! Handles Polymarket API interactions including:
//! - Market data retrieval from Gamma API
//! - WebSocket price subscription
//! - Order placement via CLOB client

use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

use crate::clob::{ApiCreds, ClobClient, MarketOrderArgs, Side, SignatureType};
use crate::config::PolymarketConfig;
use crate::models::{Platform, PolymarketEvent, PolymarketMarket, PriceUpdate};

const POLY_WS_URL: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";

/// Position data for aggregation
struct PositionData {
    asset_id: String,
    market: String,
    outcome: String,
    size: f64,
    total_cost: f64,
    trade_count: u32,
}

/// Polymarket API client
#[derive(Clone)]
pub struct PolymarketClient {
    pub config: PolymarketConfig,
    http: Client,
    /// CLOB client for order operations
    clob: Option<Arc<ClobClient>>,
}

impl PolymarketClient {
    /// Create a new Polymarket client
    pub fn new(config: PolymarketConfig) -> Self {
        Self {
            config,
            http: Client::new(),
            clob: None,
        }
    }

    /// Initialize CLOB client with API credentials
    pub async fn init_clob(&mut self) -> Result<()> {
        if self.config.private_key.is_empty() {
            return Ok(()); // No private key, skip CLOB initialization
        }

        let funder = if self.config.wallet_address.is_empty() {
            None
        } else {
            Some(self.config.wallet_address.as_str())
        };

        let sig_type = match self.config.signature_type {
            1 => SignatureType::PolyProxy,
            2 => SignatureType::PolyGnosisSafe,
            _ => SignatureType::Eoa,
        };

        // Create L1 client first
        let mut clob = ClobClient::with_l1_auth(
            &self.config.clob_url,
            137, // Polygon mainnet
            &self.config.private_key,
            Some(sig_type),
            funder,
        )?;

        // Try to derive or create API credentials
        if self.config.api_key.is_empty() {
            info!("正在派生 Polymarket API 凭证...");
            match clob.create_or_derive_api_creds(Some(0)).await {
                Ok(creds) => {
                    info!("成功派生 API 凭证");
                    clob.set_api_creds(creds);
                }
                Err(e) => {
                    warn!("派生 API 凭证失败: {}. 订单下单功能已禁用.", e);
                }
            }
        } else {
            // Use configured credentials
            clob.set_api_creds(ApiCreds {
                api_key: self.config.api_key.clone(),
                api_secret: self.config.api_secret.clone(),
                api_passphrase: self.config.api_passphrase.clone(),
            });
        }

        self.clob = Some(Arc::new(clob));
        Ok(())
    }

    /// Get account balance
    pub async fn get_balance(&self) -> Result<f64> {
        let clob = self
            .clob
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("CLOB client not initialized"))?;

        let balance = clob.as_ref().get_balance_allowance().await?;
        let balance_val: f64 = balance.balance.parse().unwrap_or(0.0);
        Ok(balance_val / 1_000_000.0) // USDC has 6 decimals
    }

    /// Get NBA events and markets from Gamma API
    pub async fn get_nba_events_and_markets(
        &self,
    ) -> Result<(Vec<PolymarketEvent>, Vec<PolymarketMarket>)> {
        let mut events = Vec::new();
        let mut markets = Vec::new();

        // Step 1: Get sports leagues
        let sports_url = format!("{}/sports", self.config.base_url);

        let sports_response = self.http.get(&sports_url).send().await?;
        if !sports_response.status().is_success() {
            anyhow::bail!("Failed to get sports leagues: {}", sports_response.status());
        }
        let sports: Vec<Value> = sports_response.json().await?;

        // Step 2: Find NBA league
        let nba_league = sports
            .iter()
            .find(|s| {
                let sport = s["sport"].as_str().unwrap_or("");
                sport.to_uppercase().contains("NBA") && !sport.to_uppercase().contains("WNBA")
            })
            .ok_or_else(|| anyhow::anyhow!("NBA league not found"))?;

        let series_id = nba_league["series"]
            .as_str()
            .or_else(|| nba_league["series"].as_i64().map(|_| ""))
            .ok_or_else(|| anyhow::anyhow!("NBA series_id not found"))?;

        // Step 3: Get NBA events
        let events_url = format!(
            "{}/events?series_id={}&tag_id=100639&active=true&closed=false&limit=100",
            self.config.base_url, series_id
        );

        let events_response = self.http.get(&events_url).send().await?;
        if !events_response.status().is_success() {
            anyhow::bail!("Failed to get NBA events: {}", events_response.status());
        }
        let api_events: Vec<Value> = events_response.json().await?;

        info!("📥 已获取 {} 个 Polymarket NBA 事件", api_events.len());

        // Step 4: Process each event
        for api_event in &api_events {
            let event_title = api_event["title"].as_str().unwrap_or("");
            let event_slug = api_event["slug"].as_str().unwrap_or("");
            let event_markets = api_event["markets"].as_array();

            // Extract date from slug
            let event_date = extract_date_from_slug(event_slug);

            if let Some(market_array) = event_markets {
                for market_data in market_array {
                    // Parse market data
                    let market_id = market_data["id"].as_str().unwrap_or("");
                    let condition_id = market_data["conditionId"]
                        .as_str()
                        .or_else(|| market_data["condition_id"].as_str())
                        .unwrap_or(market_id)
                        .to_string();

                    if condition_id.is_empty() {
                        continue;
                    }

                    let question = market_data["question"]
                        .as_str()
                        .unwrap_or(event_title);

                    // Get outcomes and prices
                    let outcomes_str = market_data["outcomes"].as_str();
                    let prices_str = market_data["outcomePrices"].as_str();

                    if outcomes_str.is_none() || prices_str.is_none() {
                        continue;
                    }

                    let outcomes: Vec<String> = match serde_json::from_str(outcomes_str.unwrap()) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    let prices: Vec<String> = match serde_json::from_str(prices_str.unwrap()) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };

                    // Must be binary market
                    if outcomes.len() != 2 || prices.len() != 2 {
                        continue;
                    }

                    let price1: f64 = match prices[0].parse() {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    let price2: f64 = match prices[1].parse() {
                        Ok(v) => v,
                        Err(_) => continue,
                    };

                    // Validate prices
                    if price1 < 0.0 || price1 > 1.0 || price2 < 0.0 || price2 > 1.0 {
                        continue;
                    }

                    // Filter invalid prices
                    if (price1 == 0.0 && price2 == 1.0) || (price1 == 1.0 && price2 == 0.0) {
                        continue;
                    }

                    // Filter extreme prices
                    if price1 < 0.01 || price2 < 0.01 || price1 > 0.99 || price2 > 0.99 {
                        continue;
                    }

                    // Filter Yes/No markets
                    if outcomes.iter().any(|o| o.to_lowercase() == "yes")
                        && outcomes.iter().any(|o| o.to_lowercase() == "no")
                    {
                        continue;
                    }

                    // Only keep full game winner markets
                    if question != event_title {
                        continue;
                    }

                    // Filter Over/Under markets
                    if outcomes[0].to_lowercase() == "over" || outcomes[0].to_lowercase() == "under" {
                        continue;
                    }

                    // Normalize team names
                    let team1_abbr = normalize_team_name(&outcomes[0]);
                    let team2_abbr = normalize_team_name(&outcomes[1]);

                    // Sort teams alphabetically (consistent with Kalshi)
                    let (team_a, team_b, price_a, price_b, token_index_a, token_index_b) =
                        if team1_abbr > team2_abbr {
                            (team2_abbr, team1_abbr, price2, price1, 1, 0)
                        } else {
                            (team1_abbr, team2_abbr, price1, price2, 0, 1)
                        };

                    // Build standardized event name
                    let event_name = format!("{}-{}", team_a, team_b);

                    // Get volume
                    let volume = market_data["volume"]
                        .as_str()
                        .and_then(|s| s.parse::<f64>().ok())
                        .or_else(|| market_data["volume"].as_f64());

                    // Get token IDs (for WebSocket subscription)
                    let tokens_str = market_data["clobTokenIds"].as_str();
                    let token_ids: Vec<String> = tokens_str
                        .and_then(|s| serde_json::from_str(s).ok())
                        .unwrap_or_default();

                    let token_id_a = token_ids.get(token_index_a).cloned();
                    let token_id_b = token_ids.get(token_index_b).cloned();

                    // Create market
                    let market = PolymarketMarket {
                        market_id: condition_id.clone(),
                        event_name: event_name.clone(),
                        team_a: team_a.clone(),
                        team_b: team_b.clone(),
                        price_a,
                        price_b,
                        start_time: event_date,
                        volume,
                        token_id_a,
                        token_id_b,
                    };

                    // Create event
                    let event = PolymarketEvent {
                        event_id: condition_id.clone(),
                        name: event_name.clone(),
                        team_a: team_a.clone(),
                        team_b: team_b.clone(),
                        start_time: event_date,
                        category: "NBA".to_string(),
                        market: Some(market.clone()),
                    };

                    events.push(event);
                    markets.push(market);
                }
            }
        }

        info!(
            "✅ Polymarket: {} 个事件, {} 个市场",
            events.len(),
            markets.len()
        );

        Ok((events, markets))
    }

    /// Place a market order
    pub async fn place_market_order(
        &self,
        token_id: &str,
        side: &str,
        amount: f64,
    ) -> Result<Value> {
        let clob = self
            .clob
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("CLOB client not initialized"))?;

        let order_side = if side.to_lowercase() == "buy" {
            Side::Buy
        } else {
            Side::Sell
        };

        let order_args = MarketOrderArgs {
            token_id: token_id.to_string(),
            amount,
            side: order_side,
            fee_rate_bps: None,
            nonce: None,
            slippage: Some(0.005), // 0.5% slippage
        };

        let signed_order = clob.as_ref().create_market_order(&order_args).await?;
        let response = clob
            .as_ref()
            .post_order(&signed_order, crate::clob::OrderType::Fok)
            .await?;

        Ok(serde_json::to_value(response)?)
    }

    /// Get open orders
    pub async fn get_open_orders(&self) -> Result<Value> {
        let clob = self
            .clob
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("CLOB client not initialized"))?;

        let orders = clob.as_ref().get_orders().await?;
        Ok(serde_json::to_value(orders)?)
    }

    /// Get positions
    pub async fn get_positions(&self) -> Result<Value> {
        // If CLOB client is not initialized, return empty positions (graceful degradation)
        let clob = match self.clob.as_ref() {
            Some(c) => c,
            None => {
                info!("⚠️ [Polymarket] CLOB 客户端未初始化，返回空持仓");
                return Ok(json!([]));
            }
        };

        // Get all trades to calculate positions
        let trades = match clob.as_ref().get_trades().await {
            Ok(t) => t,
            Err(e) => {
                info!("⚠️ [Polymarket] 无法获取交易历史: {}", e);
                return Ok(json!([]));
            }
        };

        if trades.is_empty() {
            info!("✅ [Polymarket] 无交易历史，持仓为空");
            return Ok(json!([]));
        }

        // Aggregate positions from trades
        use std::collections::HashMap;
        let mut positions: HashMap<String, PositionData> = HashMap::new();

        for trade in &trades {
            let asset_id = &trade.asset_id;
            let side_str = &trade.side;
            let size: f64 = trade.size.parse().unwrap_or(0.0);
            let price: f64 = trade.price.parse().unwrap_or(0.0);

            let pos = positions.entry(asset_id.clone()).or_insert(PositionData {
                asset_id: asset_id.clone(),
                market: trade.market.clone(),
                outcome: trade.outcome.as_ref().map(|s| s.clone()).unwrap_or_default(),
                size: 0.0,
                total_cost: 0.0,
                trade_count: 0,
            });

            // BUY increases position, SELL decreases
            if side_str.to_uppercase() == "BUY" {
                pos.size += size;
                pos.total_cost += size * price;
            } else {
                pos.size -= size;
                pos.total_cost -= size * price;
            }
            pos.trade_count += 1;
        }

        // Filter and format positions
        const MIN_POSITION_SIZE: f64 = 0.5;
        let mut result = Vec::new();

        for pos in positions.values() {
            if pos.size.abs() >= MIN_POSITION_SIZE {
                let avg_price = if pos.size != 0.0 {
                    (pos.total_cost / pos.size).max(0.0).min(1.0)
                } else {
                    0.0
                };

                result.push(json!({
                    "asset": pos.asset_id,
                    "conditionId": pos.market,
                    "outcome": pos.outcome,
                    "size": pos.size.to_string(),
                    "avgPrice": avg_price.to_string(),
                    "tradeCount": pos.trade_count,
                }));
            }
        }

        info!("✅ [Polymarket] 从 {} 笔交易聚合出 {} 个持仓", trades.len(), result.len());
        Ok(json!(result))
    }

    /// Cancel an order
    pub async fn cancel_order(&self, order_id: &str) -> Result<Value> {
        let clob = self
            .clob
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("CLOB client not initialized"))?;

        let response = clob.as_ref().cancel(order_id).await?;
        Ok(serde_json::to_value(response).unwrap_or(serde_json::json!({"success": true})))
    }

    /// Connect to WebSocket for real-time price updates
    pub async fn connect_websocket(
        &self,
        token_ids: Vec<String>,
        price_tx: mpsc::Sender<PriceUpdate>,
    ) -> Result<()> {
        info!("正在连接 Polymarket WebSocket...");

        let (ws_stream, _) = connect_async(POLY_WS_URL)
            .await
            .with_context(|| "连接 Polymarket WebSocket 失败")?;

        let (mut write, mut read) = ws_stream.split();

        // Subscribe to markets - 使用正确的格式（与 Python 版本一致）
        let subscribe_msg = json!({
            "assets_ids": token_ids,
            "type": "market"
        });

        write
            .send(Message::Text(subscribe_msg.to_string()))
            .await?;

        info!("已订阅 {} 个 Polymarket 代币", token_ids.len());

        // Process messages
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    // Parse message - may return multiple updates (especially for price_change)
                    let updates = Self::parse_ws_message(&text);
                    for update in updates {
                        if price_tx.send(update).await.is_err() {
                            warn!("价格更新通道已关闭");
                            return Ok(());
                        }
                    }
                }
                Ok(Message::Close(_)) => {
                    info!("Polymarket WebSocket 已关闭");
                    break;
                }
                Err(e) => {
                    error!("Polymarket WebSocket 错误: {}", e);
                    break;
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// Parse WebSocket message
    /// 
    /// Supports two message formats (matching Python implementation):
    /// 1. `book` (initial orderbook snapshot): { "event_type": "book", "asset_id": "...", "bids": [...], "asks": [...] }
    /// 2. `price_change` (real-time updates): { "event_type": "price_change", "price_changes": [{ "asset_id": "...", "best_bid": "...", "best_ask": "..." }, ...] }
    /// 
    /// Returns a Vec because price_change messages can contain multiple asset updates.
    fn parse_ws_message(text: &str) -> Vec<PriceUpdate> {
        let mut updates = Vec::new();
        
        // Parse JSON - handle both single object and array format
        let raw_data: Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(_) => return updates,
        };
        
        // Handle array format (WebSocket may return [{...}] instead of {...})
        let items: Vec<Value> = if raw_data.is_array() {
            raw_data.as_array().unwrap().clone()
        } else {
            vec![raw_data]
        };
        
        for data in items {
            if let Some(parsed) = Self::parse_single_message(&data) {
                updates.extend(parsed);
            }
        }
        
        updates
    }
    
    /// Parse a single message object
    fn parse_single_message(data: &Value) -> Option<Vec<PriceUpdate>> {
        let event_type = data.get("event_type").and_then(|v| v.as_str())?;
        
        match event_type {
            "book" => Self::parse_book_message(data),
            "price_change" => Self::parse_price_change_message(data),
            "connected" => None, // Ignore connection confirmation messages
            _ => None,
        }
    }
    
    /// Parse book message (initial orderbook snapshot)
    /// 
    /// Format: { "event_type": "book", "asset_id": "...", "bids": [...], "asks": [...] }
    /// - bids: sorted ascending by price, best bid (highest) is last
    /// - asks: sorted descending by price, best ask (lowest) is last
    fn parse_book_message(data: &Value) -> Option<Vec<PriceUpdate>> {
        let asset_id = data.get("asset_id").and_then(|v| v.as_str())?.to_string();
        let bids = data.get("bids").and_then(|v| v.as_array())?;
        let asks = data.get("asks").and_then(|v| v.as_array())?;
        
        // Best bid = highest price (last element, bids sorted ascending)
        let yes_bid = if !bids.is_empty() {
            bids.last().and_then(|entry| Self::extract_price_from_entry(entry))
        } else {
            None
        };
        
        // Best ask = lowest price (last element, asks sorted descending)
        let yes_ask = if !asks.is_empty() {
            asks.last().and_then(|entry| Self::extract_price_from_entry(entry))
        } else {
            None
        };
        
        Some(vec![PriceUpdate {
            platform: Platform::Polymarket,
            market_id: asset_id,
            yes_bid,
            yes_ask,
            no_bid: None,
            no_ask: None,
            timestamp: Utc::now(),
        }])
    }
    
    /// Parse price_change message (real-time price updates)
    /// 
    /// Format: { "event_type": "price_change", "market": "...", "price_changes": [{ "asset_id": "...", "best_bid": "...", "best_ask": "..." }, ...] }
    fn parse_price_change_message(data: &Value) -> Option<Vec<PriceUpdate>> {
        let price_changes = data.get("price_changes").and_then(|v| v.as_array())?;
        let mut updates = Vec::new();
        
        for change in price_changes {
            let asset_id = match change.get("asset_id").and_then(|v| v.as_str()) {
                Some(id) => id.to_string(),
                None => continue,
            };
            
            // Parse best_bid - can be string, number, null, or empty string
            let yes_bid = change.get("best_bid").and_then(|v| {
                if v.is_null() {
                    None
                } else if let Some(s) = v.as_str() {
                    if s.is_empty() { None } else { s.parse().ok() }
                } else {
                    v.as_f64()
                }
            });
            
            // Parse best_ask - can be string, number, null, or empty string
            let yes_ask = change.get("best_ask").and_then(|v| {
                if v.is_null() {
                    None
                } else if let Some(s) = v.as_str() {
                    if s.is_empty() { None } else { s.parse().ok() }
                } else {
                    v.as_f64()
                }
            });
            
            updates.push(PriceUpdate {
                platform: Platform::Polymarket,
                market_id: asset_id,
                yes_bid,
                yes_ask,
                no_bid: None,
                no_ask: None,
                timestamp: Utc::now(),
            });
        }
        
        if updates.is_empty() {
            None
        } else {
            Some(updates)
        }
    }
    
    /// Extract price from an orderbook entry
    /// 
    /// Entry can be either:
    /// - Object: { "price": "0.50", "size": "100" }
    /// - Array: [0.50, 100] (price, size)
    fn extract_price_from_entry(entry: &Value) -> Option<f64> {
        if let Some(obj) = entry.as_object() {
            // Object format: { "price": "0.50", ... }
            obj.get("price").and_then(|p| {
                p.as_str().and_then(|s| s.parse().ok()).or(p.as_f64())
            })
        } else if let Some(arr) = entry.as_array() {
            // Array format: [price, size]
            arr.first().and_then(|p| {
                p.as_str().and_then(|s| s.parse().ok()).or(p.as_f64())
            })
        } else {
            None
        }
    }
}

/// Extract date from slug (e.g., "lakers-vs-grizzlies-2026-01-07" -> 2026-01-07)
fn extract_date_from_slug(slug: &str) -> Option<DateTime<Utc>> {
    let parts: Vec<&str> = slug.split('-').collect();
    if parts.len() >= 3 {
        let year_str = parts[parts.len() - 3];
        let month_str = parts[parts.len() - 2];
        let day_str = parts[parts.len() - 1];

        if let (Ok(year), Ok(month), Ok(day)) = (
            year_str.parse::<i32>(),
            month_str.parse::<u32>(),
            day_str.parse::<u32>(),
        ) {
            use chrono::NaiveDate;
            if let Some(naive_date) = NaiveDate::from_ymd_opt(year, month, day) {
                let naive_datetime = naive_date.and_hms_opt(12, 0, 0)?;
                return Some(DateTime::from_naive_utc_and_offset(naive_datetime, Utc));
            }
        }
    }
    None
}

/// Normalize team name to standard abbreviation
fn normalize_team_name(name: &str) -> String {
    use std::collections::HashMap;
    
    // NBA team mappings (full name/alias -> standard abbreviation)
    let mut mappings = HashMap::new();
    
    // Eastern Conference
    mappings.insert("ATLANTA HAWKS", "ATL");
    mappings.insert("HAWKS", "ATL");
    mappings.insert("ATL", "ATL");
    mappings.insert("BOSTON CELTICS", "BOS");
    mappings.insert("CELTICS", "BOS");
    mappings.insert("BOS", "BOS");
    mappings.insert("BROOKLYN NETS", "BKN");
    mappings.insert("NETS", "BKN");
    mappings.insert("BKN", "BKN");
    mappings.insert("BRK", "BKN");
    mappings.insert("CHARLOTTE HORNETS", "CHA");
    mappings.insert("HORNETS", "CHA");
    mappings.insert("CHA", "CHA");
    mappings.insert("CHO", "CHA");
    mappings.insert("CHICAGO BULLS", "CHI");
    mappings.insert("BULLS", "CHI");
    mappings.insert("CHI", "CHI");
    mappings.insert("CLEVELAND CAVALIERS", "CLE");
    mappings.insert("CAVALIERS", "CLE");
    mappings.insert("CAVS", "CLE");
    mappings.insert("CLE", "CLE");
    mappings.insert("DETROIT PISTONS", "DET");
    mappings.insert("PISTONS", "DET");
    mappings.insert("DET", "DET");
    mappings.insert("INDIANA PACERS", "IND");
    mappings.insert("PACERS", "IND");
    mappings.insert("IND", "IND");
    mappings.insert("MIAMI HEAT", "MIA");
    mappings.insert("HEAT", "MIA");
    mappings.insert("MIA", "MIA");
    mappings.insert("MILWAUKEE BUCKS", "MIL");
    mappings.insert("BUCKS", "MIL");
    mappings.insert("MIL", "MIL");
    mappings.insert("NEW YORK KNICKS", "NYK");
    mappings.insert("KNICKS", "NYK");
    mappings.insert("NYK", "NYK");
    mappings.insert("NY", "NYK");
    mappings.insert("ORLANDO MAGIC", "ORL");
    mappings.insert("MAGIC", "ORL");
    mappings.insert("ORL", "ORL");
    mappings.insert("PHILADELPHIA 76ERS", "PHI");
    mappings.insert("76ERS", "PHI");
    mappings.insert("SIXERS", "PHI");
    mappings.insert("PHI", "PHI");
    mappings.insert("TORONTO RAPTORS", "TOR");
    mappings.insert("RAPTORS", "TOR");
    mappings.insert("TOR", "TOR");
    mappings.insert("WASHINGTON WIZARDS", "WAS");
    mappings.insert("WIZARDS", "WAS");
    mappings.insert("WAS", "WAS");
    mappings.insert("WSH", "WAS");
    
    // Western Conference
    mappings.insert("DALLAS MAVERICKS", "DAL");
    mappings.insert("MAVERICKS", "DAL");
    mappings.insert("MAVS", "DAL");
    mappings.insert("DAL", "DAL");
    mappings.insert("DENVER NUGGETS", "DEN");
    mappings.insert("NUGGETS", "DEN");
    mappings.insert("DEN", "DEN");
    mappings.insert("GOLDEN STATE WARRIORS", "GSW");
    mappings.insert("WARRIORS", "GSW");
    mappings.insert("GSW", "GSW");
    mappings.insert("GS", "GSW");
    mappings.insert("HOUSTON ROCKETS", "HOU");
    mappings.insert("ROCKETS", "HOU");
    mappings.insert("HOU", "HOU");
    mappings.insert("LOS ANGELES CLIPPERS", "LAC");
    mappings.insert("CLIPPERS", "LAC");
    mappings.insert("LAC", "LAC");
    mappings.insert("LA CLIPPERS", "LAC");
    mappings.insert("LOS ANGELES LAKERS", "LAL");
    mappings.insert("LAKERS", "LAL");
    mappings.insert("LAL", "LAL");
    mappings.insert("LA LAKERS", "LAL");
    mappings.insert("MEMPHIS GRIZZLIES", "MEM");
    mappings.insert("GRIZZLIES", "MEM");
    mappings.insert("MEM", "MEM");
    mappings.insert("MINNESOTA TIMBERWOLVES", "MIN");
    mappings.insert("TIMBERWOLVES", "MIN");
    mappings.insert("WOLVES", "MIN");
    mappings.insert("MIN", "MIN");
    mappings.insert("NEW ORLEANS PELICANS", "NOP");
    mappings.insert("PELICANS", "NOP");
    mappings.insert("NOP", "NOP");
    mappings.insert("NO", "NOP");
    mappings.insert("OKLAHOMA CITY THUNDER", "OKC");
    mappings.insert("THUNDER", "OKC");
    mappings.insert("OKC", "OKC");
    mappings.insert("PHOENIX SUNS", "PHX");
    mappings.insert("SUNS", "PHX");
    mappings.insert("PHX", "PHX");
    mappings.insert("PHO", "PHX");
    mappings.insert("PORTLAND TRAIL BLAZERS", "POR");
    mappings.insert("TRAIL BLAZERS", "POR");
    mappings.insert("BLAZERS", "POR");
    mappings.insert("POR", "POR");
    mappings.insert("SACRAMENTO KINGS", "SAC");
    mappings.insert("KINGS", "SAC");
    mappings.insert("SAC", "SAC");
    mappings.insert("SAN ANTONIO SPURS", "SAS");
    mappings.insert("SPURS", "SAS");
    mappings.insert("SAS", "SAS");
    mappings.insert("UTAH JAZZ", "UTA");
    mappings.insert("JAZZ", "UTA");
    mappings.insert("UTA", "UTA");
    
    let name_upper = name.trim().to_uppercase();
    mappings
        .get(name_upper.as_str())
        .map(|s| s.to_string())
        .unwrap_or(name_upper)
}
