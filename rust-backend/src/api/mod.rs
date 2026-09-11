//! API Layer
//!
//! HTTP routes and WebSocket server for the frontend.

pub mod routes;
pub mod static_files;
pub mod websocket;

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use axum::{
    routing::{delete, get, post, put},
    Router,
};
use chrono::Utc;
use tokio::sync::{mpsc, RwLock};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::clients::{KalshiOrderResult, PolymarketBookLevel, PolymarketMarketBook, PolymarketOrderResult};
use crate::models::{PolymarketPositionSide, PriceUpdate};
use crate::services::{ArbitrageService, AutoTradeExecutionRecord, PerformanceMetrics, TelegramClient};

/// Market scan interval in seconds (5 minutes)
const MARKET_SCAN_INTERVAL_SECS: u64 = 300;
/// A cached websocket book may only drive an automatic order while it is this
/// recent. The Python endpoint is requested synchronously, so its response is
/// inherently a fresh Polymarket US book.
const EXECUTABLE_BOOK_MAX_AGE: Duration = Duration::from_secs(2);

/// Application state shared across handlers
pub struct AppState {
    pub service: RwLock<ArbitrageService>,
    pub config: Config,
    pub telegram_client: Arc<TelegramClient>,
}

/// Create the Axum application
pub async fn create_app(config: Config) -> Result<Router> {
    // Initialize the arbitrage service
    let mut service = ArbitrageService::new(&config).await?;
    service.initialize().await?;

    // Get metrics reference before moving service
    let metrics = service.metrics.clone();

    // Create price update channel
    let (price_tx, mut price_rx) = mpsc::channel::<PriceUpdate>(10000);

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

    // Initialize Telegram client
    let telegram_client = Arc::new(TelegramClient::new(config.telegram.clone()));
    if telegram_client.is_enabled() {
        info!("✅ Telegram notifications enabled");
    } else {
        info!("ℹ️ Telegram notifications disabled");
    }

    // Create shared state
    let state = Arc::new(AppState {
        service: RwLock::new(service),
        config: config.clone(),
        telegram_client,
    });

    // Spawn price update handler
    let state_clone = state.clone();
    tokio::spawn(async move {
        while let Some(update) = price_rx.recv().await {
            let service = state_clone.service.read().await;
            service.ws_manager.on_price_update(update);
        }
    });

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

    // Spawn periodic market scanner (every 5 minutes)
    let state_for_scanner = state.clone();
    tokio::spawn(async move {
        info!(
            "🔍 Market scan task started, interval {} seconds ({} minutes)",
            MARKET_SCAN_INTERVAL_SECS,
            MARKET_SCAN_INTERVAL_SECS / 60
        );

        let mut interval =
            tokio::time::interval(tokio::time::Duration::from_secs(MARKET_SCAN_INTERVAL_SECS));
        let mut scan_count = 0u64;

        // Wait for initial WebSocket connections to establish
        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;

        loop {
            interval.tick().await;
            scan_count += 1;

            info!("🔄 Starting periodic market scan #{}...", scan_count);

            // Scan for new markets
            let scan_result = {
                let mut service = state_for_scanner.service.write().await;
                service.scan_for_new_markets().await
            };

            match scan_result {
                Ok((new_markets, sub_info)) => {
                    if new_markets.is_empty() {
                        info!(
                            "✅ Periodic scan #{} complete; no new markets found",
                            scan_count
                        );
                    } else {
                        info!(
                            "🆕 Found {} new matching markets; starting hot subscription...",
                            new_markets.len()
                        );

                        // Update WebSocketManager with new markets
                        {
                            let service = state_for_scanner.service.read().await;
                            let added = service.ws_manager.add_matched_markets(
                                new_markets.clone(),
                                sub_info.market_lookup.clone(),
                            );
                            info!("📊 Added {} markets to the WebSocket manager", added);
                        }

                        // Hot subscribe to new markets
                        let kalshi_success = {
                            let service = state_for_scanner.service.read().await;
                            if !sub_info.kalshi_tickers.is_empty() {
                                match service
                                    .kalshi_client
                                    .subscribe_markets(sub_info.kalshi_tickers.clone())
                                    .await
                                {
                                    Ok(success) => success,
                                    Err(e) => {
                                        error!("❌ Kalshi hot-subscription failed: {}", e);
                                        false
                                    }
                                }
                            } else {
                                true
                            }
                        };

                        if kalshi_success {
                            let service = state_for_scanner.service.read().await;
                            let total_markets =
                                service.ws_manager.get_matched_markets_for_frontend().len();
                            info!("✅ Hot subscription succeeded; {} matched markets currently active", total_markets);

                            // Broadcast scan stats to frontend via WebSocket
                            let scan_stats = crate::models::ScanStats {
                                scan_count,
                                new_markets_found: new_markets.len(),
                                total_matched_markets: total_markets,
                                timestamp: Utc::now(),
                            };
                            service.ws_manager.broadcast_scan_stats(scan_stats);
                            info!("📡 Broadcast scan stats to the frontend");
                        } else {
                            warn!("⚠️ Part of the hot subscription failed");
                        }
                    }
                }
                Err(e) => {
                    error!("❌ Market scan failed: {}", e);
                }
            }
        }
    });

    // Spawn auto-trade queue checker (every 200ms for faster response)
    let state_for_queue = state.clone();
    let metrics_for_queue = metrics.clone();
    tokio::spawn(async move {
        info!("📋 Auto-trade queue check task started, interval 200ms");

        let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(200));

        loop {
            interval.tick().await;

            // Check and add opportunities to queue
            check_and_queue_auto_trade(&state_for_queue, &metrics_for_queue).await;
        }
    });

    // Spawn auto-trade executor (processes queue with 1s interval)
    let state_for_executor = state.clone();
    let metrics_for_executor = metrics.clone();
    tokio::spawn(async move {
        info!("🚀 Auto-trade executor started, opportunity interval 1 second");

        loop {
            let service = state_for_executor.service.read().await;

            // Check if already executing
            if service
                .ws_manager
                .is_auto_trading
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                drop(service);
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                continue;
            }

            // Get next opportunity from queue
            let key = {
                let mut queue = service.ws_manager.auto_trade_queue.write();
                queue.pop_front()
            };

            if let Some(key) = key {
                // Mark as executing
                service
                    .ws_manager
                    .is_auto_trading
                    .store(true, std::sync::atomic::Ordering::Relaxed);

                info!("🎯 [Auto-trade executor] Starting processing: {}", key);

                // Execute the opportunity
                execute_single_auto_trade(
                    &service,
                    &state_for_executor,
                    &metrics_for_executor,
                    &key,
                )
                .await;

                // Mark as done
                service
                    .ws_manager
                    .is_auto_trading
                    .store(false, std::sync::atomic::Ordering::Relaxed);

                drop(service);

                info!("⏱️  [Auto-trade executor] Waiting 1 second before processing the next opportunity");
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            } else {
                // Queue empty, wait a bit
                drop(service);
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        }
    });

    // Spawn ended market cleanup task (every 60 seconds)
    let state_for_cleanup = state.clone();
    tokio::spawn(async move {
        info!("🧹 Ended-game cleanup task started, interval 60 seconds");

        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));

        // Wait for initial data to be populated
        tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;

        loop {
            interval.tick().await;

            // Check and clean up ended markets
            cleanup_ended_markets(&state_for_cleanup).await;
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
        .route(
            "/api/auto-trade/excluded",
            get(routes::get_excluded_markets),
        )
        .route("/api/auto-trade/exclude", post(routes::exclude_market))
        .route("/api/auto-trade/unexclude", post(routes::unexclude_market))
        .route("/api/auto-trade/queue", get(routes::get_auto_trade_queue))
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
        .layer(TraceLayer::new_for_http())
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

/// Calculate the number of contracts to trade based on depth and settings
///
/// Returns None if depth is insufficient (below min_contracts)
fn calculate_contracts_to_trade(
    kalshi_depth: i32,
    poly_depth_contracts: i32,
    flexible_mode: bool,
    min_contracts: i32,
    max_contracts: i32,
) -> Option<i32> {
    let min_depth = kalshi_depth.min(poly_depth_contracts);

    // Check minimum depth requirement
    if min_depth < min_contracts {
        return None;
    }

    // Fixed mode: always use min_contracts
    if !flexible_mode {
        return Some(min_contracts);
    }

    // Flexible mode logic:
    // - Depth 10-20: use min_contracts (10)
    // - Depth >= 20: use half of the smaller depth
    let contracts = if min_depth < 20 {
        min_contracts
    } else {
        min_depth / 2
    };

    // Apply max_contracts limit
    Some(contracts.min(max_contracts))
}

#[derive(Debug)]
struct RecoveryResult {
    filled_contracts: i32,
    order_id: Option<String>,
    error: Option<String>,
}

fn failed_poly_order(error: String) -> PolymarketOrderResult {
    PolymarketOrderResult {
        accepted: false,
        filled_contracts: 0,
        fill_quantity_valid: true,
        order_id: None,
        status: None,
        error: Some(error),
        latency_ms: None,
        average_fill_price: None,
    }
}

fn failed_kalshi_order(error: String) -> KalshiOrderResult {
    KalshiOrderResult {
        accepted: false,
        filled_contracts: 0,
        fill_quantity_valid: true,
        order_id: None,
        status: None,
        error: Some(error),
        average_fill_price: None,
    }
}

fn kalshi_buy_levels(book: &crate::clients::kalshi::OrderBook, side: &str) -> Vec<(i32, i32)> {
    let mut levels = match side {
        "yes" => book.no.iter().rev().map(|(price, qty)| (100 - price, *qty)).collect(),
        "no" => book.yes.iter().rev().map(|(price, qty)| (100 - price, *qty)).collect(),
        _ => Vec::new(),
    };
    levels.sort_by_key(|(price, _)| *price);
    levels
}

fn kalshi_sell_levels(book: &crate::clients::kalshi::OrderBook, side: &str) -> Vec<(i32, i32)> {
    let mut levels = match side {
        "yes" => book.yes.iter().rev().map(|(price, qty)| (*price, *qty)).collect(),
        "no" => book.no.iter().rev().map(|(price, qty)| (*price, *qty)).collect(),
        _ => Vec::new(),
    };
    levels.sort_by(|left, right| right.0.cmp(&left.0));
    levels
}

fn polymarket_buy_levels(
    book: &PolymarketMarketBook,
    position_side: PolymarketPositionSide,
) -> Vec<(f64, i32)> {
    let levels = match position_side {
        // Long buys consume native LONG offers in ascending price order.
        PolymarketPositionSide::Long => &book.offers,
        // Short buys consume native LONG bids descending, at complement price.
        PolymarketPositionSide::Short => &book.bids,
    };
    let iter: Box<dyn Iterator<Item = &PolymarketBookLevel>> = match position_side {
        PolymarketPositionSide::Long => Box::new(levels.iter()),
        PolymarketPositionSide::Short => Box::new(levels.iter().rev()),
    };
    let mut result: Vec<_> = iter.filter_map(|level| {
        let quantity = level.quantity.floor() as i32;
        (quantity > 0).then_some((
            if position_side == PolymarketPositionSide::Long {
                level.price
            } else {
                1.0 - level.price
            },
            quantity,
        ))
    })
    .collect();
    result.sort_by(|left, right| left.0.total_cmp(&right.0));
    result
}

fn polymarket_sell_levels(
    book: &PolymarketMarketBook,
    position_side: PolymarketPositionSide,
) -> Vec<(f64, i32)> {
    let levels = match position_side {
        // Selling LONG consumes native LONG bids descending.
        PolymarketPositionSide::Long => &book.bids,
        // Selling SHORT consumes native LONG offers ascending, complemented.
        PolymarketPositionSide::Short => &book.offers,
    };
    let iter: Box<dyn Iterator<Item = &PolymarketBookLevel>> = match position_side {
        PolymarketPositionSide::Long => Box::new(levels.iter()),
        PolymarketPositionSide::Short => Box::new(levels.iter()),
    };
    let mut result: Vec<_> = iter.filter_map(|level| {
        let quantity = level.quantity.floor() as i32;
        (quantity > 0).then_some((
            if position_side == PolymarketPositionSide::Long {
                level.price
            } else {
                1.0 - level.price
            },
            quantity,
        ))
    })
    .collect();
    result.sort_by(|left, right| right.0.total_cmp(&left.0));
    result
}

fn executable_depth<T>(levels: &[(T, i32)]) -> i32 {
    levels.iter().fold(0_i32, |total, (_, quantity)| total.saturating_add(*quantity))
}

fn worst_price<T: Copy>(levels: &[(T, i32)], contracts: i32) -> Option<T> {
    let mut remaining = contracts;
    for (price, quantity) in levels {
        if *quantity <= 0 {
            continue;
        }
        remaining -= (*quantity).min(remaining);
        if remaining <= 0 {
            return Some(*price);
        }
    }
    None
}

fn kalshi_fee(contracts: i32, price: f64) -> f64 {
    (0.07 * contracts as f64 * price * (1.0 - price) * 100.0).ceil() / 100.0
}

fn find_profitable_contract_size(
    size_cap: i32,
    min_contracts: i32,
    max_amount: f64,
    kalshi_levels: &[(i32, i32)],
    poly_levels: &[(f64, i32)],
) -> Option<(i32, i32, f64, f64, f64)> {
    if !max_amount.is_finite() || max_amount <= 0.0 {
        return None;
    }
    for contracts in (min_contracts..=size_cap).rev() {
        let kalshi_price_cents = worst_price(kalshi_levels, contracts)?;
        let poly_price = worst_price(poly_levels, contracts)?;
        let kalshi_price = kalshi_price_cents as f64 / 100.0;
        let fee = kalshi_fee(contracts, kalshi_price);
        let total_cost = contracts as f64 * (kalshi_price + poly_price) + fee;
        if total_cost.is_finite() && total_cost <= max_amount && total_cost < contracts as f64 {
            let margin = ((contracts as f64 - total_cost) / total_cost) * 100.0;
            return Some((contracts, kalshi_price_cents, poly_price, fee, margin));
        }
    }
    None
}

async fn neutralize_polymarket(
    service: &ArbitrageService,
    market_slug: &str,
    position_side: PolymarketPositionSide,
    contracts: i32,
    entry_price: Option<f64>,
    max_loss_cents: i32,
) -> Result<RecoveryResult, String> {
    let entry_price = entry_price.ok_or_else(|| {
        "Polymarket actual entry price was not returned; bounded close cannot be verified".to_string()
    })?;
    let book = service.polymarket_client.get_market_book(market_slug).await
        .map_err(|error| format!("Unable to fetch fresh Polymarket close book: {error}"))?;
    let close_price = worst_price(&polymarket_sell_levels(&book, position_side), contracts)
        .ok_or_else(|| "Fresh Polymarket close book lacks required executable depth".to_string())?;
    if close_price + f64::EPSILON < entry_price - max_loss_cents as f64 / 100.0 {
        return Err(format!(
            "Polymarket close price {:.4} exceeds configured {}¢ loss bound from actual entry {:.4}",
            close_price, max_loss_cents, entry_price
        ));
    }
    let result = service.polymarket_client.submit_limit_order(
        market_slug, position_side, "sell", contracts, close_price
    ).await.map_err(|error| format!("Polymarket bounded close submission failed: {error}"))?;
    Ok(RecoveryResult {
        filled_contracts: result.filled_contracts,
        order_id: result.order_id,
        error: result.error,
    })
}

async fn neutralize_kalshi(
    service: &ArbitrageService,
    ticker: &str,
    side: &str,
    contracts: i32,
    entry_price: Option<f64>,
    max_loss_cents: i32,
) -> Result<RecoveryResult, String> {
    let entry_price = entry_price.ok_or_else(|| {
        "Kalshi actual entry price was not returned; bounded close cannot be verified".to_string()
    })?;
    let book = service.kalshi_client.get_fresh_orderbook(ticker, EXECUTABLE_BOOK_MAX_AGE)
        .ok_or_else(|| "Fresh Kalshi websocket close depth is unavailable".to_string())?;
    let close_price = worst_price(&kalshi_sell_levels(&book, side), contracts)
        .ok_or_else(|| "Fresh Kalshi close book lacks required executable depth".to_string())?;
    let close_price_dollars = close_price as f64 / 100.0;
    if close_price_dollars + f64::EPSILON < entry_price - max_loss_cents as f64 / 100.0 {
        return Err(format!(
            "Kalshi close price {:.4} exceeds configured {}¢ loss bound from actual entry {:.4}",
            close_price_dollars, max_loss_cents, entry_price
        ));
    }
    let result = service.kalshi_client.submit_order(ticker, "sell", side, contracts, close_price)
        .await.map_err(|error| format!("Kalshi bounded close submission failed: {error}"))?;
    Ok(RecoveryResult {
        filled_contracts: result.filled_contracts,
        order_id: result.order_id,
        error: result.error,
    })
}

fn save_auto_trade_skip(
    service: &ArbitrageService,
    key: &str,
    record: &crate::models::ArbitrageTrackingRecord,
    opportunity: &crate::models::ArbitrageOpportunity,
    contracts: i32,
    duration_ms: i64,
    reason: &str,
) {
    info!("⚠️ [Auto-trade] {}: {} - {}", reason, record.event_name, record.team_name);
    if service.ws_manager.should_record_skip(key, reason) {
        if let Err(error) = service.ws_manager.get_storage().save_skipped_auto_trade_record(
            &record.event_name, &record.team_name, &record.kalshi_market_id,
            &record.polymarket_market_id, &opportunity.kalshi_side, &opportunity.polymarket_side,
            contracts, opportunity.kalshi_price, opportunity.polymarket_price,
            opportunity.profit_margin, duration_ms, reason,
        ) {
            error!("Failed to save skipped auto-trade record: {}", error);
        }
    }
}

/// Check and add eligible opportunities to auto-trade queue
async fn check_and_queue_auto_trade(state: &Arc<AppState>, _metrics: &Arc<PerformanceMetrics>) {
    let service = state.service.read().await;

    // Get auto-trade state
    let auto_state = service.ws_manager.get_auto_trade_state();

    // Early return if not enabled or already at limit
    if !auto_state.enabled {
        return;
    }

    if auto_state.trade_count >= auto_state.max_trade_count {
        return;
    }

    // Get active tracking records
    let tracking_records = service.ws_manager.get_active_tracking_for_auto_trade();

    // Add eligible opportunities to queue
    for (key, _record, duration_ms) in tracking_records {
        // Check eligibility
        let (eligible, _reason) = service
            .ws_manager
            .check_auto_trade_eligibility(&key, duration_ms);

        if !eligible {
            continue;
        }

        // Add to queue if not already queued
        let mut queue = service.ws_manager.auto_trade_queue.write();
        if !queue.contains(&key) {
            queue.push_back(key.clone());
            info!("📋 [Auto-trade queue] Added opportunity: {}", key);
        }
    }
}

/// Execute auto-trade for a single opportunity (extracted from original for loop)
async fn execute_single_auto_trade(
    service: &ArbitrageService,
    state: &Arc<AppState>,
    _metrics: &Arc<PerformanceMetrics>,
    key: &str,
) {
    let (record, duration_ms) = match service.ws_manager.get_tracking_record(key) {
        Some((record, duration_ms)) => (record, duration_ms),
        None => {
            info!("⚠️ [Auto-trade] Opportunity no longer exists: {}", key);
            return;
        }
    };

    let auto_state = service.ws_manager.get_auto_trade_state();
    let (eligible, reason) = service
        .ws_manager
        .check_auto_trade_eligibility(key, duration_ms);
    if !eligible {
        info!(
            "⚠️ [Auto-trade] Opportunity no longer meets conditions: {} - {}",
            key, reason
        );
        return;
    }

    let opportunity = match service.ws_manager.get_opportunity_by_key(key) {
        Some(opportunity) => opportunity,
        None => {
            info!("⚠️ [Auto-trade] Opportunity data does not exist: {}", key);
            return;
        }
    };

    let Some(matched_market) = service
        .get_matched_markets()
        .iter()
        .find(|market| market.market_key() == key)
    else {
        save_auto_trade_skip(service, key, &record, &opportunity, auto_state.min_contracts,
            duration_ms, "Matched market disappeared before execution");
        return;
    };
    let Some(poly_execution) = matched_market
        .polymarket_market
        .us_execution_for_competitor(&record.team_name)
    else {
        save_auto_trade_skip(service, key, &record, &opportunity, auto_state.min_contracts,
            duration_ms, "Polymarket US position side is unavailable for the matched competitor");
        return;
    };

    let Some(kalshi_book) = service.kalshi_client.get_fresh_orderbook(
        &record.kalshi_market_id,
        EXECUTABLE_BOOK_MAX_AGE,
    ) else {
        save_auto_trade_skip(service, key, &record, &opportunity, auto_state.min_contracts,
            duration_ms, "Fresh Kalshi websocket executable depth is unavailable");
        return;
    };
    let poly_book = match service
        .polymarket_client
        .get_market_book(&poly_execution.market_slug)
        .await
    {
        Ok(book) => book,
        Err(reason) => {
            save_auto_trade_skip(service, key, &record, &opportunity, auto_state.min_contracts,
                duration_ms, &format!("Fresh Polymarket US executable book is unavailable: {reason}"));
            return;
        }
    };

    let kalshi_levels = kalshi_buy_levels(&kalshi_book, &opportunity.kalshi_side);
    let poly_levels = polymarket_buy_levels(&poly_book, poly_execution.position_side);
    let max_depth = executable_depth(&kalshi_levels).min(executable_depth(&poly_levels));
    let Some(size_cap) = calculate_contracts_to_trade(
        max_depth,
        max_depth,
        auto_state.flexible_mode,
        auto_state.min_contracts,
        auto_state.max_contracts,
    ) else {
        save_auto_trade_skip(service, key, &record, &opportunity, auto_state.min_contracts,
            duration_ms, "Fresh executable depth is below the configured contract minimum");
        return;
    };

    let Some((contracts, kalshi_price_cents, poly_price, kalshi_fee, profit_margin)) =
        find_profitable_contract_size(
            size_cap,
            auto_state.min_contracts,
            auto_state.max_amount,
            &kalshi_levels,
            &poly_levels,
        )
    else {
        save_auto_trade_skip(service, key, &record, &opportunity, size_cap, duration_ms,
            "No fresh executable size remains profitable after fees, worst-case prices, and amount limit");
        return;
    };

    let started = Instant::now();
    let lifecycle_id = service.ws_manager.get_storage().save_auto_trade_execution(
        &AutoTradeExecutionRecord {
            event_name: record.event_name.clone(),
            team_name: record.team_name.clone(),
            kalshi_market_id: record.kalshi_market_id.clone(),
            polymarket_market_id: poly_execution.market_slug.clone(),
            kalshi_side: opportunity.kalshi_side.clone(),
            polymarket_side: poly_execution.position_side.as_str().to_string(),
            contracts,
            kalshi_price: kalshi_price_cents as f64 / 100.0,
            kalshi_fee,
            polymarket_price: poly_price,
            profit_margin,
            duration_ms,
            total_duration_ms: 0,
            kalshi_success: false,
            polymarket_success: false,
            kalshi_order_id: None,
            polymarket_order_id: None,
            kalshi_error: None,
            polymarket_error: None,
            kalshi_latency_ms: None,
            poly_latency_ms: None,
            kalshi_filled_contracts: 0,
            polymarket_filled_contracts: 0,
            kalshi_order_status: None,
            polymarket_order_status: None,
            neutralization_leg: None,
            neutralization_success: None,
            neutralization_order_id: None,
            neutralization_filled_contracts: 0,
            neutralization_error: None,
            residual_leg: None,
            residual_contracts: 0,
            status: "submitting".to_string(),
        },
    ).map_err(|reason| {
        error!("Failed to persist auto-trade submission before order placement: {}", reason);
        reason
    }).ok();
    // Every input below came from the direct executable books above. Both
    // native orders are IOC/FAK; an acknowledgement with zero fill is failure.
    let poly_started = Instant::now();
    let poly_result = match service
        .polymarket_client
        .submit_limit_order(
            &poly_execution.market_slug,
            poly_execution.position_side,
            "buy",
            contracts,
            poly_price,
        )
        .await
    {
        Ok(result) => result,
        Err(reason) => failed_poly_order(reason.to_string()),
    };
    let poly_latency = poly_result
        .latency_ms
        .or(Some(poly_started.elapsed().as_millis() as i64));

    let kalshi_started = Instant::now();
    let kalshi_result = match service
        .kalshi_client
        .submit_order(
            &record.kalshi_market_id,
            "buy",
            &opportunity.kalshi_side,
            contracts,
            kalshi_price_cents,
        )
        .await
    {
        Ok(result) => result,
        Err(reason) => failed_kalshi_order(reason.to_string()),
    };
    let kalshi_latency = Some(kalshi_started.elapsed().as_millis() as i64);

    let mut neutralization_leg = None;
    let mut neutralization_success = None;
    let mut neutralization_order_id = None;
    let mut neutralization_filled_contracts = 0;
    let mut neutralization_error = None;
    let mut residual_leg = None;
    let mut residual_contracts = 0;
    let unsafe_fill_quantity = !poly_result.fill_quantity_valid || !kalshi_result.fill_quantity_valid;

    if unsafe_fill_quantity {
        residual_leg = Some("unknown".to_string());
        neutralization_error = Some(
            "An exchange reported a non-whole fill quantity; exact bounded neutralization is unsafe"
                .to_string(),
        );
    } else if poly_result.filled_contracts != kalshi_result.filled_contracts {
        let (leg, excess) = if poly_result.filled_contracts > kalshi_result.filled_contracts {
            ("polymarket", poly_result.filled_contracts - kalshi_result.filled_contracts)
        } else {
            ("kalshi", kalshi_result.filled_contracts - poly_result.filled_contracts)
        };
        neutralization_leg = Some(leg.to_string());
        let recovery = if leg == "polymarket" {
            neutralize_polymarket(
                service,
                &poly_execution.market_slug,
                poly_execution.position_side,
                excess,
                poly_result.average_fill_price,
                auto_state.neutralization_max_loss_cents,
            )
            .await
        } else {
            neutralize_kalshi(
                service,
                &record.kalshi_market_id,
                &opportunity.kalshi_side,
                excess,
                kalshi_result.average_fill_price,
                auto_state.neutralization_max_loss_cents,
            )
            .await
        };
        match recovery {
            Ok(recovery) if recovery.filled_contracts >= excess => {
                neutralization_success = Some(true);
                neutralization_order_id = recovery.order_id;
                neutralization_filled_contracts = recovery.filled_contracts;
            }
            Ok(recovery) => {
                neutralization_success = Some(false);
                neutralization_order_id = recovery.order_id;
                neutralization_filled_contracts = recovery.filled_contracts;
                neutralization_error = recovery.error;
                residual_leg = Some(leg.to_string());
                residual_contracts = excess - neutralization_filled_contracts;
            }
            Err(reason) => {
                neutralization_success = Some(false);
                neutralization_error = Some(reason);
                residual_leg = Some(leg.to_string());
                residual_contracts = excess;
            }
        }
    }

    let status = if unsafe_fill_quantity || residual_contracts > 0 {
        "exposed"
    } else if neutralization_leg.is_some() {
        "neutralized"
    } else if poly_result.filled_contracts == contracts && kalshi_result.filled_contracts == contracts {
        "executed"
    } else if poly_result.filled_contracts == 0 && kalshi_result.filled_contracts == 0 {
        "rejected"
    } else {
        // Equal partial fills can still be directionally paired, but are
        // recorded explicitly rather than presented as a full execution.
        "partial_paired"
    };
    let execution_record = AutoTradeExecutionRecord {
        event_name: record.event_name.clone(),
        team_name: record.team_name.clone(),
        kalshi_market_id: record.kalshi_market_id.clone(),
        polymarket_market_id: poly_execution.market_slug.clone(),
        kalshi_side: opportunity.kalshi_side.clone(),
        polymarket_side: poly_execution.position_side.as_str().to_string(),
        contracts,
        kalshi_price: kalshi_price_cents as f64 / 100.0,
        kalshi_fee,
        polymarket_price: poly_price,
        profit_margin,
        duration_ms,
        total_duration_ms: started.elapsed().as_millis() as i64,
        kalshi_success: kalshi_result.filled_contracts > 0,
        polymarket_success: poly_result.filled_contracts > 0,
        kalshi_order_id: kalshi_result.order_id.clone(),
        polymarket_order_id: poly_result.order_id.clone(),
        kalshi_error: kalshi_result.error.clone(),
        polymarket_error: poly_result.error.clone(),
        kalshi_latency_ms: kalshi_latency,
        poly_latency_ms: poly_latency,
        kalshi_filled_contracts: kalshi_result.filled_contracts,
        polymarket_filled_contracts: poly_result.filled_contracts,
        kalshi_order_status: kalshi_result.status.clone(),
        polymarket_order_status: poly_result.status.clone(),
        neutralization_leg,
        neutralization_success,
        neutralization_order_id,
        neutralization_filled_contracts,
        neutralization_error: neutralization_error.clone(),
        residual_leg: residual_leg.clone(),
        residual_contracts,
        status: status.to_string(),
    };
    if let Some(lifecycle_id) = lifecycle_id {
        if let Err(reason) = service.ws_manager.get_storage().finish_auto_trade_execution(lifecycle_id, &execution_record) {
            error!("Failed to finalize automatic paired execution: {}", reason);
        }
    } else if let Err(reason) = service.ws_manager.get_storage().save_auto_trade_execution(&execution_record) {
        error!("Failed to persist automatic paired execution: {}", reason);
    }

    if unsafe_fill_quantity || residual_contracts > 0 {
        // The recovery bound was unavailable or could not be met. Stop future
        // automated submissions and notify through the configured alert path.
        if let Err(reason) = service.ws_manager.disable_auto_trade() {
            error!("Failed to halt auto-trading after residual exposure: {}", reason);
        }
        state.telegram_client.send_auto_trade_notification(
            &record.event_name, &record.team_name, profit_margin,
            execution_record.kalshi_success, execution_record.polymarket_success,
            execution_record.kalshi_error.as_deref(), execution_record.polymarket_error.as_deref(),
            contracts as f64 * (kalshi_price_cents as f64 / 100.0 + poly_price) + kalshi_fee,
            0.0,
        ).await;
    } else {
        service.ws_manager.mark_as_auto_traded(key);
    }
    if status == "executed" {
        if let Err(reason) = service.ws_manager.increment_trade_count() {
            error!("Failed to increment auto-trade count: {}", reason);
        }
    }
}

/// Clean up ended markets by unsubscribing from WebSocket feeds
///
/// This function:
/// 1. Checks all markets for extreme prices (Kalshi 99/2, Poly 100/0)
/// 2. If extreme prices persist for 20+ minutes, marks market as ended
/// 3. Unsubscribes from WebSocket feeds for ended markets
/// 4. Removes ended markets from internal caches
async fn cleanup_ended_markets(state: &Arc<AppState>) {
    let service = state.service.read().await;

    // Get detection counts for logging
    let detecting_count = service.ws_manager.get_ending_detection_count();
    let ended_count = service.ws_manager.get_confirmed_ended_count();

    if detecting_count > 0 {
        info!(
            "🔍 [Cleanup] Monitoring {} potentially ended markets, confirmed {}",
            detecting_count, ended_count
        );
    }

    // Remove ended markets and get Kalshi subscriptions to unsubscribe.
    let kalshi_to_unsub = service.ws_manager.remove_ended_markets();

    if kalshi_to_unsub.is_empty() {
        return;
    }

    // Unsubscribe from Kalshi markets
    if !kalshi_to_unsub.is_empty() {
        match service
            .kalshi_client
            .unsubscribe_markets(kalshi_to_unsub.clone())
            .await
        {
            Ok(success) => {
                if success {
                    info!(
                        "✅ [Cleanup] Kalshi unsubscribe succeeded: {} markets",
                        kalshi_to_unsub.len()
                    );
                } else {
                    warn!("⚠️ [Cleanup] Kalshi unsubscribe partially failed");
                }
            }
            Err(e) => {
                error!("❌ [Cleanup] Kalshi unsubscribe failed: {}", e);
            }
        }
    }

    // Log summary
    let remaining_markets = service.ws_manager.get_matched_markets_for_frontend().len();
    info!(
        "✅ [Cleanup] Complete; {} active markets remain",
        remaining_markets
    );
}

#[cfg(test)]
mod auto_execution_tests {
    use super::*;

    fn native_book() -> PolymarketMarketBook {
        PolymarketMarketBook {
            success: true,
            market_slug: "match-winner".to_string(),
            state: "open".to_string(),
            transact_time: Some("2026-09-11T19:37:00Z".to_string()),
            fetched_at_ms: 1,
            bids: vec![
                PolymarketBookLevel { price: 0.62, quantity: 2.0 },
                PolymarketBookLevel { price: 0.60, quantity: 3.0 },
            ],
            offers: vec![
                PolymarketBookLevel { price: 0.64, quantity: 2.0 },
                PolymarketBookLevel { price: 0.66, quantity: 3.0 },
            ],
        }
    }

    #[test]
    fn derives_short_buy_depth_from_descending_native_bids() {
        let levels = polymarket_buy_levels(&native_book(), PolymarketPositionSide::Short);
        assert_eq!(levels, vec![(0.38, 2), (0.40, 3)]);
        assert_eq!(worst_price(&levels, 3), Some(0.40));
    }

    #[test]
    fn rejects_size_when_worst_case_fee_removes_profit() {
        let kalshi = vec![(50, 10)];
        let poly = vec![(0.50, 10)];
        assert!(find_profitable_contract_size(10, 1, 100.0, &kalshi, &poly).is_none());
    }

    #[test]
    fn uses_worst_level_and_respects_total_amount_limit() {
        let kalshi = vec![(40, 2), (42, 3)];
        let poly = vec![(0.45, 2), (0.47, 3)];
        let result = find_profitable_contract_size(5, 1, 5.0, &kalshi, &poly).unwrap();
        assert_eq!(result.0, 5);
        assert_eq!(result.1, 42);
        assert_eq!(result.2, 0.47);
    }
}
