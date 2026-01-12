//! Polymarket CLOB Client
//!
//! Main client for interacting with the Polymarket CLOB API.
//! Supports three authentication levels:
//! - L0: Public endpoints only
//! - L1: EIP-712 signature authentication
//! - L2: HMAC signature authentication with API credentials

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use parking_lot::RwLock;
use reqwest::Client;
use serde_json::{json, Value};
use tracing::{debug, info};

use super::config::{endpoints, hosts, POLYGON_CHAIN_ID};
use super::headers::{create_level_1_headers, create_level_2_headers, to_header_map, RequestArgs};
use super::order_builder::OrderBuilder;
use super::signer::Signer;
use super::types::*;
use crate::utils;

/// Authentication level
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthLevel {
    /// No authentication, public endpoints only
    L0,
    /// EIP-712 signature authentication
    L1,
    /// HMAC signature authentication with API credentials
    L2,
}

/// CLOB Client for Polymarket
pub struct ClobClient {
    /// HTTP client
    http: Client,
    /// CLOB API host
    host: String,
    /// Chain ID
    chain_id: u64,
    /// Signer for L1/L2 auth
    signer: Option<Signer>,
    /// API credentials for L2 auth
    creds: Option<ApiCreds>,
    /// Order builder
    builder: Option<OrderBuilder>,
    /// Signature type (for balance API)
    sig_type: SignatureType,
    /// Current authentication level
    auth_level: AuthLevel,
    /// Cached tick sizes
    tick_sizes: Arc<RwLock<HashMap<String, f64>>>,
    /// Cached neg_risk flags
    neg_risk: Arc<RwLock<HashMap<String, bool>>>,
}

