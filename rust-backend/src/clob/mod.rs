//! Polymarket CLOB Client Implementation
//!
//! This module implements the Polymarket CLOB (Central Limit Order Book) client,
//! including signing, authentication headers, and order building.

pub mod client;
pub mod config;
pub mod headers;
pub mod order_builder;
pub mod signer;
pub mod signing;
pub mod types;

pub use client::ClobClient;
pub use config::ContractConfig;
pub use signer::Signer;
pub use types::*;
