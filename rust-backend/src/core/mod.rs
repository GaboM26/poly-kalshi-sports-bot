//! Core business logic
//!
//! Contains the arbitrage calculator, exchange fee models, market matcher, and
//! team normalization.

pub mod calculator;
pub mod competitors;
pub mod fees;
pub mod matcher;
pub mod nba_teams;
pub mod team_codes;

pub use calculator::ArbitrageCalculator;
pub use competitors::{competitor_event_name, normalize_competitor_name};
pub use matcher::{EventMatcher, SubscriptionInfo};
pub use nba_teams::normalize_team_name;
pub use team_codes::{canonical_team_code, team_game_event_name};
