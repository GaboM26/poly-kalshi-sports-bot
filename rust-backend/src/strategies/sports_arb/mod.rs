//! Cross-exchange sports (and crypto window) arbitrage between Kalshi and
//! Polymarket US: market matching, detection, paired execution.

pub mod execution;
pub mod runner;
pub mod service;
pub mod websocket_manager;

pub use execution::PairedOrderParams;
pub use service::ArbitrageService;
pub use websocket_manager::WebSocketManager;
