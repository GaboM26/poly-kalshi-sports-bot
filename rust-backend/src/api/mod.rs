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
    routing::{get, post, delete, put},
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

    // Spawn auto-trade checker (every 1 second)
    let state_for_auto_trade = state.clone();
    tokio::spawn(async move {
        info!("🤖 自动下单检查任务启动，间隔 1 秒");
        
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
        
        loop {
            interval.tick().await;
            
            // Check and execute auto-trade
            check_and_execute_auto_trade(&state_for_auto_trade).await;
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
        // Auto-trade
        .route("/api/auto-trade/status", get(routes::get_auto_trade_status))
        .route("/api/auto-trade/enable", post(routes::enable_auto_trade))
        .route("/api/auto-trade/disable", post(routes::disable_auto_trade))
        .route("/api/auto-trade/reset", post(routes::reset_auto_trade))
        .route("/api/auto-trade/settings", put(routes::update_auto_trade_settings))
        .route("/api/auto-trade/history", get(routes::get_auto_trade_history))
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

/// Check and execute auto-trade for eligible opportunities
async fn check_and_execute_auto_trade(state: &Arc<AppState>) {
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
    
    for (key, record, duration_ms) in tracking_records {
        // Check eligibility
        let (eligible, reason) = service.ws_manager.check_auto_trade_eligibility(&key, duration_ms);
        
        if !eligible {
            continue;
        }
        
        // Get current opportunity data
        let opportunity = match service.ws_manager.get_opportunity_by_key(&key) {
            Some(opp) => opp,
            None => continue,
        };
        
        info!("🤖 [自动下单] 检测到符合条件的套利机会:");
        info!("   事件: {} - {}", record.event_name, record.team_name);
        info!("   持续时间: {}ms, 利润率: {:.2}%", duration_ms, opportunity.profit_margin);
        info!("   原因: {}", reason);
        
        // Fixed contract count strategy: both platforms buy same number of contracts
        // This ensures symmetric hedging and simple calculation
        let fixed_contracts = 10;  // Fixed 10 contracts on each platform
        
        // Calculate amounts based on fixed contracts
        let kalshi_contracts = fixed_contracts;
        let kalshi_bet = fixed_contracts as f64 * opportunity.kalshi_price;
        let kalshi_price_cents = (opportunity.kalshi_price * 100.0).round() as i32;
        
        // Calculate Kalshi trading fee: fee = 0.07 × C × P × (1-P), rounded up to cent
        let kalshi_fee_raw = 0.07 * kalshi_contracts as f64 * opportunity.kalshi_price * (1.0 - opportunity.kalshi_price);
        let kalshi_fee = (kalshi_fee_raw * 100.0).ceil() / 100.0;
        
        // Polymarket: same number of "contracts" (equivalent amount), no trading fee
        let poly_amount = fixed_contracts as f64 * opportunity.polymarket_price;
        
        // Calculate total investment including fees
        let total_bet = kalshi_bet + kalshi_fee + poly_amount;
        
        // Verify total doesn't exceed max_amount
        if total_bet > auto_state.max_amount {
            info!("   ⚠️ 总投入 ${:.2} (含手续费 ${:.2}) 超过限额 ${:.2}，跳过此机会", total_bet, kalshi_fee, auto_state.max_amount);
            continue;
        }
        
        // Get the correct poly token based on side
        let poly_token = match service.ws_manager.get_poly_token_for_side(
            &record.event_name,
            &record.team_name,
            &opportunity.polymarket_side,
        ) {
            Some(token) => token,
            None => {
                error!("❌ 无法获取 Polymarket token: {} - {} ({}侧)", 
                    record.event_name, record.team_name, opportunity.polymarket_side);
                continue;
            }
        };
        
        info!("   📊 订单计算 (固定 {} 合约):", fixed_contracts);
        info!("      Kalshi: {} 合约 @ {:.2}¢ = ${:.2} + 手续费 ${:.2} ({}侧)", kalshi_contracts, kalshi_price_cents, kalshi_bet, kalshi_fee, opportunity.kalshi_side);
        info!("      Polymarket: {} 合约等价 = ${:.4} ({}侧)", fixed_contracts, poly_amount, opportunity.polymarket_side);
        info!("      总投入: ${:.2} (含手续费) / ${:.2}", total_bet, auto_state.max_amount);
        
        // Mark as auto-traded BEFORE executing (to prevent duplicate orders)
        service.ws_manager.mark_as_auto_traded(&key);
        
        // Execute Kalshi order
        let kalshi_result = service.kalshi_client.place_order(
            &record.kalshi_market_id,
            "buy",
            &opportunity.kalshi_side,
            kalshi_contracts,
            kalshi_price_cents,
        ).await;
        
        // Execute Polymarket order
        let poly_result = service.polymarket_client.place_market_order(
            &poly_token,
            "buy",
            poly_amount,
        ).await;
        
        // Extract results for logging and recording
        let kalshi_success = kalshi_result.is_ok();
        let poly_success = poly_result.is_ok();
        
        // Extract order IDs and errors
        let (kalshi_order_id, kalshi_error) = match &kalshi_result {
            Ok(res) => (res.get("order_id").and_then(|v| v.as_str()).map(|s| s.to_string()), None),
            Err(e) => (None, Some(e.to_string())),
        };
        
        let (poly_order_id, poly_error) = match &poly_result {
            Ok(res) => (res.get("order_id").and_then(|v| v.as_str()).map(|s| s.to_string()), None),
            Err(e) => (None, Some(e.to_string())),
        };
        
        // Save trade record to database
        let storage = service.ws_manager.get_storage();
        if let Err(e) = storage.save_auto_trade_record(
            &record.event_name,
            &record.team_name,
            &record.kalshi_market_id,
            &record.polymarket_market_id,
            &opportunity.kalshi_side,
            &opportunity.polymarket_side,
            kalshi_contracts,
            opportunity.kalshi_price,
            kalshi_fee,
            poly_amount,
            opportunity.polymarket_price,
            total_bet,
            opportunity.profit_margin,
            duration_ms,
            kalshi_success,
            poly_success,
            kalshi_order_id.as_deref(),
            poly_order_id.as_deref(),
            kalshi_error.as_deref(),
            poly_error.as_deref(),
        ) {
            error!("保存自动下单记录失败: {}", e);
        }
        
        // Log results
        if kalshi_success && poly_success {
            // Increment trade count on success
            if let Ok(new_count) = service.ws_manager.increment_trade_count() {
                info!("✅ [自动下单] 成功! 已执行 {}/{} 次", new_count, auto_state.max_trade_count);
            }
            info!("   Kalshi: {:?}", kalshi_result.unwrap());
            info!("   Polymarket: {:?}", poly_result.unwrap());
        } else {
            // Still increment count even on partial failure to track attempts
            if let Ok(new_count) = service.ws_manager.increment_trade_count() {
                info!("⚠️ [自动下单] 部分成功，已记录 {}/{} 次", new_count, auto_state.max_trade_count);
            }
            error!("❌ [自动下单] 部分失败:");
            if let Some(err) = &kalshi_error {
                error!("   Kalshi 失败: {}", err);
            }
            if let Some(err) = &poly_error {
                error!("   Polymarket 失败: {}", err);
            }
        }
        
        // Only process one opportunity per cycle to avoid overwhelming the system
        break;
    }
}
