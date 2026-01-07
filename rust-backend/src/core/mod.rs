//! Core business logic
//!
//! Contains the arbitrage calculator and market matcher.

pub mod calculator;
pub mod matcher;

pub use calculator::ArbitrageCalculator;
pub use matcher::EventMatcher;