impl ClobClient {
    /// Create a new CLOB client (L0 - public endpoints only)
    pub fn new(host: &str) -> Self {
        let host = host.trim_end_matches('/').to_string();
        Self {
            http: Client::new(),
            host,
            chain_id: POLYGON_CHAIN_ID,
            signer: None,
            creds: None,
            builder: None,
            sig_type: SignatureType::Eoa,
            auth_level: AuthLevel::L0,
            tick_sizes: Arc::new(RwLock::new(HashMap::new())),
            neg_risk: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new CLOB client with L1 authentication
    pub fn with_l1_auth(
        host: &str,
        chain_id: u64,
        private_key: &str,
        signature_type: Option<SignatureType>,
        funder: Option<&str>,
    ) -> Result<Self> {
        let host = host.trim_end_matches('/').to_string();
        let signer = Signer::new(private_key, chain_id)?;

        let funder_addr = if let Some(f) = funder {
            Some(f.parse().with_context(|| "Invalid funder address")?)
        } else {
            None
        };

        let sig_type = signature_type.unwrap_or(SignatureType::Eoa);
        let builder = OrderBuilder::new(signer.clone(), Some(sig_type), funder_addr);

        Ok(Self {
            http: Client::new(),
            host,
            chain_id,
            signer: Some(signer),
            creds: None,
            builder: Some(builder),
            sig_type,
            auth_level: AuthLevel::L1,
            tick_sizes: Arc::new(RwLock::new(HashMap::new())),
            neg_risk: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Create a new CLOB client with L2 authentication
    pub fn with_l2_auth(
        host: &str,
        chain_id: u64,
        private_key: &str,
        creds: ApiCreds,
        signature_type: Option<SignatureType>,
        funder: Option<&str>,
    ) -> Result<Self> {
        let mut client = Self::with_l1_auth(host, chain_id, private_key, signature_type, funder)?;
        client.creds = Some(creds);
        client.auth_level = AuthLevel::L2;
        Ok(client)
    }

    /// Set API credentials and upgrade to L2 auth
    pub fn set_api_creds(&mut self, creds: ApiCreds) {
        self.creds = Some(creds);
        if self.signer.is_some() {
            self.auth_level = AuthLevel::L2;
        }
    }

    /// Get signer address
    pub fn get_address(&self) -> Option<String> {
        self.signer.as_ref().map(|s| format!("{:?}", s.address()))
    }

    // === L0 Public Endpoints ===

    /// Health check
    pub async fn get_ok(&self) -> Result<Value> {
        self.get(&format!("{}/", self.host)).await
    }

    /// Get server time
    pub async fn get_server_time(&self) -> Result<Value> {
        self.get(&format!("{}{}", self.host, endpoints::GET_SERVER_TIME))
            .await
    }

    /// Get order book for a token
    pub async fn get_order_book(&self, token_id: &str) -> Result<OrderBookSummary> {
        let url = format!(
            "{}{}?token_id={}",
            self.host,
            endpoints::GET_ORDER_BOOK,
            token_id
        );
        let raw: Value = self.get(&url).await?;
        parse_orderbook_summary(&raw)
    }

    /// Get mid price for a token
    pub async fn get_midpoint(&self, token_id: &str) -> Result<Value> {
        let url = format!(
            "{}{}?token_id={}",
            self.host,
            endpoints::GET_MID_POINT,
            token_id
        );
        self.get(&url).await
    }

    /// Get price for a token and side
    pub async fn get_price(&self, token_id: &str, side: &str) -> Result<Value> {
        let url = format!(
            "{}{}?token_id={}&side={}",
            self.host,
            endpoints::GET_PRICE,
            token_id,
            side
        );
        self.get(&url).await
    }

    /// Get tick size for a token
    pub async fn get_tick_size(&self, token_id: &str) -> Result<f64> {
        // Check cache
        {
            let cache = self.tick_sizes.read();
            if let Some(&ts) = cache.get(token_id) {
                return Ok(ts);
            }
        }

        let url = format!(
            "{}{}?token_id={}",
            self.host,
            endpoints::GET_TICK_SIZE,
            token_id
        );
        let result: Value = self.get(&url).await?;
        let tick_size: f64 = result["minimum_tick_size"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.01);

        // Cache it
        self.tick_sizes.write().insert(token_id.to_string(), tick_size);

        Ok(tick_size)
    }

    /// Validate a token_id by checking if it exists and has valid market data
    /// Returns detailed validation result for debugging
    pub async fn validate_token_id(&self, token_id: &str) -> TokenValidationResult {
        let token_short = &token_id[..16.min(token_id.len())];
        info!("🔍 [Token验证] 开始验证 token_id: {}...", token_short);
        
        // 1. Try to get orderbook
        let orderbook_result = match self.get_order_book(token_id).await {
            Ok(ob) => {
                let has_bids = !ob.bids.is_empty();
                let has_asks = !ob.asks.is_empty();
                // API returns: bids[last] = best_bid (highest), asks[last] = best_ask (lowest)
                let best_bid = ob.bids.last().map(|(p, _)| *p);
                let best_ask = ob.asks.last().map(|(p, _)| *p);
                info!(
                    "✅ [Token验证] Orderbook有效: has_bids={}, has_asks={}, best_bid={:?}, best_ask={:?}",
                    has_bids, has_asks, best_bid, best_ask
                );
                Some(OrderbookValidation {
                    valid: true,
                    has_bids,
                    has_asks,
                    best_bid,
                    best_ask,
                    error: None,
                })
            }
            Err(e) => {
                let error_msg = format!("{}", e);
                tracing::error!("❌ [Token验证] Orderbook获取失败: {}", error_msg);
                Some(OrderbookValidation {
                    valid: false,
                    has_bids: false,
                    has_asks: false,
                    best_bid: None,
                    best_ask: None,
                    error: Some(error_msg),
                })
            }
        };
        
        // 2. Try to get tick size (this also validates token exists)
        let tick_size_result = match self.get_tick_size(token_id).await {
            Ok(ts) => {
                info!("✅ [Token验证] Tick size有效: {}", ts);
                Some(ts)
            }
            Err(e) => {
                tracing::error!("❌ [Token验证] Tick size获取失败: {}", e);
                None
            }
        };
        
        // 3. Try to get midpoint price
        let midpoint_result = match self.get_midpoint(token_id).await {
            Ok(mid) => {
                let mid_price = mid.get("mid").and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok());
                info!("✅ [Token验证] Midpoint有效: {:?}", mid_price);
                mid_price
            }
            Err(e) => {
                tracing::error!("❌ [Token验证] Midpoint获取失败: {}", e);
                None
            }
        };
        
        let is_valid = orderbook_result.as_ref().map(|o| o.valid).unwrap_or(false) 
            && tick_size_result.is_some();
        
        let result = TokenValidationResult {
            token_id: token_id.to_string(),
            is_valid,
            orderbook: orderbook_result,
            tick_size: tick_size_result,
            midpoint: midpoint_result,
        };
        
        // Write to debug log
        utils::write_debug_log(
            "client.rs:validate_token_id",
            "Token验证结果",
            serde_json::json!({
                "token_id": token_id,
                "is_valid": result.is_valid,
                "has_orderbook": result.orderbook.as_ref().map(|o| o.valid),
                "has_tick_size": result.tick_size.is_some(),
                "midpoint": result.midpoint,
                "orderbook_error": result.orderbook.as_ref().and_then(|o| o.error.clone()),
            })
        );
        
        if result.is_valid {
            info!("✅ [Token验证] Token有效: {}...", token_short);
        } else {
            tracing::error!("❌ [Token验证] Token无效: {}...", token_short);
        }
        
        result
    }

    /// Get neg_risk flag for a token
    pub async fn get_neg_risk(&self, token_id: &str) -> Result<bool> {
        // Check cache
        {
            let cache = self.neg_risk.read();
            if let Some(&nr) = cache.get(token_id) {
                return Ok(nr);
            }
        }

        let url = format!(
            "{}{}?token_id={}",
            self.host,
            endpoints::GET_NEG_RISK,
            token_id
        );
        let result: Value = self.get(&url).await?;
        let neg_risk = result["neg_risk"].as_bool().unwrap_or(false);

        // Cache it
        self.neg_risk.write().insert(token_id.to_string(), neg_risk);

        Ok(neg_risk)
    }

    // === L1 Authenticated Endpoints ===

    fn assert_l1_auth(&self) -> Result<()> {
        if self.signer.is_none() {
            bail!("L1 authentication required. Please provide a private key.");
        }
        Ok(())
    }

    /// Create a new API key
    pub async fn create_api_key(&self, nonce: Option<u64>) -> Result<ApiCreds> {
        self.assert_l1_auth()?;
        let signer = self.signer.as_ref().unwrap();

        let headers = create_level_1_headers(signer, nonce).await?;
        let url = format!("{}{}", self.host, endpoints::CREATE_API_KEY);

        let response: Value = self.post_with_headers(&url, &headers, None).await?;

        Ok(ApiCreds {
            api_key: response["apiKey"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing apiKey"))?
                .to_string(),
            api_secret: response["secret"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing secret"))?
                .to_string(),
            api_passphrase: response["passphrase"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing passphrase"))?
                .to_string(),
        })
    }

    /// Derive an existing API key
    pub async fn derive_api_key(&self, nonce: Option<u64>) -> Result<ApiCreds> {
        self.assert_l1_auth()?;
        let signer = self.signer.as_ref().unwrap();

        let headers = create_level_1_headers(signer, nonce).await?;
        let url = format!("{}{}", self.host, endpoints::DERIVE_API_KEY);

        let response: Value = self.get_with_headers(&url, &headers).await?;

        Ok(ApiCreds {
            api_key: response["apiKey"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing apiKey"))?
                .to_string(),
            api_secret: response["secret"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing secret"))?
                .to_string(),
            api_passphrase: response["passphrase"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing passphrase"))?
                .to_string(),
        })
    }

    /// Create or derive API credentials
    pub async fn create_or_derive_api_creds(&self, nonce: Option<u64>) -> Result<ApiCreds> {
        match self.create_api_key(nonce).await {
            Ok(creds) => Ok(creds),
            Err(_) => self.derive_api_key(nonce).await,
        }
    }

    /// Create an order
    pub async fn create_order(&self, order_args: &OrderArgs) -> Result<SignedOrder> {
        self.assert_l1_auth()?;
        let builder = self
            .builder
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Order builder not initialized"))?;

        let tick_size = self.get_tick_size(&order_args.token_id).await?;
        let neg_risk = self.get_neg_risk(&order_args.token_id).await?;

        builder.create_order(order_args, tick_size, neg_risk).await
    }

    /// Create a market order
    pub async fn create_market_order(&self, order_args: &MarketOrderArgs) -> Result<SignedOrder> {
        self.assert_l1_auth()?;
        let builder = self
            .builder
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Order builder not initialized"))?;

        let tick_size = self.get_tick_size(&order_args.token_id).await?;
        let neg_risk = self.get_neg_risk(&order_args.token_id).await?;

        // Log market order parameters for debugging
        info!(
            "📋 [Poly市价单] 开始创建: token_id={}, side={:?}, amount={:.4}, tick_size={}, neg_risk={}",
            order_args.token_id, order_args.side, order_args.amount, tick_size, neg_risk
        );

        // Calculate market price from order book
        // Use Fak (Fill and Kill) mode - fill as much as possible, cancel the rest
        let order_book = self.get_order_book(&order_args.token_id).await?;
        
        // Log orderbook state for debugging
        // API returns: bids[last] = best_bid (highest), asks[last] = best_ask (lowest)
        let best_bid = order_book.bids.last().map(|(p, s)| format!("{}@{}", s, p));
        let best_ask = order_book.asks.last().map(|(p, s)| format!("{}@{}", s, p));
        debug!(
            "📊 [Poly订单簿] best_bid={:?}, best_ask={:?}, bids_levels={}, asks_levels={}",
            best_bid, best_ask, order_book.bids.len(), order_book.asks.len()
        );
        
        let price = match order_args.side {
            Side::Buy => builder.calculate_buy_market_price(
                &order_book,
                order_args.amount,
                OrderType::Fak,
            )?,
            Side::Sell => builder.calculate_sell_market_price(
                &order_book,
                order_args.amount,
                OrderType::Fak,
            )?,
        };

        info!(
            "📋 [Poly市价单] 计算价格: price={:.4}, side={:?}",
            price, order_args.side
        );

        builder
            .create_market_order(order_args, price, tick_size, neg_risk)
            .await
    }

    /// Create a market order by specifying tokens quantity (recommended method)
    /// 
    /// This method:
    /// 1. Traverses the order book from best price
    /// 2. Calculates the total USDC needed to buy `tokens` quantity
    /// 3. Adds slippage protection (default 5%)
    /// 4. Creates order with correct maker/taker semantics
    /// 
    /// This behaves like Kalshi's IOC - fills from best price level up until target is met
    pub async fn create_market_order_by_tokens(
        &self,
        token_id: &str,
        side: Side,
        tokens: f64,
        slippage: Option<f64>,  // Default 5%
    ) -> Result<SignedOrder> {
        self.assert_l1_auth()?;
        let builder = self
            .builder
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Order builder not initialized"))?;

        let tick_size = self.get_tick_size(token_id).await?;
        let neg_risk = self.get_neg_risk(token_id).await?;
        let slippage = slippage.unwrap_or(0.05); // Default 5% slippage

        info!(
            "📋 [Poly市价单-Tokens模式] 开始创建: token_id={}, side={:?}, tokens={:.4}, slippage={:.1}%, tick_size={}, neg_risk={}",
            token_id, side, tokens, slippage * 100.0, tick_size, neg_risk
        );

        // Get order book
        let order_book = self.get_order_book(token_id).await?;
        
        // Log orderbook state
        // API returns: bids[last] = best_bid (highest), asks[last] = best_ask (lowest)
        let best_bid = order_book.bids.last().map(|(p, s)| format!("{:.4}@{:.4}", s, p));
        let best_ask = order_book.asks.last().map(|(p, s)| format!("{:.4}@{:.4}", s, p));
        info!(
            "📊 [Poly订单簿] best_bid={:?}, best_ask={:?}, bids_levels={}, asks_levels={}",
            best_bid, best_ask, order_book.bids.len(), order_book.asks.len()
        );

        // Calculate cost based on side
        let usdc_with_slippage = match side {
            Side::Buy => {
                // Calculate USDC needed to buy `tokens` from order book (with slippage)
                let (usdc, worst_price, available_tokens) = builder.calculate_buy_cost_for_tokens(
                    &order_book,
                    tokens,
                    slippage,
                )?;
                
                if available_tokens < tokens {
                    info!(
                        "⚠️ [流动性不足] 目标tokens={:.4}, 可用tokens={:.4}, 将部分成交",
                        tokens, available_tokens
                    );
                }
                
                info!(
                    "📋 [BUY计算完成] tokens={:.4}, usdc_with_slippage={:.4}, worst_price={:.4}",
                    tokens, usdc, worst_price
                );
                usdc
            }
            Side::Sell => {
                // For SELL, calculate USDC you'll receive (with slippage as min acceptable)
                // API returns bids: [low...high], use .rev() to start from best_bid (last)
                let mut total_usdc = 0.0;
                let mut total_tokens = 0.0;
                
                for (price, size) in order_book.bids.iter().rev() {
                    let tokens_at_level = (*size).min(tokens - total_tokens);
                    total_tokens += tokens_at_level;
                    total_usdc += tokens_at_level * price;
                    
                    if total_tokens >= tokens {
                        break;
                    }
                }
                
                // Apply slippage (reduce expected USDC for sell)
                let usdc_with_slippage = total_usdc * (1.0 - slippage);
                
                info!(
                    "📋 [SELL计算完成] tokens={:.4}, usdc_with_slippage={:.4}",
                    tokens, usdc_with_slippage
                );
                usdc_with_slippage
            }
        };

        // Create order with correct maker/taker semantics
        builder
            .create_market_order_by_tokens(
                token_id,
                side,
                tokens,
                usdc_with_slippage,
                tick_size,
                neg_risk,
                None, // fee_rate_bps
                None, // nonce
            )
            .await
    }

    // === L2 Authenticated Endpoints ===

    fn assert_l2_auth(&self) -> Result<()> {
        self.assert_l1_auth()?;
        if self.creds.is_none() {
            bail!("L2 authentication required. Please provide API credentials.");
        }
        Ok(())
    }

    /// Post an order
    pub async fn post_order(
        &self,
        order: &SignedOrder,
        order_type: OrderType,
    ) -> Result<OrderResponse> {
        self.assert_l2_auth()?;
        let signer = self.signer.as_ref().unwrap();
        let creds = self.creds.as_ref().unwrap();

        let body = order_to_json(order, &creds.api_key, order_type);
        let body_str = serde_json::to_string(&body)?;

        // Log the full order payload for debugging - DETAILED
        info!(
            "📤 [Poly提交订单] token_id={}, side={}, maker_amt={}, taker_amt={}, order_type={:?}",
            order.token_id,
            if order.side == "0" { "BUY" } else { "SELL" },
            order.maker_amount,
            order.taker_amount,
            order_type
        );
        
        // 🔍 详细日志：输出完整的签名订单信息
        info!(
            "🔍 [Poly订单详细] salt={}, maker={}, signer={}, taker={}, expiration={}, nonce={}, fee_rate_bps={}, signature_type={}",
            order.salt, order.maker, order.signer, order.taker, 
            order.expiration, order.nonce, order.fee_rate_bps, order.signature_type
        );
        info!("🔍 [Poly订单签名] signature={}", order.signature);
        
        // 🔍 输出完整请求体（用于调试）
        info!("🔍 [Poly完整请求体] {}", body_str);

        // #region agent log - 写入调试日志文件 (假设 A, B, C)
        {
            use std::io::Write;
            let debug_log = serde_json::json!({
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "location": "client.rs:post_order",
                "hypothesisId": "A,B,C",
                "message": "Poly订单提交",
                "data": {
                    "token_id": &order.token_id,
                    "side": &order.side,
                    "maker_amount": &order.maker_amount,
                    "taker_amount": &order.taker_amount,
                    "salt": &order.salt,
                    "maker": &order.maker,
                    "signer": &order.signer,
                    "signature_type": &order.signature_type,
                    "order_type": format!("{:?}", order_type),
                    "full_body": &body_str
                }
            });
            let path = utils::get_debug_log_path();
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                let _ = writeln!(f, "{}", debug_log.to_string());
            }
        }
        // #endregion

        let request_args = RequestArgs::new("POST", endpoints::POST_ORDER).with_body(&body_str);
        let headers = create_level_2_headers(signer, creds, &request_args);
        
        // 🔍 输出请求头（隐藏敏感信息）
        info!("🔍 [Poly请求头] POLY_ADDRESS={}, POLY_TIMESTAMP存在={}", 
            headers.get("POLY_ADDRESS").unwrap_or(&"N/A".to_string()),
            headers.contains_key("POLY_TIMESTAMP")
        );

        let url = format!("{}{}", self.host, endpoints::POST_ORDER);
        
        // Try to post order and catch specific errors for debugging
        let response_result = self.post_with_headers(&url, &headers, Some(&body_str)).await;
        
        match response_result {
            Ok(response) => {
                // Log response for debugging
                info!(
                    "📥 [Poly订单响应] success={}, order_id={}, status={}",
                    response.get("success").and_then(|v| v.as_bool()).unwrap_or(false),
                    response.get("orderID").and_then(|v| v.as_str()).unwrap_or("N/A"),
                    response.get("status").and_then(|v| v.as_str()).unwrap_or("N/A")
                );
                
                // 🔍 输出完整响应（用于调试）
                info!("🔍 [Poly完整响应] {}", serde_json::to_string(&response).unwrap_or_default());
                
                if let Some(error_msg) = response.get("errorMsg").and_then(|v| v.as_str()) {
                    if !error_msg.is_empty() {
                        info!("📥 [Poly订单错误] errorMsg={}", error_msg);
                    }
                }

                Ok(serde_json::from_value(response)?)
            }
            Err(e) => {
                let error_str = format!("{}", e);
                
                // Check if this is an "Invalid order payload" error - validate token_id
                if error_str.contains("Invalid order payload") || error_str.contains("market not found") {
                    tracing::error!(
                        "❌ [Poly下单失败] 检测到无效订单错误，开始验证token_id: {}...",
                        &order.token_id[..16.min(order.token_id.len())]
                    );
                    
                    // Validate the token_id via API
                    let validation = self.validate_token_id(&order.token_id).await;
                    
                    // Log validation result
                    if validation.is_valid {
                        tracing::error!(
                            "⚠️ [Token验证结果] Token有效但下单失败，可能是其他问题 (签名/金额/时间戳等)"
                        );
                    } else {
                        tracing::error!(
                            "❌ [Token验证结果] Token无效! orderbook={:?}, tick_size={:?}, midpoint={:?}",
                            validation.orderbook.as_ref().map(|o| o.valid),
                            validation.tick_size,
                            validation.midpoint
                        );
                    }
                    
                    // Write detailed validation to debug log
                    {
                        use std::io::Write;
                        let debug_log = serde_json::json!({
                            "timestamp": chrono::Utc::now().to_rfc3339(),
                            "location": "client.rs:post_order:error_validation",
                            "message": "下单失败后Token验证",
                            "data": {
                                "error": &error_str,
                                "token_id": &order.token_id,
                                "validation_result": validation,
                                "order_side": &order.side,
                                "maker_amount": &order.maker_amount,
                                "taker_amount": &order.taker_amount,
                            }
                        });
                        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/Users/meloner/rustcode/polytaoli/.cursor/debug.log") {
                            let _ = writeln!(f, "{}", debug_log.to_string());
                        }
                    }
                }
                
                // Re-return the original error
                Err(e)
            }
        }
    }

    /// Cancel an order
    pub async fn cancel(&self, order_id: &str) -> Result<Value> {
        self.assert_l2_auth()?;
        let signer = self.signer.as_ref().unwrap();
        let creds = self.creds.as_ref().unwrap();

        let body = json!({ "orderID": order_id });
        let body_str = serde_json::to_string(&body)?;

        let request_args =
            RequestArgs::new("DELETE", endpoints::CANCEL_ORDER).with_body(&body_str);
        let headers = create_level_2_headers(signer, creds, &request_args);

        let url = format!("{}{}", self.host, endpoints::CANCEL_ORDER);
        self.delete_with_headers(&url, &headers, Some(&body_str))
            .await
    }

    /// Cancel all orders
    pub async fn cancel_all(&self) -> Result<Value> {
        self.assert_l2_auth()?;
        let signer = self.signer.as_ref().unwrap();
        let creds = self.creds.as_ref().unwrap();

        let request_args = RequestArgs::new("DELETE", endpoints::CANCEL_ALL);
        let headers = create_level_2_headers(signer, creds, &request_args);

        let url = format!("{}{}", self.host, endpoints::CANCEL_ALL);
        self.delete_with_headers(&url, &headers, None).await
    }

    /// Get open orders
    pub async fn get_orders(&self) -> Result<Vec<OpenOrder>> {
        self.assert_l2_auth()?;
        let signer = self.signer.as_ref().unwrap();
        let creds = self.creds.as_ref().unwrap();

        let request_args = RequestArgs::new("GET", endpoints::GET_ORDERS);
        let headers = create_level_2_headers(signer, creds, &request_args);

        let url = format!("{}{}", self.host, endpoints::GET_ORDERS);
        let response: Value = self.get_with_headers(&url, &headers).await?;

        let data = response["data"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid response format"))?;

        data.iter()
            .map(|v| serde_json::from_value(v.clone()).map_err(Into::into))
            .collect()
    }

    /// Get trades
    pub async fn get_trades(&self) -> Result<Vec<Trade>> {
        self.assert_l2_auth()?;
        let signer = self.signer.as_ref().unwrap();
        let creds = self.creds.as_ref().unwrap();

        let request_args = RequestArgs::new("GET", endpoints::GET_TRADES);
        let headers = create_level_2_headers(signer, creds, &request_args);

        let url = format!("{}{}", self.host, endpoints::GET_TRADES);
        let response: Value = self.get_with_headers(&url, &headers).await?;

        let data = response["data"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid response format"))?;

        data.iter()
            .map(|v| serde_json::from_value(v.clone()).map_err(Into::into))
            .collect()
    }

    /// Get balance and allowance
    pub async fn get_balance_allowance(&self) -> Result<BalanceAllowance> {
        self.assert_l2_auth()?;
        let signer = self.signer.as_ref().unwrap();
        let creds = self.creds.as_ref().unwrap();

        let request_args = RequestArgs::new("GET", endpoints::GET_BALANCE_ALLOWANCE);
        let headers = create_level_2_headers(signer, creds, &request_args);

        // Add query parameters (required by Polymarket API, matching Python implementation)
        let sig_type_num: u8 = match self.sig_type {
            SignatureType::Eoa => 0,
            SignatureType::PolyProxy => 1,
            SignatureType::PolyGnosisSafe => 2,
        };
        let url = format!(
            "{}{}?asset_type=COLLATERAL&signature_type={}",
            self.host, endpoints::GET_BALANCE_ALLOWANCE, sig_type_num
        );
        
        let response: Value = self.get_with_headers(&url, &headers).await?;

        Ok(serde_json::from_value(response)?)
    }

    // === HTTP Helpers ===

    async fn get(&self, url: &str) -> Result<Value> {
        debug!("GET {}", url);
        let response = self.http.get(url).send().await?;
        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() {
            bail!("HTTP {} - {}", status, body);
        }

        serde_json::from_str(&body).with_context(|| format!("Failed to parse response: {}", body))
    }

    async fn get_with_headers(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
    ) -> Result<Value> {
        debug!("GET {} with auth", url);
        let header_map = to_header_map(headers)?;
        let response = self.http.get(url).headers(header_map).send().await?;
        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() {
            bail!("HTTP {} - {}", status, body);
        }

        serde_json::from_str(&body).with_context(|| format!("Failed to parse response: {}", body))
    }

    async fn post_with_headers(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        body: Option<&str>,
    ) -> Result<Value> {
        debug!("POST {} with auth", url);
        let header_map = to_header_map(headers)?;
        let mut request = self
            .http
            .post(url)
            .headers(header_map)
            .header("Content-Type", "application/json");

        if let Some(body_str) = body {
            request = request.body(body_str.to_string());
        }

        let response = request.send().await?;
        let status = response.status();
        let response_body = response.text().await?;

        if !status.is_success() {
            // 🔍 详细错误日志
            tracing::error!(
                "❌ [Poly HTTP错误] status={}, url={}, response_body={}",
                status, url, response_body
            );
            
            // #region agent log - 写入失败日志 (假设 A, B, C, D, E)
            {
                use std::io::Write;
                let debug_log = serde_json::json!({
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                    "location": "client.rs:post_with_headers:error",
                    "hypothesisId": "A,B,C,D,E",
                    "message": "HTTP请求失败",
                    "data": {
                        "status": status.to_string(),
                        "url": url,
                        "response_body": &response_body,
                        "request_body": body.map(|b| b.to_string())
                    }
                });
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/Users/meloner/rustcode/polytaoli/.cursor/debug.log") {
                    let _ = writeln!(f, "{}", debug_log.to_string());
                }
            }
            // #endregion
            
            bail!("HTTP {} - {}", status, response_body);
        }

        serde_json::from_str(&response_body)
            .with_context(|| format!("Failed to parse response: {}", response_body))
    }

    async fn delete_with_headers(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        body: Option<&str>,
    ) -> Result<Value> {
        debug!("DELETE {} with auth", url);
        let header_map = to_header_map(headers)?;
        let mut request = self
            .http
            .delete(url)
            .headers(header_map)
            .header("Content-Type", "application/json");

        if let Some(body_str) = body {
            request = request.body(body_str.to_string());
        }

        let response = request.send().await?;
        let status = response.status();
        let response_body = response.text().await?;

        if !status.is_success() {
            bail!("HTTP {} - {}", status, response_body);
        }

        serde_json::from_str(&response_body)
            .with_context(|| format!("Failed to parse response: {}", response_body))
    }
}

/// Parse orderbook summary from raw response
/// 
/// Polymarket API returns:
/// - bids: ascending order (low to high), best_bid = LAST (highest buy price)
/// - asks: descending order (high to low), best_ask = LAST (lowest sell price)
/// 
/// We keep the original order from API and use .last() to get best prices
fn parse_orderbook_summary(raw: &Value) -> Result<OrderBookSummary> {
    let bids: Vec<(f64, f64)> = raw["bids"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| {
                    let price = entry["price"].as_str()?.parse().ok()?;
                    let size = entry["size"].as_str()?.parse().ok()?;
                    Some((price, size))
                })
                .collect()
        })
        .unwrap_or_default();

    let asks: Vec<(f64, f64)> = raw["asks"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| {
                    let price = entry["price"].as_str()?.parse().ok()?;
                    let size = entry["size"].as_str()?.parse().ok()?;
                    Some((price, size))
                })
                .collect()
        })
        .unwrap_or_default();

    // DO NOT SORT - keep API's original order:
    // bids: [low...high], best_bid at last
    // asks: [high...low], best_ask at last

    Ok(OrderBookSummary { bids, asks })
}

/// Convert signed order to JSON for API submission
fn order_to_json(order: &SignedOrder, owner: &str, order_type: OrderType) -> Value {
    // Convert side from "0"/"1" to "BUY"/"SELL" (matching Python py-clob-client behavior)
    let side_str = if order.side == "0" { "BUY" } else { "SELL" };
    
    // Parse salt as number (Python py-clob-client sends salt as number, not string)
    let salt_num: u64 = order.salt.parse().unwrap_or(0);
    
    // Build the JSON with correct types (matching Python py-clob-client):
    // - salt: number (NOT string)
    // - maker, signer, taker: string (addresses)
    // - tokenId, makerAmount, takerAmount, expiration, nonce, feeRateBps: string
    // - side: "BUY" or "SELL"
    // - signatureType: number (NOT string)
    // - signature: string
    let signature_type_num: u8 = order.signature_type.parse().unwrap_or(0);
    
    json!({
        "order": {
            "salt": salt_num,  // NUMBER, not string
            "maker": &order.maker,
            "signer": &order.signer,
            "taker": &order.taker,
            "tokenId": &order.token_id,
            "makerAmount": &order.maker_amount,
            "takerAmount": &order.taker_amount,
            "expiration": &order.expiration,
            "nonce": &order.nonce,
            "feeRateBps": &order.fee_rate_bps,
            "side": side_str,
            "signatureType": signature_type_num,  // NUMBER, not string
            "signature": &order.signature
        },
        "owner": owner,
        "orderType": order_type.to_string()
    })
}

impl Default for ClobClient {
    fn default() -> Self {
        Self::new(hosts::CLOB_HOST)
    }
}
