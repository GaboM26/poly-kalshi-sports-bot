//! Polymarket platform client
//!
//! Handles Polymarket API interactions including:
//! - Market data retrieval from Gamma API
//! - WebSocket price subscription
//! - Order placement via Python order service
//! - Local orderbook maintenance for depth queries

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use parking_lot::RwLock;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

use crate::config::PolymarketConfig;
use crate::core::normalize_team_name;
use crate::models::{Platform, PolymarketEvent, PolymarketMarket, PriceUpdate};

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

// ==================== Python Order Service Types ====================

/// Market order request to Python service
#[derive(Debug, Serialize)]
struct MarketOrderRequest {
    token_id: String,
    side: String,
    amount: f64,
    price: Option<f64>,  // Rust计算的价格（包含滑点），避免Python重复获取订单簿
    order_type: Option<String>,
}

/// Limit order request to Python service
#[derive(Debug, Serialize)]
struct LimitOrderRequest {
    token_id: String,
    side: String,
    price: f64,
    size: f64,
    order_type: Option<String>,
}

/// Cancel order request to Python service
#[derive(Debug, Serialize)]
struct CancelOrderRequest {
    order_id: String,
}

/// Order response from Python service
#[derive(Debug, Deserialize)]
struct OrderResponse {
    success: bool,
    order_id: Option<String>,
    status: Option<String>,
    error: Option<String>,
    data: Option<Value>,
    latency_ms: Option<i64>,  // Python服务内部的API调用延迟
}

