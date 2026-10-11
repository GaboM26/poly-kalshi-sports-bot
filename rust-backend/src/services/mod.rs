//! Services layer
//!
//! Contains:
//! - ArbitrageService: Orchestrates market scanning and arbitrage detection
//! - WebSocketManager: Manages real-time connections to both platforms
//! - Storage: SQLite persistence for arbitrage tracking
//! - Metrics: Performance monitoring and API latency tracking
//! - Discord: webhook notifications for trades and opportunities

pub mod arbitrage;
pub mod discord;
pub mod metrics;
pub mod paired_execution;
pub mod pnl;
pub mod positions;
pub mod storage;
pub mod websocket_manager;

pub use arbitrage::ArbitrageService;
pub use metrics::{Operation, PerformanceMetrics};
pub use paired_execution::PairedOrderParams;
pub use storage::{ArbitrageStorage, AutoTradeExecutionRecord};
pub use discord::DiscordClient;
pub use websocket_manager::WebSocketManager;
