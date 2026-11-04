//! Directional mention markets as a `Strategy`.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use axum::routing::{get, post, put};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::error;

use super::routes;
use crate::config::{MentionConfig, MentionMode};
use crate::models::WsMessage;
use crate::services::Platform;
use crate::strategies::mention::{DirectionalExecutionService, MentionSignalClient};
use crate::strategies::{send_ws, RouteTable, Strategy};

/// How often each frontend client gets the signal dashboard.
const DASHBOARD_PUSH: Duration = Duration::from_secs(2);

pub struct Mention {
    pub directional_execution: Arc<DirectionalExecutionService>,
    poll_interval_secs: u64,
}

impl Mention {
    pub fn new(platform: &Arc<Platform>) -> Result<Arc<Self>> {
        let config = &platform.config;
        // The directional strategy is isolated from the paired path: an invalid [mention]
        // section must never stop arbitrage from starting. It fails closed instead, running
        // with defaults, mode off, and the directional kill switch engaged.
        let mention_config = match config.mention.validate() {
            Ok(()) => config.mention.clone(),
            Err(reason) => {
                error!(
                    "Invalid [mention] config; directional trading is disabled: {}",
                    reason
                );
                let _ = platform.storage.set_directional_mode(MentionMode::Off);
                let _ = platform.storage.set_directional_kill_switch(true);
                MentionConfig::default()
            }
        };
        platform
            .storage
            .initialize_directional_mode(mention_config.mode)?;
        let mention_signals = MentionSignalClient::new(&mention_config)?;
        let directional_execution = Arc::new(DirectionalExecutionService::new(
            mention_config.clone(),
            platform.storage.clone(),
            platform.kalshi.clone(),
            mention_signals.clone(),
            platform.discord.clone(),
        ));
        Ok(Arc::new(Self {
            directional_execution,
            poll_interval_secs: mention_config.poll_interval_secs.max(1),
        }))
    }

    fn dashboard(&self) -> WsMessage {
        let status = self.directional_execution.status().unwrap_or_else(|error| {
            serde_json::json!({"mode":"off","error":error.to_string()})
        });
        WsMessage::MentionSignals {
            data: self.directional_execution.signal_dashboard(),
            status,
        }
    }
}

#[async_trait]
impl Strategy for Mention {
    fn id(&self) -> &'static str {
        "mention"
    }

    fn routes(self: Arc<Self>) -> RouteTable {
        let state = self;
        vec![
            ("/status", get(routes::get_mention_status).with_state(state.clone())),
            ("/signals", get(routes::get_mention_signals).with_state(state.clone())),
            ("/history", get(routes::get_mention_history).with_state(state.clone())),
            ("/decisions", get(routes::get_mention_decisions).with_state(state.clone())),
            ("/mode", put(routes::set_mention_mode).with_state(state.clone())),
            ("/kill", post(routes::mention_kill_switch).with_state(state.clone())),
            ("/reset-loss-breaker", post(routes::reset_mention_loss_breaker).with_state(state)),
        ]
    }

    fn start(self: Arc<Self>) {
        let directional_executor = self.directional_execution.clone();
        let directional_interval = self.poll_interval_secs;
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(tokio::time::Duration::from_secs(directional_interval));
            loop {
                interval.tick().await;
                directional_executor.run_once().await;
            }
        });
    }

    async fn feed_client(self: Arc<Self>, tx: mpsc::Sender<String>) -> Vec<JoinHandle<()>> {
        send_ws(&tx, &self.dashboard()).await;
        let task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(DASHBOARD_PUSH);
            loop {
                interval.tick().await;
                if !send_ws(&tx, &self.dashboard()).await {
                    break;
                }
            }
        });
        vec![task]
    }
}
