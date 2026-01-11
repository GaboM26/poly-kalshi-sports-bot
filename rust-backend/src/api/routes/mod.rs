//! HTTP API Routes
//!
//! Organized by functional domain:
//! - health: Health check endpoints
//! - stats: Statistics and monitoring
//! - markets: Market data queries
//! - orders: Order operations
//! - accounts: Account balances and positions
//! - auto_trade: Auto-trade API
//! - settings: Application settings
//! - history: History queries and search

mod health;
mod stats;
mod markets;
mod orders;
mod accounts;
mod auto_trade;
mod settings;
mod history;

// Re-export all handlers
pub use health::*;
pub use stats::*;
pub use markets::*;
pub use orders::*;
pub use accounts::*;
pub use auto_trade::*;
pub use settings::*;
pub use history::*;
