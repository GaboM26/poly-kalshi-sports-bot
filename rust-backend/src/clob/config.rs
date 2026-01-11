//! CLOB contract configuration
//!
//! Contract addresses for Polygon Mainnet (chain_id: 137)

use alloy::primitives::Address;
use std::str::FromStr;

/// Contract configuration for a specific chain
#[derive(Debug, Clone)]
pub struct ContractConfig {
    /// Exchange contract address (for neg_risk=false markets)
    pub exchange: Address,
    /// Neg Risk CTF Exchange contract address (for neg_risk=true markets)
    pub neg_risk_exchange: Address,
    /// Collateral token address (USDC)
    pub collateral: Address,
    /// Conditional tokens contract address
    pub conditional_tokens: Address,
}

/// Polygon Mainnet chain ID
pub const POLYGON_CHAIN_ID: u64 = 137;

/// Amoy Testnet chain ID
pub const AMOY_CHAIN_ID: u64 = 80002;

/// Get contract config for a given chain ID
pub fn get_contract_config(chain_id: u64) -> Option<ContractConfig> {
    match chain_id {
        POLYGON_CHAIN_ID => Some(ContractConfig {
            exchange: Address::from_str("0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E").unwrap(),
            // NegRiskCtfExchange for neg_risk=true markets (e.g., NBA sports markets)
            neg_risk_exchange: Address::from_str("0xC5d563A36AE78145C45a50134d48A1215220f80a").unwrap(),
            collateral: Address::from_str("0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174").unwrap(),
            conditional_tokens: Address::from_str("0x4D97DCd97eC945f40cF65F87097ACe5EA0476045")
                .unwrap(),
        }),
        AMOY_CHAIN_ID => Some(ContractConfig {
            exchange: Address::from_str("0xdFE02Eb6733538f8Ea35D585af8DE5958AD99E40").unwrap(),
            // Amoy testnet neg_risk exchange (same as mainnet pattern)
            neg_risk_exchange: Address::from_str("0x87d1A0DdB4C63a6301916F02090A51a7241571e4").unwrap(),
            collateral: Address::from_str("0x9c4e1703476e875070ee25b56a58b008cfb8fa78").unwrap(),
            conditional_tokens: Address::from_str("0x69308FB512518e39F9b16112fA8d994F4e2Bf8bB")
                .unwrap(),
        }),
        _ => None,
    }
}

/// CLOB API host URLs
pub mod hosts {
    /// Production CLOB API
    pub const CLOB_HOST: &str = "https://clob.polymarket.com";
    /// Gamma API for market data
    pub const GAMMA_HOST: &str = "https://gamma-api.polymarket.com";
    /// WebSocket host for price updates
    pub const WS_HOST: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";
}

/// CLOB API endpoints
pub mod endpoints {
    // Public endpoints (no auth required)
    pub const GET_OK: &str = "/";
    pub const GET_SERVER_TIME: &str = "/time";
    pub const GET_MARKETS: &str = "/markets";
    pub const GET_MARKET: &str = "/markets/{}"; // market_id
    pub const GET_ORDER_BOOK: &str = "/book";
    pub const GET_ORDER_BOOKS: &str = "/books";
    pub const GET_PRICE: &str = "/price";
    pub const GET_PRICES: &str = "/prices";
    pub const GET_SPREAD: &str = "/spread";
    pub const GET_SPREADS: &str = "/spreads";
    pub const GET_MID_POINT: &str = "/midpoint";
    pub const GET_MID_POINTS: &str = "/midpoints";
    pub const GET_LAST_TRADE_PRICE: &str = "/last-trade-price";
    pub const GET_LAST_TRADES_PRICES: &str = "/last-trades-prices";
    pub const GET_TICK_SIZE: &str = "/tick-size";
    pub const GET_NEG_RISK: &str = "/neg-risk";
    pub const GET_SAMPLING_SIMPLIFIED_MARKETS: &str = "/sampling-simplified-markets";
    pub const GET_SAMPLING_MARKETS: &str = "/sampling-markets";

    // Level 1 auth endpoints (EIP-712 signature)
    pub const GET_API_KEYS: &str = "/auth/api-keys";
    pub const CREATE_API_KEY: &str = "/auth/api-key";
    pub const DELETE_API_KEY: &str = "/auth/api-key";
    pub const DERIVE_API_KEY: &str = "/auth/derive-api-key";
    pub const GET_READONLY_API_KEYS: &str = "/auth/api-keys/readonly";
    pub const CREATE_READONLY_API_KEY: &str = "/auth/api-key/readonly";
    pub const DELETE_READONLY_API_KEY: &str = "/auth/api-key/readonly";

    // Level 2 auth endpoints (HMAC signature)
    pub const GET_ORDER: &str = "/data/order/{}"; // order_id
    pub const GET_ORDERS: &str = "/data/orders";
    pub const POST_ORDER: &str = "/order";
    pub const CANCEL_ORDER: &str = "/order";
    pub const CANCEL_ORDERS: &str = "/orders";
    pub const CANCEL_ALL: &str = "/cancel-all";
    pub const CANCEL_MARKET_ORDERS: &str = "/cancel-market-orders";
    pub const GET_TRADES: &str = "/data/trades";
    pub const GET_BALANCE_ALLOWANCE: &str = "/balance-allowance";
    pub const UPDATE_BALANCE_ALLOWANCE: &str = "/balance-allowance";
    pub const IS_ORDER_SCORING: &str = "/order-scoring";
    pub const ARE_ORDERS_SCORING: &str = "/orders-scoring";
    pub const DROP_NOTIFICATIONS: &str = "/notifications";
    pub const GET_NOTIFICATIONS: &str = "/notifications";
    pub const POST_HEARTBEAT: &str = "/heartbeat";
}

/// Token decimal places (USDC has 6 decimals)
pub const TOKEN_DECIMALS: u32 = 6;

/// Tick size for prices
pub const TICK_SIZE: f64 = 0.01;

/// Minimum tick size
pub const MIN_TICK_SIZE: f64 = 0.001;

/// Helper to convert to token decimals
pub fn to_token_decimals(amount: f64) -> u64 {
    (amount * 10_f64.powi(TOKEN_DECIMALS as i32)).round() as u64
}

/// Helper to convert from token decimals
pub fn from_token_decimals(amount: u64) -> f64 {
    amount as f64 / 10_f64.powi(TOKEN_DECIMALS as i32)
}
