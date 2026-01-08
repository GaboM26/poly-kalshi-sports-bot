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

        // Calculate market price from order book
        let order_book = self.get_order_book(&order_args.token_id).await?;
        let price = match order_args.side {
            Side::Buy => builder.calculate_buy_market_price(
                &order_book,
                order_args.amount,
                OrderType::Fok,
            )?,
            Side::Sell => builder.calculate_sell_market_price(
                &order_book,
                order_args.amount,
                OrderType::Fok,
            )?,
        };

        builder
            .create_market_order(order_args, price, tick_size, neg_risk)
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

        let request_args = RequestArgs::new("POST", endpoints::POST_ORDER).with_body(&body_str);
        let headers = create_level_2_headers(signer, creds, &request_args);

        let url = format!("{}{}", self.host, endpoints::POST_ORDER);
        let response: Value = self.post_with_headers(&url, &headers, Some(&body_str)).await?;

        Ok(serde_json::from_value(response)?)
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
fn parse_orderbook_summary(raw: &Value) -> Result<OrderBookSummary> {
    let bids = raw["bids"]
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

    let asks = raw["asks"]
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

    Ok(OrderBookSummary { bids, asks })
}

/// Convert signed order to JSON for API submission
fn order_to_json(order: &SignedOrder, _owner: &str, order_type: OrderType) -> Value {
    json!({
        "order": {
            "salt": order.salt,
            "maker": order.maker,
            "signer": order.signer,
            "taker": order.taker,
            "tokenId": order.token_id,
            "makerAmount": order.maker_amount,
            "takerAmount": order.taker_amount,
            "expiration": order.expiration,
            "nonce": order.nonce,
            "feeRateBps": order.fee_rate_bps,
            "side": order.side,
            "signatureType": order.signature_type,
            "signature": order.signature
        },
        "orderType": order_type.to_string()
    })
}

impl Default for ClobClient {
    fn default() -> Self {
        Self::new(hosts::CLOB_HOST)
    }
}
