//! Platform clients for Kalshi and Polymarket
//!
//! These clients handle:
//! - REST API interactions
//! - WebSocket connections for real-time price updates
//! - Authentication (RSA for Kalshi, EIP-712/HMAC for Polymarket)

pub mod kalshi;
pub mod polymarket;

pub use kalshi::KalshiClient;
pub use polymarket::PolymarketClient;
