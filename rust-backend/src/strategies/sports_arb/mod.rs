//! Cross-exchange sports (and crypto window) arbitrage between Kalshi and
//! Polymarket US: market matching, detection, paired execution.

pub mod execution;
pub mod routes;
pub mod runner;
pub mod service;
pub mod strategy;
pub mod websocket_manager;

pub use execution::PairedOrderParams;
pub use service::ArbitrageService;
pub use strategy::SportsArb;
pub use websocket_manager::WebSocketManager;
