//! CLOB types and data structures

use serde::{Deserialize, Serialize};

/// API credentials for Polymarket CLOB
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiCreds {
    pub api_key: String,
    pub api_secret: String,
    pub api_passphrase: String,
}

/// Order side
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Side {
    Buy,
    Sell,
}

impl std::fmt::Display for Side {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Side::Buy => write!(f, "BUY"),
            Side::Sell => write!(f, "SELL"),
        }
    }
}

/// Order type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum OrderType {
    /// Good Till Cancelled
    Gtc,
    /// Fill Or Kill - must fill completely or cancel entirely
    Fok,
    /// Fill And Kill - fill as much as possible, cancel the rest immediately
    Fak,
    /// Good Till Date
    Gtd,
}

impl std::fmt::Display for OrderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrderType::Gtc => write!(f, "GTC"),
            OrderType::Fok => write!(f, "FOK"),
            OrderType::Fak => write!(f, "FAK"),
            OrderType::Gtd => write!(f, "GTD"),
        }
    }
}

/// Signature type for orders
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignatureType {
    /// EOA signature (type 0)
    Eoa = 0,
    /// Poly Proxy signature (type 1) - Magic Link users
    PolyProxy = 1,
    /// Poly Gnosis Safe signature (type 2)
    PolyGnosisSafe = 2,
}

impl Default for SignatureType {
    fn default() -> Self {
        Self::PolyProxy
    }
}

/// Order arguments for creating an order
#[derive(Debug, Clone)]
pub struct OrderArgs {
    /// Token ID to trade
    pub token_id: String,
    /// Price (0.0 to 1.0)
    pub price: f64,
    /// Size in shares
    pub size: f64,
    /// Buy or Sell
    pub side: Side,
    /// Fee rate BPS (default 0)
    pub fee_rate_bps: Option<i64>,
    /// Nonce (auto-generated if None)
    pub nonce: Option<u64>,
    /// Expiration timestamp (0 for no expiration)
    pub expiration: Option<u64>,
    /// Order type
    pub order_type: Option<OrderType>,
}

/// Market order arguments
#[derive(Debug, Clone)]
pub struct MarketOrderArgs {
    /// Token ID to trade
    pub token_id: String,
    /// Amount to spend (for buy) or receive (for sell) in USD
    pub amount: f64,
    /// Buy or Sell
    pub side: Side,
    /// Fee rate BPS (default 0)
    pub fee_rate_bps: Option<i64>,
    /// Nonce (auto-generated if None)
    pub nonce: Option<u64>,
    /// Slippage tolerance (default 0.5%)
    pub slippage: Option<f64>,
}

/// Signed order ready for submission
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedOrder {
    /// Order salt/nonce
    pub salt: String,
    /// Maker address (funder)
    pub maker: String,
    /// Signer address
    pub signer: String,
    /// Taker address (usually 0x0)
    pub taker: String,
    /// Token ID
    pub token_id: String,
    /// Maker amount (in token decimals)
    pub maker_amount: String,
    /// Taker amount (in token decimals)
    pub taker_amount: String,
    /// Expiration timestamp
    pub expiration: String,
    /// Nonce
    pub nonce: String,
    /// Fee rate BPS
    pub fee_rate_bps: String,
    /// Side (0 = buy, 1 = sell)
    pub side: String,
    /// Signature type
    pub signature_type: String,
    /// Order signature
    pub signature: String,
}

/// Order book entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBookEntry {
    pub price: String,
    pub size: String,
}

/// Order book response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBook {
    pub market: String,
    pub asset_id: String,
    pub bids: Vec<OrderBookEntry>,
    pub asks: Vec<OrderBookEntry>,
    pub hash: String,
    pub timestamp: String,
}

/// Order book summary (parsed prices)
#[derive(Debug, Clone)]
pub struct OrderBookSummary {
    pub bids: Vec<(f64, f64)>, // (price, size)
    pub asks: Vec<(f64, f64)>, // (price, size)
}

impl OrderBookSummary {
    /// Get best bid price
    pub fn best_bid(&self) -> Option<f64> {
        self.bids.first().map(|(p, _)| *p)
    }

    /// Get best ask price
    pub fn best_ask(&self) -> Option<f64> {
        self.asks.first().map(|(p, _)| *p)
    }

    /// Get mid price
    pub fn mid_price(&self) -> Option<f64> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => Some((bid + ask) / 2.0),
            _ => None,
        }
    }

    /// Get spread
    pub fn spread(&self) -> Option<f64> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => Some(ask - bid),
            _ => None,
        }
    }
}

/// Order response from API
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_msg: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

/// Balance response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BalanceAllowance {
    pub balance: String,
    #[serde(default)]
    pub allowances: std::collections::HashMap<String, String>,
}

/// Trade parameters for RFQ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeParams {
    pub amount_in: String,
    pub amount_out_min: String,
    pub token_in: String,
    pub token_out: String,
}

/// Open order info
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenOrder {
    pub id: String,
    pub status: String,
    pub market: String,
    pub asset_id: String,
    pub side: String,
    pub original_size: String,
    pub size_matched: String,
    pub price: String,
    pub outcome: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiration: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub associate_trades: Option<Vec<serde_json::Value>>,
}

/// Trade info
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Trade {
    pub id: String,
    pub market: String,
    pub asset_id: String,
    pub side: String,
    pub size: String,
    pub price: String,
    pub fee_rate_bps: String,
    pub status: String,
    pub match_time: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maker_address: Option<String>,
}

/// Result of token validation check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenValidationResult {
    /// The token ID that was validated
    pub token_id: String,
    /// Whether the token is valid
    pub is_valid: bool,
    /// Orderbook validation result
    pub orderbook: Option<OrderbookValidation>,
    /// Tick size if available
    pub tick_size: Option<f64>,
    /// Midpoint price if available
    pub midpoint: Option<f64>,
}

/// Orderbook validation details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderbookValidation {
    /// Whether orderbook request succeeded
    pub valid: bool,
    /// Whether orderbook has bids
    pub has_bids: bool,
    /// Whether orderbook has asks
    pub has_asks: bool,
    /// Best bid price
    pub best_bid: Option<f64>,
    /// Best ask price
    pub best_ask: Option<f64>,
    /// Error message if failed
    pub error: Option<String>,
}
