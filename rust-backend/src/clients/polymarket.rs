//! Polymarket platform client
//!
//! Handles Polymarket API interactions including:
//! - Market data retrieval from Gamma API
//! - WebSocket price subscription
//! - Order placement via CLOB client
//! - Local orderbook maintenance for depth queries

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use parking_lot::RwLock;
use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

use crate::clob::{ApiCreds, ClobClient, MarketOrderArgs, Side, SignatureType};
use crate::config::PolymarketConfig;
use crate::core::normalize_team_name;
use crate::models::{Platform, PolymarketEvent, PolymarketMarket, PriceUpdate};
use crate::utils;

/// Polymarket order book structure (price, size)
#[derive(Debug, Clone, Default)]
pub struct PolyOrderBook {
    /// Bids sorted by price ascending (best bid = last)
    pub bids: Vec<(f64, f64)>,
    /// Asks sorted by price descending (best ask = last)
    pub asks: Vec<(f64, f64)>,
}

impl PolyOrderBook {
    /// Get best bid (highest price)
    pub fn best_bid(&self) -> Option<(f64, f64)> {
        self.bids.last().copied()
    }

    /// Get best ask (lowest price)
    pub fn best_ask(&self) -> Option<(f64, f64)> {
        self.asks.last().copied()
    }

    /// 计算 asks 侧的可用深度（买入时使用）
    /// 返回在指定金额内可获得的总深度（USD）
    /// asks 降序排列，last 是 best_ask（最低卖价）
    pub fn ask_depth(&self, max_amount: f64) -> f64 {
        let mut depth = 0.0;
        // 从 best_ask（最低价）开始累计
        for (price, size) in self.asks.iter().rev() {
            let level_value = price * size;
            depth += level_value;
            if depth >= max_amount {
                return max_amount;
            }
        }
        depth
    }

    /// 计算 bids 侧的可用深度（卖出时使用）
    pub fn bid_depth(&self, max_amount: f64) -> f64 {
        let mut depth = 0.0;
        // 从 best_bid（最高价）开始累计
        for (price, size) in self.bids.iter().rev() {
            let level_value = price * size;
            depth += level_value;
            if depth >= max_amount {
                return max_amount;
            }
        }
        depth
    }
}

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

/// Subscription command for Polymarket WebSocket
#[derive(Debug, Clone)]
pub enum PolyWsCommand {
    Subscribe(Vec<String>),
    Unsubscribe(Vec<String>),
}

/// Polymarket API client
#[derive(Clone)]
pub struct PolymarketClient {
    pub config: PolymarketConfig,
    http: Client,
    /// CLOB client for order operations
    clob: Option<Arc<ClobClient>>,
    /// Order book cache: token_id -> PolyOrderBook
    orderbook_cache: Arc<RwLock<HashMap<String, PolyOrderBook>>>,
    /// Channel sender for dynamic subscriptions/unsubscriptions
    command_tx: Arc<RwLock<Option<mpsc::Sender<PolyWsCommand>>>>,
}

impl PolymarketClient {
    /// Create a new Polymarket client
    pub fn new(config: PolymarketConfig) -> Self {
        Self {
            config,
            http: Client::new(),
            clob: None,
            orderbook_cache: Arc::new(RwLock::new(HashMap::new())),
            command_tx: Arc::new(RwLock::new(None)),
        }
    }

