//! API Layer
//!
//! HTTP routes and WebSocket server for the frontend. Shared routes
//! (accounts, positions, manual orders) are registered here; each strategy
//! contributes its own routes under `/api/<strategy id>`.

pub mod routes;
pub mod static_files;
pub mod websocket;

use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use axum::{
    routing::{delete, get, post},
    Router,
};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::{info, warn, Level};

use crate::config::Config;
use crate::services::{PerformanceMetrics, Platform};
use crate::strategies::{self, Strategy};

/// How often API latency is measured and metrics are reset.
const METRICS_PERIOD: std::time::Duration = std::time::Duration::from_secs(10);

/// Strategies whose routes are also served at their pre-strategy paths
/// (`/api/<route>` without the id), until the frontend has switched to
/// `/api/<id>/<route>`. Remove an entry once nothing calls the old paths.
const LEGACY_UNPREFIXED_ROUTES: &[&str] = &["sports_arb"];

/// Application state shared across the shared handlers
pub struct AppState {
    pub config: Config,
    pub platform: Arc<Platform>,
    pub strategies: Vec<Arc<dyn Strategy>>,
}

/// Create the Axum application
pub async fn create_app(config: Config) -> Result<Router> {
    let platform = Arc::new(Platform::new(config.clone())?);
    let strategies = strategies::build_all(&platform).await?;
    for strategy in &strategies {
        strategy.clone().start();
    }

    // Create shared state
    let state = Arc::new(AppState {
        config: config.clone(),
        platform: platform.clone(),
        strategies: strategies.clone(),
    });

    // Spawn metrics reporter and API ping tester (every 10 seconds)
    let platform_for_metrics = platform.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(METRICS_PERIOD);

        loop {
            interval.tick().await;

            // Perform API ping tests
            ping_apis(&platform_for_metrics).await;

            // Reset metrics for next period (metrics are sent via WebSocket in websocket.rs)
            platform_for_metrics.metrics.reset();
        }
    });

    // Build router
    let mut app = Router::new()
        // Health check
        .route("/api/health", get(routes::health_check))
        // Authentication
        .route("/api/auth/login", post(routes::login))
        // Account info
        .route("/api/balance/kalshi", get(routes::get_kalshi_balance))
        .route(
            "/api/balance/polymarket",
            get(routes::get_polymarket_balance),
        )
        .route("/api/account-balance", get(routes::get_account_balance))
        // Manual orders
        .route("/api/order/kalshi", post(routes::place_kalshi_order))
        .route(
            "/api/order/polymarket",
            post(routes::place_polymarket_order),
        )
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
        // WebSocket
        .route("/ws", get(websocket::ws_handler))
        // Add state
        .with_state(state);

    // Strategy routes, each under /api/<id>
    for strategy in &strategies {
        let id = strategy.id();
        for (path, handler) in strategy.clone().routes() {
            if LEGACY_UNPREFIXED_ROUTES.contains(&id) {
                app = app.route(&format!("/api{path}"), handler.clone());
            }
            app = app.route(&format!("/api/{id}{path}"), handler);
        }
    }

    let app = app
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
async fn ping_apis(platform: &Platform) {
    let metrics: &PerformanceMetrics = &platform.metrics;
    // Test Kalshi API latency and cache balance
    let kalshi_start = Instant::now();

    match platform.kalshi.get_balance().await {
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
    match platform.polymarket.get_balance().await {
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
