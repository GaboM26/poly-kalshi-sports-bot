//! API Layer
//!
//! HTTP routes and WebSocket server for the frontend.

pub mod routes;
pub mod websocket;
pub mod static_files;

use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use axum::{
    routing::{get, post, delete},
    Router,
};
use chrono::Utc;
use tokio::sync::{mpsc, RwLock};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::{info, warn, error};

use crate::config::Config;
use crate::models::PriceUpdate;
use crate::services::{ArbitrageService, PerformanceMetrics};

/// Market scan interval in seconds (5 minutes)
const MARKET_SCAN_INTERVAL_SECS: u64 = 300;

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

    // Get metrics reference before moving service
    let metrics = service.metrics.clone();

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
        info!("🔍 市场扫描任务启动，间隔 {} 秒 ({} 分钟)", 
            MARKET_SCAN_INTERVAL_SECS, 
            MARKET_SCAN_INTERVAL_SECS / 60
        );
        
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(MARKET_SCAN_INTERVAL_SECS));
        let mut scan_count = 0u64;
        
        // Wait for initial WebSocket connections to establish
        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
        
        loop {
            interval.tick().await;
            scan_count += 1;
            
            info!("🔄 开始第 {} 次定期市场扫描...", scan_count);
            
            // Scan for new markets
            let scan_result = {
                let mut service = state_for_scanner.service.write().await;
                service.scan_for_new_markets().await
            };
            
            match scan_result {
                Ok((new_markets, sub_info)) => {
                    if new_markets.is_empty() {
                        info!("✅ 第 {} 次扫描完成，没有发现新市场", scan_count);
                    } else {
                        info!("🆕 发现 {} 个新匹配市场，开始热订阅...", new_markets.len());
                        
                        // Update WebSocketManager with new markets
                        {
                            let service = state_for_scanner.service.read().await;
                            let added = service.ws_manager.add_matched_markets(
                                new_markets.clone(),
                                sub_info.market_lookup.clone(),
                            );
                            info!("📊 已添加 {} 个市场到 WebSocket 管理器", added);
                        }
                        
                        // Hot subscribe to new markets
                        let kalshi_success = {
                            let service = state_for_scanner.service.read().await;
                            if !sub_info.kalshi_tickers.is_empty() {
                                match service.kalshi_client.subscribe_markets(sub_info.kalshi_tickers.clone()).await {
                                    Ok(success) => success,
                                    Err(e) => {
                                        error!("❌ Kalshi 热订阅失败: {}", e);
                                        false
                                    }
                                }
                            } else {
                                true
                            }
                        };
                        
                        let poly_success = {
                            let service = state_for_scanner.service.read().await;
                            if !sub_info.polymarket_token_ids.is_empty() {
                                match service.polymarket_client.subscribe_tokens(sub_info.polymarket_token_ids.clone()).await {
                                    Ok(success) => success,
                                    Err(e) => {
                                        error!("❌ Polymarket 热订阅失败: {}", e);
                                        false
                                    }
                                }
                            } else {
                                true
                            }
                        };
                        
                        if kalshi_success && poly_success {
                            let service = state_for_scanner.service.read().await;
                            let total_markets = service.ws_manager.get_matched_markets_for_frontend().len();
                            info!("✅ 热订阅成功，当前共 {} 个配对市场", total_markets);
                            
                            // Broadcast scan stats to frontend via WebSocket
                            let scan_stats = crate::models::ScanStats {
                                scan_count,
                                new_markets_found: new_markets.len(),
                                total_matched_markets: total_markets,
                                timestamp: Utc::now(),
                            };
                            service.ws_manager.broadcast_scan_stats(scan_stats);
                            info!("📡 已广播扫描统计到前端");
                        } else {
                            warn!("⚠️ 热订阅部分失败");
                        }
                    }
                }
                Err(e) => {
                    error!("❌ 市场扫描失败: {}", e);
                }
            }
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
        // Orderbook depth
        .route("/api/orderbook/depth", get(routes::get_orderbook_depth))
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

    info!("✅ API 路由配置完成（包含前端静态文件）");

    Ok(app)
}

/// Ping both APIs to measure latency
async fn ping_apis(state: &Arc<AppState>, metrics: &Arc<PerformanceMetrics>) {
    // Test Kalshi API latency
    let kalshi_start = Instant::now();
    let service = state.service.read().await;
    
    match service.kalshi_client.get_balance().await {
        Ok(_) => {
            let latency_ms = kalshi_start.elapsed().as_millis() as u64;
            metrics.set_kalshi_latency(latency_ms);
        }
        Err(e) => {
            warn!("Kalshi API ping 失败: {}", e);
        }
    }
    
    // Test Polymarket API latency
    let poly_start = Instant::now();
    match service.polymarket_client.get_balance().await {
        Ok(_) => {
            let latency_ms = poly_start.elapsed().as_millis() as u64;
            metrics.set_polymarket_latency(latency_ms);
        }
        Err(e) => {
            warn!("Polymarket API ping 失败: {}", e);
        }
    }
}