/// Position data for aggregation (internal use)
#[allow(dead_code)]
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
            orderbook_cache: Arc::new(RwLock::new(HashMap::new())),
            command_tx: Arc::new(RwLock::new(None)),
        }
    }

    /// Get order book from cache
    pub fn get_orderbook(&self, token_id: &str) -> Option<PolyOrderBook> {
        self.orderbook_cache.read().get(token_id).cloned()
    }

    /// Initialize client (check if Python order service is available)
    pub async fn init_clob(&mut self) -> Result<()> {
        // Check if Python order service is running
        let health_url = format!("{}/health", self.config.order_service_url);
        match self.http.get(&health_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                info!("✅ Polymarket Python 下单服务已连接: {}", self.config.order_service_url);
            }
            Ok(resp) => {
                warn!("⚠️ Polymarket Python 下单服务返回非成功状态: {}", resp.status());
            }
            Err(e) => {
                warn!("⚠️ Polymarket Python 下单服务未运行: {}. 下单功能将不可用.", e);
            }
        }
        Ok(())
    }

    /// Get account balance via Python service
    pub async fn get_balance(&self) -> Result<f64> {
        let url = format!("{}/balance", self.config.order_service_url);
        let resp = self.http.get(&url)
            .send()
            .await
            .context("调用 Python 下单服务失败")?;

        if !resp.status().is_success() {
            anyhow::bail!("获取余额失败: HTTP {}", resp.status());
        }

        let data: Value = resp.json().await?;
        if data.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
            let balance = data.get("balance")
                .and_then(|b| b.get("balance"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.0);
            Ok(balance / 1_000_000.0) // USDC has 6 decimals
        } else {
            let error = data.get("error").and_then(|v| v.as_str()).unwrap_or("Unknown error");
            anyhow::bail!("获取余额失败: {}", error)
        }
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

    // ========================================================================
    // ORDER PLACEMENT API (via Python service)
    // ========================================================================

    /// Market buy - buy tokens worth a specific USDC amount
    ///
    /// # Arguments
    /// * `token_id` - The token/asset ID to buy
    /// * `usdc_amount` - Amount of USDC to spend
    pub async fn market_buy(&self, token_id: &str, usdc_amount: f64) -> Result<Value> {
        let token_short = &token_id[..20.min(token_id.len())];
        info!("════════════════════════════════════════════════════════════");
        info!("🎯 [市价买入] token={}..., usdc_amount={:.4}", token_short, usdc_amount);
        
        // 从本地订单簿缓存获取价格并计算滑点
        let price = if let Some(book) = self.get_orderbook(token_id) {
            if let Some((best_ask, _)) = book.best_ask() {
                let price_with_slippage = (best_ask + 0.02).min(0.99);
                info!("   📊 使用本地订单簿: best_ask={:.4}, 下单价={:.4} (+0.02滑点)", best_ask, price_with_slippage);
                Some(price_with_slippage)
            } else {
                warn!("   ⚠️ 本地订单簿无卖单，Python将从API获取");
                None
            }
        } else {
            warn!("   ⚠️ 本地订单簿不存在，Python将从API获取");
            None
        };
        
        info!("════════════════════════════════════════════════════════════");

        let url = format!("{}/order/market", self.config.order_service_url);
        let request = MarketOrderRequest {
            token_id: token_id.to_string(),
            side: "buy".to_string(),
            amount: usdc_amount,
            price,  // 传递Rust计算的价格
            order_type: Some("FAK".to_string()),
        };

        let resp = self.http.post(&url)
            .json(&request)
            .send()
            .await
            .context("调用 Python 下单服务失败")?;

        let response: OrderResponse = resp.json().await?;

        if response.success {
            info!("   ✅ 市价买入成功! order_id={:?}", response.order_id);
            info!("════════════════════════════════════════════════════════════");
            Ok(response.data.unwrap_or(json!({"success": true, "order_id": response.order_id})))
        } else {
            let error = response.error.unwrap_or_else(|| "Unknown error".to_string());
            error!("   ❌ 市价买入失败: {}", error);
            info!("════════════════════════════════════════════════════════════");
            anyhow::bail!("市价买入失败: {}", error)
        }
    }

    /// Market sell - sell a specific number of tokens
    ///
    /// # Arguments
    /// * `token_id` - The token/asset ID to sell
    /// * `tokens` - Number of tokens to sell
    pub async fn market_sell(&self, token_id: &str, tokens: f64) -> Result<Value> {
        let token_short = &token_id[..20.min(token_id.len())];
        info!("════════════════════════════════════════════════════════════");
        info!("🎯 [市价卖出] token={}..., tokens={:.4}", token_short, tokens);
        
        // 从本地订单簿缓存获取价格并计算滑点
        let price = if let Some(book) = self.get_orderbook(token_id) {
            if let Some((best_bid, _)) = book.best_bid() {
                let price_with_slippage = (best_bid - 0.02).max(0.01);
                info!("   📊 使用本地订单簿: best_bid={:.4}, 下单价={:.4} (-0.02滑点)", best_bid, price_with_slippage);
                Some(price_with_slippage)
            } else {
                warn!("   ⚠️ 本地订单簿无买单，Python将从API获取");
                None
            }
        } else {
            warn!("   ⚠️ 本地订单簿不存在，Python将从API获取");
            None
        };
        
        info!("════════════════════════════════════════════════════════════");

        let url = format!("{}/order/market", self.config.order_service_url);
        let request = MarketOrderRequest {
            token_id: token_id.to_string(),
            side: "sell".to_string(),
            amount: tokens,
            price,  // 传递Rust计算的价格
            order_type: Some("FAK".to_string()),
        };

        let resp = self.http.post(&url)
            .json(&request)
            .send()
            .await
            .context("调用 Python 下单服务失败")?;

        let response: OrderResponse = resp.json().await?;

        if response.success {
            info!("   ✅ 市价卖出成功! order_id={:?}", response.order_id);
            info!("════════════════════════════════════════════════════════════");
            Ok(response.data.unwrap_or(json!({"success": true, "order_id": response.order_id})))
        } else {
            let error = response.error.unwrap_or_else(|| "Unknown error".to_string());
            error!("   ❌ 市价卖出失败: {}", error);
            info!("════════════════════════════════════════════════════════════");
            anyhow::bail!("市价卖出失败: {}", error)
        }
    }

    /// Limit buy - place a limit buy order at a specific price
    ///
    /// # Arguments
    /// * `token_id` - The token/asset ID to buy
    /// * `price` - Price per token (0.0 to 1.0)
    /// * `size` - Number of tokens to buy
    pub async fn limit_buy(&self, token_id: &str, price: f64, size: f64) -> Result<Value> {
        let token_short = &token_id[..20.min(token_id.len())];
        info!("════════════════════════════════════════════════════════════");
        info!("🎯 [限价买入] token={}..., price={:.4}, size={:.4}", token_short, price, size);
        info!("════════════════════════════════════════════════════════════");

        let url = format!("{}/order/limit", self.config.order_service_url);
        let request = LimitOrderRequest {
            token_id: token_id.to_string(),
            side: "buy".to_string(),
            price,
            size,
            order_type: Some("GTC".to_string()),
        };

        let resp = self.http.post(&url)
            .json(&request)
            .send()
            .await
            .context("调用 Python 下单服务失败")?;

        let response: OrderResponse = resp.json().await?;

        if response.success {
            info!("   ✅ 限价买入订单已提交! order_id={:?}", response.order_id);
            info!("════════════════════════════════════════════════════════════");
            Ok(response.data.unwrap_or(json!({"success": true, "order_id": response.order_id})))
        } else {
            let error = response.error.unwrap_or_else(|| "Unknown error".to_string());
            error!("   ❌ 限价买入失败: {}", error);
            info!("════════════════════════════════════════════════════════════");
            anyhow::bail!("限价买入失败: {}", error)
        }
    }

    /// Limit sell - place a limit sell order at a specific price
    ///
    /// # Arguments
    /// * `token_id` - The token/asset ID to sell
    /// * `price` - Price per token (0.0 to 1.0)
    /// * `size` - Number of tokens to sell
    pub async fn limit_sell(&self, token_id: &str, price: f64, size: f64) -> Result<Value> {
        let token_short = &token_id[..20.min(token_id.len())];
        info!("════════════════════════════════════════════════════════════");
        info!("🎯 [限价卖出] token={}..., price={:.4}, size={:.4}", token_short, price, size);
        info!("════════════════════════════════════════════════════════════");

        let url = format!("{}/order/limit", self.config.order_service_url);
        let request = LimitOrderRequest {
            token_id: token_id.to_string(),
            side: "sell".to_string(),
            price,
            size,
            order_type: Some("GTC".to_string()),
        };

        let resp = self.http.post(&url)
            .json(&request)
            .send()
            .await
            .context("调用 Python 下单服务失败")?;

        let response: OrderResponse = resp.json().await?;

        if response.success {
            info!("   ✅ 限价卖出订单已提交! order_id={:?}", response.order_id);
            info!("════════════════════════════════════════════════════════════");
            Ok(response.data.unwrap_or(json!({"success": true, "order_id": response.order_id})))
        } else {
            let error = response.error.unwrap_or_else(|| "Unknown error".to_string());
            error!("   ❌ 限价卖出失败: {}", error);
            info!("════════════════════════════════════════════════════════════");
            anyhow::bail!("限价卖出失败: {}", error)
        }
    }

    // ========================================================================
    // LEGACY ORDER PLACEMENT METHODS (kept for backward compatibility)
    // ========================================================================

    /// Place a market order by specifying tokens quantity (LEGACY)
    ///
    /// Note: Consider using `market_buy` or `market_sell` instead.
    pub async fn place_market_order_by_tokens(
        &self,
        token_id: &str,
        side: &str,
        tokens: f64,
    ) -> Result<Value> {
        // Route to the appropriate high-level method
        if side.to_lowercase() == "buy" {
            // For BUY with tokens, estimate USDC amount using local orderbook cache
            let orderbook = self.get_orderbook(token_id);
            let mut total_usdc = 0.0;
            let mut remaining = tokens;
            
            if let Some(book) = orderbook {
                // Estimate USDC needed (asks are sorted high to low, best ask at last)
                for (price, size) in book.asks.iter().rev() {
                    let fill = (*size).min(remaining);
                    total_usdc += fill * price;
                    remaining -= fill;
                    if remaining <= 0.0 {
                        break;
                    }
                }
            }
            
            // If no orderbook, estimate with 0.5 price
            if total_usdc == 0.0 {
                total_usdc = tokens * 0.5;
            }
            
            // 不要将滑点加到下单金额上！
            // 滑点应该只影响最高接受价格，由 market_buy 中的 price_with_slippage 处理
            // 下单金额保持为实际需要的 USDC 数量
            info!("   💰 预计花费: {:.4} USDC 买入 {:.2} tokens", total_usdc, tokens);
            self.market_buy(token_id, total_usdc).await
        } else {
            self.market_sell(token_id, tokens).await
        }
    }

    /// Place a market order (legacy method - uses USDC amount for BUY, tokens for SELL)
    ///
    /// Note: Consider using `market_buy` or `market_sell` instead.
    pub async fn place_market_order(
        &self,
        token_id: &str,
        side: &str,
        amount: f64,
    ) -> Result<Value> {
        // Route to the appropriate high-level method
        if side.to_lowercase() == "buy" {
            self.market_buy(token_id, amount).await
        } else {
            self.market_sell(token_id, amount).await
        }
    }

    /// Get open orders via Python service
    pub async fn get_open_orders(&self) -> Result<Value> {
        let url = format!("{}/orders", self.config.order_service_url);
        let resp = self.http.get(&url)
            .send()
            .await
            .context("调用 Python 下单服务失败")?;

        if !resp.status().is_success() {
            anyhow::bail!("获取订单失败: HTTP {}", resp.status());
        }

        let data: Value = resp.json().await?;
        if data.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
            Ok(data.get("orders").cloned().unwrap_or(json!([])))
        } else {
            let error = data.get("error").and_then(|v| v.as_str()).unwrap_or("Unknown error");
            anyhow::bail!("获取订单失败: {}", error)
        }
    }

    /// Get positions (placeholder - returns empty for now)
    pub async fn get_positions(&self) -> Result<Value> {
        // TODO: Implement via Python service if needed
        info!("⚠️ [Polymarket] get_positions 暂未实现，返回空持仓");
        Ok(json!([]))
    }

    /// Cancel an order via Python service
    pub async fn cancel_order(&self, order_id: &str) -> Result<Value> {
        let url = format!("{}/order/cancel", self.config.order_service_url);
        let request = CancelOrderRequest {
            order_id: order_id.to_string(),
        };

        let resp = self.http.post(&url)
            .json(&request)
            .send()
            .await
            .context("调用 Python 下单服务失败")?;

        let response: OrderResponse = resp.json().await?;

        if response.success {
            Ok(json!({"success": true, "order_id": order_id}))
        } else {
            let error = response.error.unwrap_or_else(|| "Unknown error".to_string());
            anyhow::bail!("取消订单失败: {}", error)
        }
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

