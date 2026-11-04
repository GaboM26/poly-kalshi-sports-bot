//! Directional Kalshi mention markets ("will Trump say X"), driven by the
//! local kalshi-mention signal service.

pub mod execution;
pub mod routes;
pub mod signal;
pub mod strategy;

pub use execution::DirectionalExecutionService;
pub use signal::MentionSignalClient;
pub use strategy::Mention;