    /// Get order book from cache
    pub fn get_orderbook(&self, token_id: &str) -> Option<PolyOrderBook> {
        self.orderbook_cache.read().get(token_id).cloned()
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

    /// Place a market order by specifying tokens quantity (RECOMMENDED)
    /// 
    /// This method:
    /// - Takes `tokens` as the number of tokens/contracts to buy
    /// - Calculates USDC needed by traversing order book from best price
    /// - Uses 5% slippage for protection
    /// - Uses FAK (Fill and Kill) mode
    /// 
    /// This works like Kalshi's IOC - fills at best available prices level by level
    pub async fn place_market_order_by_tokens(
        &self,
        token_id: &str,
        side: &str,
        tokens: f64,  // Number of tokens/contracts to buy
    ) -> Result<Value> {
        let token_short = &token_id[..20.min(token_id.len())];
        info!("════════════════════════════════════════════════════════════");
        info!("🎯 [Poly下单-Tokens模式] token={}..., side={}, tokens={:.4}", token_short, side, tokens);
        info!("════════════════════════════════════════════════════════════");
        
        let clob = self
            .clob
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("CLOB client not initialized"))?;

        let order_side = if side.to_lowercase() == "buy" {
            Side::Buy
        } else {
            Side::Sell
        };

        // Use the new create_market_order_by_tokens with 5% slippage
        info!("🔐 [Step 1] 创建并签名订单 (Tokens模式, 5%滑点)...");
        let signed_order = match clob.as_ref().create_market_order_by_tokens(
            token_id,
            order_side,
            tokens,
            Some(0.05),  // 5% slippage
        ).await {
            Ok(order) => {
                info!("   ✅ 订单创建成功:");
                info!("      maker_amount: {} (tokens)", order.maker_amount);
                info!("      taker_amount: {} (USDC with slippage)", order.taker_amount);
                info!("      salt: {}", order.salt);
                info!("      signature: {}...", &order.signature[..20.min(order.signature.len())]);
                order
            }
            Err(e) => {
                tracing::error!("   ❌ 订单创建失败: {}", e);
                Self::write_debug_log("place_market_order_by_tokens:create_failed", serde_json::json!({
                    "token_id": token_id,
                    "side": side,
                    "tokens": tokens,
                    "error": e.to_string(),
                }));
                return Err(e);
            }
        };
        
        // Step 2: Post order
        info!("📤 [Step 2] 提交订单到Polymarket...");
        let response = match clob
            .as_ref()
            .post_order(&signed_order, crate::clob::OrderType::Fak)
            .await
        {
            Ok(resp) => {
                info!("   ✅ 订单提交成功!");
                info!("════════════════════════════════════════════════════════════");
                resp
            }
            Err(e) => {
                tracing::error!("   ❌ 订单提交失败: {}", e);
                info!("════════════════════════════════════════════════════════════");
                
                Self::write_debug_log("place_market_order_by_tokens:post_failed", serde_json::json!({
                    "token_id": token_id,
                    "side": side,
                    "tokens": tokens,
                    "signed_order": {
                        "maker_amount": signed_order.maker_amount,
                        "taker_amount": signed_order.taker_amount,
                        "salt": signed_order.salt,
                        "maker": signed_order.maker,
                        "signer": signed_order.signer,
                        "signature_type": signed_order.signature_type,
                    },
                    "error": e.to_string(),
                }));
                return Err(e);
            }
        };

        Ok(serde_json::to_value(response)?)
    }

    /// Place a market order (legacy method - uses USDC amount)
    /// Uses FAK (Fill and Kill) mode - fills as much as possible, cancels the rest immediately
    pub async fn place_market_order(
        &self,
        token_id: &str,
        side: &str,
        amount: f64,
    ) -> Result<Value> {
        let token_short = &token_id[..20.min(token_id.len())];
        info!("════════════════════════════════════════════════════════════");
        info!("🎯 [Poly下单开始-旧模式] token={}..., side={}, amount={:.4}", token_short, side, amount);
        info!("════════════════════════════════════════════════════════════");
        
        let clob = self
            .clob
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("CLOB client not initialized"))?;

        // Step 1: Pre-order validation - check orderbook
        info!("📊 [Step 1] 获取订单簿状态...");
        match clob.as_ref().get_order_book(token_id).await {
            Ok(orderbook) => {
                let best_bid = orderbook.bids.last().map(|(p, s)| format!("{:.4}@{:.2}", s, p));
                let best_ask = orderbook.asks.first().map(|(p, s)| format!("{:.4}@{:.2}", s, p));
                let bid_depth: f64 = orderbook.bids.iter().map(|(p, s)| p * s).sum();
                let ask_depth: f64 = orderbook.asks.iter().map(|(p, s)| p * s).sum();
                info!("   ✅ 订单簿有效: best_bid={:?}, best_ask={:?}", best_bid, best_ask);
                info!("   📈 深度: bid_depth=${:.2}, ask_depth=${:.2}, bid_levels={}, ask_levels={}", 
                    bid_depth, ask_depth, orderbook.bids.len(), orderbook.asks.len());
            }
            Err(e) => {
                tracing::error!("   ❌ 订单簿获取失败: {}", e);
                // Write to debug log
                Self::write_debug_log("place_market_order:orderbook_failed", serde_json::json!({
                    "token_id": token_id,
                    "side": side,
                    "amount": amount,
                    "error": e.to_string(),
                }));
            }
        }
        
