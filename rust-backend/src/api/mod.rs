//! API Layer
//!
//! HTTP routes and WebSocket server for the frontend.

pub mod routes;
pub mod static_files;
pub mod websocket;

use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use axum::{
    routing::{delete, get, post, put},
    Router,
};
use tokio::sync::{mpsc, RwLock};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::{error, info, warn, Level};

use crate::config::{Config, MentionConfig, MentionMode};
use crate::models::PriceUpdate;
use crate::services::{DiscordClient, PerformanceMetrics, Platform};
use crate::strategies::mention::{DirectionalExecutionService, MentionSignalClient};
use crate::strategies::sports_arb::ArbitrageService;

/// Application state shared across handlers
pub struct AppState {
    pub service: Arc<RwLock<ArbitrageService>>,
    pub config: Config,
    pub discord_client: Arc<DiscordClient>,
    pub directional_execution: Arc<DirectionalExecutionService>,
}

/// Create the Axum application
pub async fn create_app(config: Config) -> Result<Router> {
    // Initialize the arbitrage service
    let platform = Arc::new(Platform::new(config.clone())?);
    let mut service = ArbitrageService::new(&platform).await?;
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

    // Get metrics reference before moving service
    let metrics = service.metrics.clone();

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
            let _ = service.storage.set_directional_mode(MentionMode::Off);
            let _ = service.storage.set_directional_kill_switch(true);
            MentionConfig::default()
        }
    };
    service
        .storage
        .initialize_directional_mode(mention_config.mode)?;
    let mention_signals = MentionSignalClient::new(&mention_config)?;
    let directional_execution = Arc::new(DirectionalExecutionService::new(
        mention_config.clone(),
        service.storage.clone(),
        service.kalshi_client.clone(),
        mention_signals.clone(),
        discord_client.clone(),
    ));

    // Create shared state
    let state = Arc::new(AppState {
        service: Arc::new(RwLock::new(service)),
        config: config.clone(),
        discord_client,
        directional_execution,
    });

    let directional_executor = state.directional_execution.clone();
    let directional_interval = mention_config.poll_interval_secs.max(1);
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(tokio::time::Duration::from_secs(directional_interval));
        loop {
            interval.tick().await;
            directional_executor.run_once().await;
        }
    });

    crate::strategies::sports_arb::runner::spawn(
        state.service.clone(),
        state.discord_client.clone(),
        metrics.clone(),
        price_rx,
    );

    // Spawn metrics reporter and API ping tester (every 10 seconds)
    let state_for_metrics = state.clone();
    let metrics_clone = metrics.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(10));

        loop {
            interval.tick().await;

            // Perform API ping tests
            ping_apis(&state_for_metrics, &metrics_clone).await;

            // Reset metrics for next period (metrics are sent via WebSocket in websocket.rs)
            metrics_clone.reset();
        }
    });

    // Build router
    let app = Router::new()
        // Health check
        .route("/api/health", get(routes::health_check))
        // Authentication
        .route("/api/auth/login", post(routes::login))
        // Stats and data
        .route("/api/stats", get(routes::get_stats))
        .route("/api/data-coverage", get(routes::get_data_coverage))
        .route("/api/opportunities", get(routes::get_opportunities))
        .route("/api/matched-markets", get(routes::get_matched_markets))
        .route("/api/arbitrage-history", get(routes::get_arbitrage_history))
        // Account info
        .route("/api/balance/kalshi", get(routes::get_kalshi_balance))
        .route(
            "/api/balance/polymarket",
            get(routes::get_polymarket_balance),
        )
        .route("/api/account-balance", get(routes::get_account_balance))
        // Orders
        .route("/api/order/kalshi", post(routes::place_kalshi_order))
        .route(
            "/api/order/polymarket",
            post(routes::place_polymarket_order),
        )
        .route("/api/arbitrage/execute", post(routes::execute_arbitrage))
        // Order management
        .route("/api/orders/kalshi", get(routes::get_kalshi_orders))
        .route("/api/orders/polymarket", get(routes::get_polymarket_orders))
        .route(
            "/api/orders/kalshi/:order_id",
            delete(routes::cancel_kalshi_order),
        )
        .route(
            "/api/orders/polymarket/:order_id",
            delete(routes::cancel_polymarket_order),
        )
        // Position management
        .route("/api/positions/kalshi", get(routes::get_kalshi_positions))
        .route(
            "/api/positions/polymarket",
            get(routes::get_polymarket_positions),
        )
        .route("/api/positions/unified", get(routes::get_unified_positions))
        // Tracking
        .route("/api/tracking", get(routes::get_tracking))
        // History search
        .route("/api/history/search", get(routes::search_history))
        .route(
            "/api/history/statistics",
            get(routes::get_history_statistics),
        )
        // Orderbook depth
        .route("/api/orderbook/depth", get(routes::get_orderbook_depth))
        // Auto-trade
        .route("/api/auto-trade/status", get(routes::get_auto_trade_status))
        .route("/api/auto-trade/enable", post(routes::enable_auto_trade))
        .route("/api/auto-trade/disable", post(routes::disable_auto_trade))
        .route("/api/auto-trade/reset", post(routes::reset_auto_trade))
        .route(
            "/api/auto-trade/settings",
            put(routes::update_auto_trade_settings),
        )
        .route(
            "/api/auto-trade/history",
            get(routes::get_auto_trade_history),
        )
        .route("/api/auto-trade/pnl", get(routes::get_auto_trade_pnl))
        .route(
            "/api/auto-trade/excluded",
            get(routes::get_excluded_markets),
        )
        .route("/api/auto-trade/exclude", post(routes::exclude_market))
        .route("/api/auto-trade/unexclude", post(routes::unexclude_market))
        .route("/api/auto-trade/queue", get(routes::get_auto_trade_queue))
        // Independent single-leg directional mention strategy
        .route("/api/mention/status", get(routes::get_mention_status))
        .route("/api/mention/signals", get(routes::get_mention_signals))
        .route("/api/mention/history", get(routes::get_mention_history))
        .route("/api/mention/decisions", get(routes::get_mention_decisions))
        .route(
            "/api/mention/mode",
            axum::routing::put(routes::set_mention_mode),
        )
        .route("/api/mention/kill", post(routes::mention_kill_switch))
        .route(
            "/api/mention/reset-loss-breaker",
            post(routes::reset_mention_loss_breaker),
        )
        // App settings (hot-updatable)
        .route("/api/settings", get(routes::get_app_settings))
        .route("/api/settings", put(routes::update_app_settings))
        // WebSocket
        .route("/ws", get(websocket::ws_handler))
        // Add state
        .with_state(state)
        // Add middleware
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        // At minimum, every request against the UI's API gets a one-line
        // terminal/log entry on completion (method, path, status, latency) -
        // previously this only fired for failures (on_failure defaults to
        // ERROR), because a successful response's default trace level is
        // DEBUG and main.rs filters tower_http down to `warn`.
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        // Static files - must be last!
        .fallback(static_files::static_handler);

    info!("✅ API routes configured (including frontend static files)");

    Ok(app)
}

/// Ping both APIs to measure latency and cache balances
async fn ping_apis(state: &Arc<AppState>, metrics: &Arc<PerformanceMetrics>) {
    // Test Kalshi API latency and cache balance
    let kalshi_start = Instant::now();
    let service = state.service.read().await;

    match service.kalshi_client.get_balance().await {
        Ok(balance) => {
            let latency_ms = kalshi_start.elapsed().as_millis() as u64;
            metrics.set_kalshi_latency(latency_ms);
            metrics.set_kalshi_balance(balance);
        }
        Err(e) => {
            warn!("Kalshi API ping failed: {}", e);
        }
    }

    // Test Polymarket API latency and cache balance
    let poly_start = Instant::now();
    match service.polymarket_client.get_balance().await {
        Ok(balance) => {
            let latency_ms = poly_start.elapsed().as_millis() as u64;
            metrics.set_polymarket_latency(latency_ms);
            metrics.set_polymarket_balance(balance);
        }
        Err(e) => {
            warn!("Polymarket API ping failed: {}", e);
        }
    }
}

