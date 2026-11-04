//! Services layer
//!
//! Contains:
//! Shared platform services used by every strategy:
//! - Storage: SQLite persistence for arbitrage tracking
//! - Metrics: Performance monitoring and API latency tracking
//! - Discord: webhook notifications for trades and opportunities

pub mod discord;
pub mod metrics;
pub mod pnl;
pub mod positions;
pub mod storage;

pub use metrics::{Operation, PerformanceMetrics};
pub use storage::{ArbitrageStorage, AutoTradeExecutionRecord};
pub use discord::DiscordClient;