        // Step 2: Get tick size and neg_risk
        info!("📏 [Step 2] 获取tick_size和neg_risk...");
        let tick_size = match clob.as_ref().get_tick_size(token_id).await {
            Ok(ts) => {
                info!("   ✅ tick_size={}", ts);
                ts
            }
            Err(e) => {
                tracing::error!("   ❌ tick_size获取失败: {}", e);
                0.01 // default
            }
        };
        
        let neg_risk = match clob.as_ref().get_neg_risk(token_id).await {
            Ok(nr) => {
                info!("   ✅ neg_risk={}", nr);
                nr
            }
            Err(e) => {
                tracing::error!("   ❌ neg_risk获取失败: {}", e);
                false // default
            }
        };

        let order_side = if side.to_lowercase() == "buy" {
            Side::Buy
        } else {
            Side::Sell
        };

        // Step 3: Create order args
        info!("📝 [Step 3] 构建订单参数...");
        let order_args = MarketOrderArgs {
            token_id: token_id.to_string(),
            amount,
            side: order_side,
            fee_rate_bps: None,
            nonce: None,
            slippage: Some(0.02), // 2% slippage for better fill rate
        };
        info!("   token_id: {}", token_id);
        info!("   amount: {:.4}", amount);
        info!("   side: {:?}", order_side);
        info!("   slippage: 2%");
        
        // Step 4: Create signed order
        info!("🔐 [Step 4] 创建并签名订单...");
        let signed_order = match clob.as_ref().create_market_order(&order_args).await {
            Ok(order) => {
                info!("   ✅ 订单创建成功:");
                info!("      maker_amount: {}", order.maker_amount);
                info!("      taker_amount: {}", order.taker_amount);
                info!("      salt: {}", order.salt);
                info!("      signature: {}...", &order.signature[..20.min(order.signature.len())]);
                order
            }
            Err(e) => {
                tracing::error!("   ❌ 订单创建失败: {}", e);
                Self::write_debug_log("place_market_order:create_failed", serde_json::json!({
                    "token_id": token_id,
                    "side": side,
                    "amount": amount,
                    "tick_size": tick_size,
                    "neg_risk": neg_risk,
                    "error": e.to_string(),
                }));
                return Err(e);
            }
        };
        
        // Step 5: Post order
        info!("📤 [Step 5] 提交订单到Polymarket...");
        let response = match clob
            .as_ref()
            .post_order(&signed_order, crate::clob::OrderType::Fak)
            .await
        {
            Ok(resp) => {
                info!("   ✅ 订单提交成功!");
                info!("════════════════════════════════════════════════════════════");
                resp
            }
            Err(e) => {
                tracing::error!("   ❌ 订单提交失败: {}", e);
                info!("════════════════════════════════════════════════════════════");
                
                // Write comprehensive debug log for failed orders
                Self::write_debug_log("place_market_order:post_failed", serde_json::json!({
                    "token_id": token_id,
                    "side": side,
                    "amount": amount,
                    "tick_size": tick_size,
                    "neg_risk": neg_risk,
                    "signed_order": {
                        "maker_amount": signed_order.maker_amount,
                        "taker_amount": signed_order.taker_amount,
                        "salt": signed_order.salt,
                        "maker": signed_order.maker,
                        "signer": signed_order.signer,
                        "signature_type": signed_order.signature_type,
                    },
                    "error": e.to_string(),
                }));
                return Err(e);
            }
        };

