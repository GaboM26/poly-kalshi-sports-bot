//! Sports arbitrage as a `Strategy`: startup, background tasks, routes and
//! frontend messages.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use axum::routing::{get, post, put};
use tokio::sync::{mpsc, RwLock};
use tokio::task::JoinHandle;
use tracing::error;

use super::routes;
use crate::models::{PriceUpdate, WsMessage};
use crate::services::positions::hedge_pairs_for_market;
use crate::services::storage::HedgePair;
use crate::services::{DiscordClient, PerformanceMetrics, Platform};
use crate::strategies::sports_arb::{runner, ArbitrageService};
use crate::strategies::{send_ws, RouteTable, Strategy};

/// How often each frontend client gets the full matched-market list.
const MATCHED_MARKETS_PUSH: Duration = Duration::from_secs(2);

pub struct SportsArb {
    pub service: Arc<RwLock<ArbitrageService>>,
    pub discord_client: Arc<DiscordClient>,
    metrics: Arc<PerformanceMetrics>,
    /// Handed to the price task on `start`.
    price_rx: parking_lot::Mutex<Option<mpsc::Receiver<PriceUpdate>>>,
}

impl SportsArb {
    /// Load and match markets, connect both exchange feeds and start the
    /// quote refresh loops. Auto-trade loops start in `start`.
    pub async fn new(platform: &Arc<Platform>) -> Result<Arc<Self>> {
        let config = &platform.config;
        let mut service = ArbitrageService::new(platform).await?;
        service.initialize().await?;
        if service
            .ws_manager
            .get_storage()
            .has_submitting_auto_trade_execution()?
        {
            service.ws_manager.disable_auto_trade()?;
            error!(
                "Auto-trading was halted at startup because an interrupted paired execution requires reconciliation"
            );
        }

        // Create price update channel
        let (price_tx, price_rx) = mpsc::channel::<PriceUpdate>(10000);

        // Start WebSocket connections
        service.start_websocket_connections(price_tx).await?;

        // Start periodic scanning
        service
            .run_periodic_scan(config.settings.refresh_interval)
            .await;
        service
            .run_polymarket_quote_refresh(config.settings.refresh_interval)
            .await;
        service
            .run_kalshi_quote_refresh(config.settings.refresh_interval)
            .await;
        service.run_ws_stats_reporter();
        match crate::clients::polymarket::resolve_credentials(&config.polymarket) {
            Ok((key_id, secret_key)) => service.run_polymarket_ws(config.polymarket.ws, key_id, secret_key),
            Err(e) => error!("Polymarket WebSocket disabled, REST polling only: {:#}", e),
        }

        let discord_client = platform.discord.clone();
        if discord_client.is_enabled() {
            service.ws_manager.set_discord_client(discord_client.clone());
        }

        Ok(Arc::new(Self {
            metrics: service.metrics.clone(),
            service: Arc::new(RwLock::new(service)),
            discord_client,
            price_rx: parking_lot::Mutex::new(Some(price_rx)),
        }))
    }
}

