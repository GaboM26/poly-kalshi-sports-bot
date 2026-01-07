//! Signing module for CLOB authentication
//!
//! Implements EIP-712 and HMAC signing for Polymarket authentication.

pub mod eip712;
pub mod hmac;

pub use eip712::{sign_clob_auth_message, ClobAuthDomain};
pub use hmac::build_hmac_signature;
