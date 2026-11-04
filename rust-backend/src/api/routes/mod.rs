//! Shared HTTP routes (accounts, positions, manual orders, health). Each
//! strategy's own routes live in its folder under `strategies/`.

mod accounts;
mod health;
mod orders;

pub use accounts::*;
pub use health::*;
pub use orders::*;
