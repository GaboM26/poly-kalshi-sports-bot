//! Directional Kalshi mention markets ("will Trump say X"), driven by the
//! local kalshi-mention signal service.

pub mod execution;
pub mod signal;

pub use execution::DirectionalExecutionService;
pub use signal::MentionSignalClient;
