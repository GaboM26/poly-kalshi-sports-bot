//! Trading strategies. Each strategy owns its detection, execution, policy,
//! HTTP routes and live frontend messages, and uses the shared `Platform`
//! (exchange clients, storage, metrics, Discord).
//!
//! Adding a strategy: a folder here implementing `Strategy`, one line in
//! `build_all`, a config section, and a page in the web registry.

pub mod mention;
pub mod sports_arb;

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use axum::routing::MethodRouter;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::services::storage::HedgePair;
use crate::services::Platform;

/// Relative HTTP routes of one strategy, mounted under `/api/<id>`.
pub type RouteTable = Vec<(&'static str, MethodRouter)>;

#[async_trait]
pub trait Strategy: Send + Sync {
    /// Stable id: URL prefix (`/api/<id>/...`) and web registry key.
    fn id(&self) -> &'static str;

    /// HTTP routes, paths relative to `/api/<id>`.
    fn routes(self: Arc<Self>) -> RouteTable;

    /// Start the strategy's background tasks. Called once.
    fn start(self: Arc<Self>);

    /// Send this strategy's initial state to one frontend WebSocket client,
    /// then keep pushing its live messages. Returns the per-client tasks so
    /// the caller can stop them when the client disconnects.
    async fn feed_client(self: Arc<Self>, tx: mpsc::Sender<String>) -> Vec<JoinHandle<()>>;

    /// Kalshi/Polymarket hedge pairs this strategy knows of, used to group
    /// the shared positions view. Single-venue strategies have none.
    async fn hedge_pairs(&self) -> Vec<HedgePair> {
        Vec::new()
    }
}

/// Every strategy, in display order.
pub async fn build_all(platform: &Arc<Platform>) -> Result<Vec<Arc<dyn Strategy>>> {
    Ok(vec![
        sports_arb::SportsArb::new(platform).await?,
        mention::Mention::new(platform)?,
    ])
}

/// Serialize a frontend message and queue it; false when the client is gone.
pub(crate) async fn send_ws(tx: &mpsc::Sender<String>, message: &crate::models::WsMessage) -> bool {
    match serde_json::to_string(message) {
        Ok(json) => tx.send(json).await.is_ok(),
        Err(_) => true,
    }
}