#[async_trait]
impl Strategy for SportsArb {
    fn id(&self) -> &'static str {
        "sports_arb"
    }

    fn routes(self: Arc<Self>) -> RouteTable {
        let state = self;
        vec![
            ("/stats", get(routes::get_stats).with_state(state.clone())),
            ("/data-coverage", get(routes::get_data_coverage).with_state(state.clone())),
            ("/opportunities", get(routes::get_opportunities).with_state(state.clone())),
            ("/matched-markets", get(routes::get_matched_markets).with_state(state.clone())),
            ("/arbitrage-history", get(routes::get_arbitrage_history).with_state(state.clone())),
            ("/arbitrage/execute", post(routes::execute_arbitrage).with_state(state.clone())),
            ("/tracking", get(routes::get_tracking).with_state(state.clone())),
            ("/history/search", get(routes::search_history).with_state(state.clone())),
            ("/history/statistics", get(routes::get_history_statistics).with_state(state.clone())),
            ("/orderbook/depth", get(routes::get_orderbook_depth).with_state(state.clone())),
            ("/auto-trade/status", get(routes::get_auto_trade_status).with_state(state.clone())),
            ("/auto-trade/enable", post(routes::enable_auto_trade).with_state(state.clone())),
            ("/auto-trade/disable", post(routes::disable_auto_trade).with_state(state.clone())),
            ("/auto-trade/reset", post(routes::reset_auto_trade).with_state(state.clone())),
            ("/auto-trade/settings", put(routes::update_auto_trade_settings).with_state(state.clone())),
            ("/auto-trade/history", get(routes::get_auto_trade_history).with_state(state.clone())),
            ("/auto-trade/pnl", get(routes::get_auto_trade_pnl).with_state(state.clone())),
            ("/auto-trade/excluded", get(routes::get_excluded_markets).with_state(state.clone())),
            ("/auto-trade/exclude", post(routes::exclude_market).with_state(state.clone())),
            ("/auto-trade/unexclude", post(routes::unexclude_market).with_state(state.clone())),
            ("/auto-trade/queue", get(routes::get_auto_trade_queue).with_state(state.clone())),
            (
                "/settings",
                get(routes::get_app_settings)
                    .put(routes::update_app_settings)
                    .with_state(state),
            ),
        ]
    }

    fn start(self: Arc<Self>) {
        let Some(price_rx) = self.price_rx.lock().take() else {
            error!("sports_arb started twice; ignoring");
            return;
        };
        runner::spawn(
            self.service.clone(),
            self.discord_client.clone(),
            self.metrics.clone(),
            price_rx,
        );
    }

    async fn feed_client(self: Arc<Self>, tx: mpsc::Sender<String>) -> Vec<JoinHandle<()>> {
        // Send initial data
        {
            let service = self.service.read().await;

            // Send matched markets list (key message for frontend)
            let markets = service.ws_manager.get_matched_markets_for_frontend();
            let opportunities_count = markets.iter().filter(|m| m.has_opportunity).count();
            let msg = WsMessage::MatchedMarketsList {
                data: markets.clone(),
                count: markets.len(),
                opportunities_count,
            };
            send_ws(&tx, &msg).await;

            // Send stats
            let stats = service.get_stats();
            send_ws(&tx, &WsMessage::Stats { data: stats }).await;

            // Send current opportunities
            let opportunities = service.get_opportunities();
            if !opportunities.is_empty() {
                send_ws(&tx, &WsMessage::Opportunities { data: opportunities }).await;
            }
        }

        // Subscribe to opportunity updates and scan stats
        let service = self.service.read().await;
        let mut opportunity_rx = service.ws_manager.subscribe();
        let mut scan_stats_rx = service.ws_manager.subscribe_scan_stats();
        drop(service);

        // Periodic matched_markets_list updates
        let periodic_service = self.service.clone();
        let tx_periodic = tx.clone();
        let periodic_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(MATCHED_MARKETS_PUSH);
            loop {
                interval.tick().await;

                let service = periodic_service.read().await;
                let markets = service.ws_manager.get_matched_markets_for_frontend();
                let opportunities_count = markets.iter().filter(|m| m.has_opportunity).count();

                let msg = WsMessage::MatchedMarketsList {
                    data: markets.clone(),
                    count: markets.len(),
                    opportunities_count,
                };
                if !send_ws(&tx_periodic, &msg).await {
                    break;
                }
            }
        });

        // Forward opportunity updates
        let tx_opp = tx.clone();
        let opportunity_task = tokio::spawn(async move {
            while let Ok(opportunity) = opportunity_rx.recv().await {
                if !send_ws(&tx_opp, &WsMessage::Opportunity { data: opportunity }).await {
                    break;
                }
            }
        });

        // Forward scan stats updates
        let scan_stats_task = tokio::spawn(async move {
            while let Ok(scan_stats) = scan_stats_rx.recv().await {
                if !send_ws(&tx, &WsMessage::ScanStats { data: scan_stats }).await {
                    break;
                }
            }
        });

        vec![periodic_task, opportunity_task, scan_stats_task]
    }

    async fn hedge_pairs(&self) -> Vec<HedgePair> {
        let service = self.service.read().await;
        service
            .get_matched_markets()
            .iter()
            .flat_map(hedge_pairs_for_market)
            .collect()
    }
}
