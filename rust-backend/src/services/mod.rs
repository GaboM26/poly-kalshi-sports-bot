//! Services layer
//!
//! Contains:
//! - ArbitrageService: Orchestrates market scanning and arbitrage detection
//! - WebSocketManager: Manages real-time connections to both platforms
//! - Storage: SQLite persistence for arbitrage tracking

pub mod arbitrage;
pub mod storage;
pub mod websocket_manager;

pub use arbitrage::ArbitrageService;
pub use storage::ArbitrageStorage;
pub use websocket_manager::WebSocketManager;