        Ok(serde_json::to_value(response)?)
    }
    
    /// Write debug log to file
    fn write_debug_log(location: &str, data: serde_json::Value) {
        utils::write_debug_log(
            &format!("polymarket.rs:{}", location),
            "Polymarket操作",
            data
        );
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

    /// Subscribe to additional tokens dynamically (hot subscription)
    ///
    /// This can be called after the WebSocket connection is established
    /// to add new token subscriptions.
    pub async fn subscribe_tokens(&self, token_ids: Vec<String>) -> Result<bool> {
        if token_ids.is_empty() {
            return Ok(true);
        }

        let tx = self.command_tx.read().clone();
        if let Some(tx) = tx {
            match tx.send(PolyWsCommand::Subscribe(token_ids.clone())).await {
                Ok(_) => {
                    info!("🔌 [Polymarket] 发送热订阅请求: {} 个 token", token_ids.len());
                    Ok(true)
                }
                Err(e) => {
                    warn!("⚠️ [Polymarket] 热订阅请求发送失败: {}", e);
                    Ok(false)
                }
            }
        } else {
            warn!("⚠️ [Polymarket] WebSocket 未连接，无法热订阅");
            Ok(false)
        }
    }

    /// Unsubscribe from tokens dynamically
    ///
    /// This can be called after the WebSocket connection is established
    /// to remove token subscriptions for ended games.
    /// Uses the operation: "unsubscribe" field as per Polymarket API.
    pub async fn unsubscribe_tokens(&self, token_ids: Vec<String>) -> Result<bool> {
        if token_ids.is_empty() {
            return Ok(true);
        }

        let tx = self.command_tx.read().clone();
        if let Some(tx) = tx {
            match tx.send(PolyWsCommand::Unsubscribe(token_ids.clone())).await {
                Ok(_) => {
                    info!("🔌 [Polymarket] 发送取消订阅请求: {} 个 token", token_ids.len());
                    Ok(true)
                }
                Err(e) => {
                    warn!("⚠️ [Polymarket] 取消订阅请求发送失败: {}", e);
                    Ok(false)
                }
            }
        } else {
            warn!("⚠️ [Polymarket] WebSocket 未连接，无法取消订阅");
            Ok(false)
        }
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

        // Create channel for dynamic subscriptions/unsubscriptions
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<PolyWsCommand>(100);
        *self.command_tx.write() = Some(cmd_tx);

        // Subscribe to initial markets - 使用正确的格式（与 Python 版本一致）
        let subscribe_msg = json!({
            "assets_ids": token_ids,
            "type": "market"
        });

        write
            .send(Message::Text(subscribe_msg.to_string()))
            .await?;

        info!("已订阅 {} 个 Polymarket 代币", token_ids.len());

        let orderbook_cache = self.orderbook_cache.clone();

        // Process messages with dynamic subscription/unsubscription support
        loop {
            tokio::select! {
                // Handle incoming WebSocket messages
                msg = read.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            // Parse message and update orderbook cache
                            let updates = Self::parse_ws_message(&text, &orderbook_cache);
                            for update in updates {
                                if price_tx.send(update).await.is_err() {
                                    warn!("价格更新通道已关闭");
                                    break;
                                }
                            }
                        }
                        Some(Ok(Message::Close(_))) => {
                            info!("Polymarket WebSocket 已关闭");
                            break;
                        }
                        Some(Err(e)) => {
                            error!("Polymarket WebSocket 错误: {}", e);
                            break;
                        }
                        None => {
                            info!("Polymarket WebSocket 流结束");
                            break;
                        }
                        _ => {}
                    }
                }
                // Handle dynamic subscription/unsubscription requests
                Some(command) = cmd_rx.recv() => {
                    match command {
                        PolyWsCommand::Subscribe(new_tokens) => {
                            info!("🔌 [Polymarket] 处理热订阅: {} 个新 token", new_tokens.len());
                            let subscribe_msg = json!({
                                "assets_ids": new_tokens,
                                "type": "market",
                                "operation": "subscribe"
                            });

                            if let Err(e) = write.send(Message::Text(subscribe_msg.to_string())).await {
                                error!("❌ [Polymarket] 热订阅发送失败: {}", e);
                            } else {
                                info!("✅ [Polymarket] 热订阅完成: {} 个 token", new_tokens.len());
                            }
                        }
                        PolyWsCommand::Unsubscribe(tokens_to_unsub) => {
                            info!("🔌 [Polymarket] 处理取消订阅: {} 个 token", tokens_to_unsub.len());
                            // Use the operation: "unsubscribe" field as per Polymarket API
                            let unsubscribe_msg = json!({
                                "assets_ids": tokens_to_unsub,
                                "type": "market",
                                "operation": "unsubscribe"
                            });

                            if let Err(e) = write.send(Message::Text(unsubscribe_msg.to_string())).await {
                                error!("❌ [Polymarket] 取消订阅发送失败: {}", e);
                            } else {
                                // Also remove from orderbook cache
                                let mut cache = orderbook_cache.write();
                                for token in &tokens_to_unsub {
                                    cache.remove(token);
                                }
                                info!("✅ [Polymarket] 取消订阅完成: {} 个 token", tokens_to_unsub.len());
                            }
                        }
                    }
                }
            }
        }

        // Clear command channel on disconnect
        *self.command_tx.write() = None;

        Ok(())
    }

    /// Parse WebSocket message and update orderbook cache
    /// 
    /// Supports two message formats (matching Python implementation):
    /// 1. `book` (initial orderbook snapshot): { "event_type": "book", "asset_id": "...", "bids": [...], "asks": [...] }
    /// 2. `price_change` (real-time updates): { "event_type": "price_change", "price_changes": [{ "asset_id": "...", "price": "...", "size": "...", "side": "..." }, ...] }
    /// 
    /// Returns a Vec because price_change messages can contain multiple asset updates.
    fn parse_ws_message(
        text: &str,
        orderbook_cache: &Arc<RwLock<HashMap<String, PolyOrderBook>>>,
    ) -> Vec<PriceUpdate> {
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
            if let Some(parsed) = Self::parse_single_message(&data, orderbook_cache) {
                updates.extend(parsed);
            }
        }
        
        updates
    }
    
    /// Parse a single message object
    fn parse_single_message(
        data: &Value,
        orderbook_cache: &Arc<RwLock<HashMap<String, PolyOrderBook>>>,
    ) -> Option<Vec<PriceUpdate>> {
        let event_type = data.get("event_type").and_then(|v| v.as_str())?;
        
        match event_type {
            "book" => Self::parse_book_message(data, orderbook_cache),
            "price_change" => Self::parse_price_change_message(data, orderbook_cache),
            "connected" => None, // Ignore connection confirmation messages
            _ => None,
        }
    }
    
    /// Parse book message (initial orderbook snapshot)
    /// 
    /// Format: { "event_type": "book", "asset_id": "...", "bids": [...], "asks": [...] }
    /// - bids: sorted ascending by price, best bid (highest) is last
    /// - asks: sorted descending by price, best ask (lowest) is last
    fn parse_book_message(
        data: &Value,
        orderbook_cache: &Arc<RwLock<HashMap<String, PolyOrderBook>>>,
    ) -> Option<Vec<PriceUpdate>> {
        let asset_id = data.get("asset_id").and_then(|v| v.as_str())?.to_string();
        let bids = data.get("bids").and_then(|v| v.as_array())?;
        let asks = data.get("asks").and_then(|v| v.as_array())?;
        
        // Build orderbook from snapshot
        let mut book = PolyOrderBook::default();
        
        for entry in bids {
            if let Some((price, size)) = Self::extract_price_size_from_entry(entry) {
                book.bids.push((price, size));
            }
        }
        
        for entry in asks {
            if let Some((price, size)) = Self::extract_price_size_from_entry(entry) {
                book.asks.push((price, size));
            }
        }
        
        // Sort: bids ascending by price, asks descending by price
        book.bids.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        book.asks.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        
        // Get best prices for PriceUpdate
        let yes_bid = book.best_bid().map(|(p, _)| p);
        let yes_ask = book.best_ask().map(|(p, _)| p);
        
        // Store in cache
        orderbook_cache.write().insert(asset_id.clone(), book);
        
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
    /// Format: { "event_type": "price_change", "price_changes": [{ "asset_id": "...", "price": "...", "size": "...", "side": "BUY"|"SELL", "best_bid": "...", "best_ask": "..." }, ...] }
    fn parse_price_change_message(
        data: &Value,
        orderbook_cache: &Arc<RwLock<HashMap<String, PolyOrderBook>>>,
    ) -> Option<Vec<PriceUpdate>> {
        let price_changes = data.get("price_changes").and_then(|v| v.as_array())?;
        let mut updates = Vec::new();
        
        for change in price_changes {
            let asset_id = match change.get("asset_id").and_then(|v| v.as_str()) {
                Some(id) => id.to_string(),
                None => continue,
            };
            
            // Update orderbook cache with price/size/side delta
            let delta_price = change.get("price").and_then(|v| Self::parse_string_or_number(v));
            let delta_size = change.get("size").and_then(|v| Self::parse_string_or_number(v));
            let delta_side = change.get("side").and_then(|v| v.as_str());
            
            if let (Some(price), Some(size), Some(side)) = (delta_price, delta_size, delta_side) {
                let mut cache = orderbook_cache.write();
                let book = cache.entry(asset_id.clone()).or_insert_with(PolyOrderBook::default);
                
                let book_side = if side.to_uppercase() == "BUY" {
                    &mut book.bids
                } else {
                    &mut book.asks
                };
                
                // Find existing price level and update or insert
                if let Some(pos) = book_side.iter().position(|(p, _)| (*p - price).abs() < 0.0001) {
                    if size <= 0.0 {
                        // Remove level if size is 0
                        book_side.remove(pos);
                    } else {
                        // Update size
                        book_side[pos].1 = size;
                    }
                } else if size > 0.0 {
                    // Insert new level
                    book_side.push((price, size));
                    // Re-sort: bids ascending, asks descending
                    if side.to_uppercase() == "BUY" {
                        book_side.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                    } else {
                        book_side.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
                    }
                }
            }
            
            // Parse best_bid/best_ask for PriceUpdate (fallback to cache if not provided)
            let raw_best_bid = change.get("best_bid");
            let raw_best_ask = change.get("best_ask");
            let parsed_bid = raw_best_bid.and_then(|v| Self::parse_string_or_number(v));
            let parsed_ask = raw_best_ask.and_then(|v| Self::parse_string_or_number(v));
            
            // 重要修复: 始终从本地订单簿缓存获取 best_ask
            // Polymarket 的 price_change 消息中的 best_ask 字段不可信（可能是过时的）
            // 我们自己维护的订单簿是通过 delta 更新的，更准确
            let cache_bid_final = orderbook_cache.read().get(&asset_id).and_then(|b| b.best_bid()).map(|(p, _)| p);
            let cache_ask_final = orderbook_cache.read().get(&asset_id).and_then(|b| b.best_ask()).map(|(p, _)| p);
            
            let yes_bid = cache_bid_final.or(parsed_bid);
            let yes_ask = cache_ask_final.or(parsed_ask);
            
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
    
    /// Parse a value that can be string or number
    fn parse_string_or_number(v: &Value) -> Option<f64> {
        if v.is_null() {
            None
        } else if let Some(s) = v.as_str() {
            if s.is_empty() { None } else { s.parse().ok() }
        } else {
            v.as_f64()
        }
    }
    
    /// Extract price from an orderbook entry
    /// 
    /// Entry can be either:
    /// - Object: { "price": "0.50", "size": "100" }
    /// - Array: [0.50, 100] (price, size)
    #[allow(dead_code)]
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

    /// Extract price and size from an orderbook entry
    /// 
    /// Entry can be either:
    /// - Object: { "price": "0.50", "size": "100" }
    /// - Array: [price, size]
    fn extract_price_size_from_entry(entry: &Value) -> Option<(f64, f64)> {
        if let Some(obj) = entry.as_object() {
            // Object format: { "price": "0.50", "size": "100" }
            let price = obj.get("price").and_then(|p| Self::parse_string_or_number(p))?;
            let size = obj.get("size").and_then(|s| Self::parse_string_or_number(s))?;
            Some((price, size))
        } else if let Some(arr) = entry.as_array() {
            // Array format: [price, size]
            let price = arr.first().and_then(|p| Self::parse_string_or_number(p))?;
            let size = arr.get(1).and_then(|s| Self::parse_string_or_number(s))?;
            Some((price, size))
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

