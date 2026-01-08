//! API Layer
//!
//! HTTP routes and WebSocket server for the frontend.

pub mod routes;
pub mod websocket;

use std::sync::Arc;

use anyhow::Result;
use axum::{
    routing::{get, post, delete},
    Router,
};
use tokio::sync::{mpsc, RwLock};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::config::Config;
use crate::models::PriceUpdate;
use crate::services::ArbitrageService;

/// Application state shared across handlers
pub struct AppState {
    pub service: RwLock<ArbitrageService>,
    pub config: Config,
}

/// Create the Axum application
pub async fn create_app(config: Config) -> Result<Router> {
    // Initialize the arbitrage service
    let mut service = ArbitrageService::new(&config).await?;
    service.initialize().await?;

    // Create price update channel
    let (price_tx, mut price_rx) = mpsc::channel::<PriceUpdate>(10000);

    // Start WebSocket connections
    service.start_websocket_connections(price_tx).await?;

    // Start periodic scanning
    service.run_periodic_scan(config.settings.refresh_interval).await;

    // Create shared state
    let state = Arc::new(AppState {
        service: RwLock::new(service),
        config: config.clone(),
    });

    // Spawn price update handler
    let state_clone = state.clone();
    tokio::spawn(async move {
        while let Some(update) = price_rx.recv().await {
            let service = state_clone.service.read().await;
            service.ws_manager.on_price_update(update);
        }
    });

    // Build router
    let app = Router::new()
        // Health check
        .route("/", get(routes::health_check))
        .route("/api/health", get(routes::health_check))
        // Stats and data
        .route("/api/stats", get(routes::get_stats))
        .route("/api/data-coverage", get(routes::get_data_coverage))
        .route("/api/opportunities", get(routes::get_opportunities))
        .route("/api/matched-markets", get(routes::get_matched_markets))
        .route("/api/arbitrage-history", get(routes::get_arbitrage_history))
        // Account info
        .route("/api/balance/kalshi", get(routes::get_kalshi_balance))
        .route("/api/balance/polymarket", get(routes::get_polymarket_balance))
        .route("/api/account-balance", get(routes::get_account_balance))
        // Orders
        .route("/api/order/kalshi", post(routes::place_kalshi_order))
        .route("/api/order/polymarket", post(routes::place_polymarket_order))
        .route("/api/arbitrage/execute", post(routes::execute_arbitrage))
        // Order management
        .route("/api/orders/kalshi", get(routes::get_kalshi_orders))
        .route("/api/orders/polymarket", get(routes::get_polymarket_orders))
        .route("/api/orders/kalshi/:order_id", delete(routes::cancel_kalshi_order))
        .route("/api/orders/polymarket/:order_id", delete(routes::cancel_polymarket_order))
        // Position management
        .route("/api/positions/kalshi", get(routes::get_kalshi_positions))
        .route("/api/positions/polymarket", get(routes::get_polymarket_positions))
        // Tracking
        .route("/api/tracking", get(routes::get_tracking))
        // History search
        .route("/api/history/search", get(routes::search_history))
        .route("/api/history/statistics", get(routes::get_history_statistics))
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
        .layer(TraceLayer::new_for_http());

    info!("✅ API 路由配置完成");

    Ok(app)
}
