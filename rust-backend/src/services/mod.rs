//! Services layer
//!
//! Contains:
//! - ArbitrageService: Orchestrates market scanning and arbitrage detection
//! - WebSocketManager: Manages real-time connections to both platforms
//! - Storage: SQLite persistence for arbitrage tracking
//! - Metrics: Performance monitoring and API latency tracking
//! - Telegram: Telegram notification service for auto-trade alerts

pub mod arbitrage;
pub mod metrics;
pub mod storage;
pub mod telegram;
pub mod websocket_manager;

pub use arbitrage::ArbitrageService;
pub use metrics::{PerformanceMetrics, MetricsReport, ApiLatency, Operation};
pub use storage::{ArbitrageStorage, AppSettings, AutoTradeState, AutoTradeRecord, StorageStats};
pub use telegram::TelegramClient;
pub use websocket_manager::WebSocketManager;
