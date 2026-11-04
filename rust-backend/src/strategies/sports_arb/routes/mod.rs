//! Sports-arb HTTP handlers, mounted under `/api/sports_arb` (see
//! `SportsArb::routes`).

mod auto_trade;
mod execute;
mod history;
mod markets;
mod settings;
mod stats;

pub use auto_trade::*;
pub use execute::*;
pub use history::*;
pub use markets::*;
pub use settings::*;
pub use stats::*;
