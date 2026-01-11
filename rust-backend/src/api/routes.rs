//! HTTP API Routes

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::{Duration, Utc};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use super::AppState;

/// Health check response
#[derive(Serialize)]
pub struct HealthResponse {
    status: String,
    version: String,
}

/// Health check endpoint
pub async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// Get system statistics
pub async fn get_stats(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let service = state.service.read().await;
    let stats = service.get_stats();
    Json(stats)
}

/// Get data coverage
pub async fn get_data_coverage(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let service = state.service.read().await;
    let coverage = service.ws_manager.get_data_coverage();
    // Return the DataCoverage struct directly (it implements Serialize)
    Json(coverage)
}

/// Get current arbitrage opportunities
pub async fn get_opportunities(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let service = state.service.read().await;
    let opportunities = service.get_opportunities();
    Json(opportunities)
}

/// Get matched markets
pub async fn get_matched_markets(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let service = state.service.read().await;
    // 使用 frontend 格式，包含 poly_token_id 等字段
    let markets = service.ws_manager.get_matched_markets_for_frontend();
    Json(markets)
}

/// Query params for history
#[derive(Deserialize)]
pub struct HistoryQuery {
    limit: Option<usize>,
}

/// Get arbitrage history
pub async fn get_arbitrage_history(
    State(state): State<Arc<AppState>>,
    Query(query): Query<HistoryQuery>,
) -> impl IntoResponse {
    let service = state.service.read().await;
    let limit = query.limit.unwrap_or(100);

    match service.get_arbitrage_history(limit) {
        Ok(history) => Json(serde_json::json!({
            "success": true,
            "data": history
        }))
        .into_response(),
        Err(e) => {
            error!("获取历史记录失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "success": false,
                    "error": e.to_string()
                })),
            )
                .into_response()
        }
    }
}

/// Get Kalshi balance
pub async fn get_kalshi_balance(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let service = state.service.read().await;

    match service.kalshi_client.get_balance().await {
        Ok(balance) => Json(serde_json::json!({
            "success": true,
            "balance": balance
        }))
        .into_response(),
        Err(e) => {
            error!("获取 Kalshi 余额失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "success": false,
                    "error": e.to_string()
                })),
            )
                .into_response()
        }
    }
}

/// Get Polymarket balance
pub async fn get_polymarket_balance(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let service = state.service.read().await;

    match service.polymarket_client.get_balance().await {
        Ok(balance) => Json(serde_json::json!({
            "success": true,
            "balance": balance
        }))
        .into_response(),
        Err(e) => {
            error!("获取 Polymarket 余额失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "success": false,
                    "error": e.to_string()
                })),
            )
                .into_response()
        }
    }
}

/// Kalshi order request
#[derive(Deserialize)]
pub struct KalshiOrderRequest {
    ticker: String,
    side: String,
    outcome: String,
    count: i32,
    price: i32,
}

/// Place a Kalshi order
pub async fn place_kalshi_order(
    State(state): State<Arc<AppState>>,
    Json(req): Json<KalshiOrderRequest>,
) -> impl IntoResponse {
    let service = state.service.read().await;

    match service
        .place_kalshi_order(&req.ticker, &req.side, &req.outcome, req.count, req.price)
        .await
    {
        Ok(response) => Json(serde_json::json!({
            "success": true,
            "data": response
        }))
        .into_response(),
        Err(e) => {
            error!("下 Kalshi 订单失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "success": false,
                    "error": e.to_string()
                })),
            )
                .into_response()
        }
    }
}

/// Polymarket order request
#[derive(Deserialize)]
pub struct PolymarketOrderRequest {
    token_id: String,
    side: String,
    amount: f64,
}

/// Place a Polymarket order
pub async fn place_polymarket_order(
    State(state): State<Arc<AppState>>,
    Json(req): Json<PolymarketOrderRequest>,
) -> impl IntoResponse {
    let service = state.service.read().await;

    match service
        .place_polymarket_order(&req.token_id, &req.side, req.amount)
        .await
    {
        Ok(response) => Json(serde_json::json!({
            "success": true,
            "data": response
        }))
        .into_response(),
        Err(e) => {
            error!("下 Polymarket 订单失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "success": false,
                    "error": e.to_string()
                })),
            )
                .into_response()
        }
    }
}

/// Arbitrage execution request
#[derive(Deserialize)]
pub struct ExecuteArbitrageRequest {
    event_name: String,
    team_name: String,
    kalshi_side: String,
    polymarket_side: String,
    amount: f64,
}

/// Execute an arbitrage trade
pub async fn execute_arbitrage(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ExecuteArbitrageRequest>,
) -> impl IntoResponse {
    let service = state.service.read().await;

    // Find the matched market
    let matched_market = service
        .get_matched_markets()
        .iter()
        .find(|m| m.event_name == req.event_name && m.team_name == req.team_name);

    let mm = match matched_market {
        Some(m) => m.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "success": false,
                    "error": "Market not found"
                })),
            )
                .into_response();
        }
    };

    // Calculate order amounts
    let total_bet = req.amount;
    // Simplified: 50/50 split (in reality, should use optimal allocation)
    let kalshi_amount = (total_bet / 2.0 * 100.0) as i32; // Convert to cents
    let poly_amount = total_bet / 2.0;

    // Get Polymarket token
    let poly_token = if req.polymarket_side == "yes" {
        mm.polymarket_market.get_token_for_team(&mm.team_name)
    } else {
        let opponent = mm.polymarket_market.get_opponent(&mm.team_name);
        opponent.and_then(|o| mm.polymarket_market.get_token_for_team(o))
    };

    // === Pre-order depth validation ===
    // Check Polymarket depth
    let poly_depth = poly_token
        .and_then(|token| service.polymarket_client.get_orderbook(token))
        .map(|book| book.ask_depth(poly_amount))
        .unwrap_or(0.0);

    // Require at least 90% of requested amount
    let min_poly_depth = poly_amount * 0.9;
    if poly_depth < min_poly_depth {
        return Json(serde_json::json!({
            "success": false,
            "error": format!("Polymarket 深度不足: 需要 {:.2} USD, 可用 {:.2} USD", min_poly_depth, poly_depth),
            "poly_depth": poly_depth,
            "required_depth": min_poly_depth
        }))
        .into_response();
    }

    // Check Kalshi depth
    let kalshi_contracts = kalshi_amount / 100; // Approximate contracts needed
    let kalshi_depth = service
        .kalshi_client
        .get_orderbook(&mm.kalshi_market.market_id)
        .map(|book| book.ask_depth_for_side(&req.kalshi_side, kalshi_contracts))
        .unwrap_or(0);

    let min_kalshi_depth = (kalshi_contracts as f64 * 0.9) as i32;
    if kalshi_depth < min_kalshi_depth {
        return Json(serde_json::json!({
            "success": false,
            "error": format!("Kalshi 深度不足: 需要 {} 合约, 可用 {} 合约", min_kalshi_depth, kalshi_depth),
            "kalshi_depth": kalshi_depth,
            "required_depth": min_kalshi_depth
        }))
        .into_response();
    }

    // === Execute orders ===
    // Place Kalshi order
    let kalshi_price = if req.kalshi_side == "yes" {
        (mm.kalshi_market.yes_price * 100.0) as i32
    } else {
        (mm.kalshi_market.no_price * 100.0) as i32
    };

    let kalshi_result = service
        .place_kalshi_order(
            &mm.kalshi_market.market_id,
            "buy",
            &req.kalshi_side,
            kalshi_amount / kalshi_price, // contracts
            kalshi_price,
        )
        .await;

    // Place Polymarket order
    let poly_result = match poly_token {
        Some(token) => service
            .place_polymarket_order(token, "buy", poly_amount)
            .await,
        None => Err(anyhow::anyhow!("Polymarket token not found")),
    };

    Json(serde_json::json!({
        "success": kalshi_result.is_ok() && poly_result.is_ok(),
        "depth_check": {
            "poly_depth": poly_depth,
            "kalshi_depth": kalshi_depth
        },
        "kalshi": kalshi_result.map(|r| serde_json::json!({"success": true, "data": r}))
            .unwrap_or_else(|e| serde_json::json!({"success": false, "error": e.to_string()})),
        "polymarket": poly_result.map(|r| serde_json::json!({"success": true, "data": r}))
            .unwrap_or_else(|e| serde_json::json!({"success": false, "error": e.to_string()}))
    }))
    .into_response()
}

/// Get unified account balance
pub async fn get_account_balance(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let service = state.service.read().await;

    // Get Kalshi balance (returns f64 in dollars)
    let kalshi_data = match service.kalshi_client.get_balance().await {
        Ok(balance) => {
            serde_json::json!({
                "available": true,
                "balance": balance,
                "portfolio_value": 0.0,  // TODO: Get from separate API if needed
                "updated_ts": 0
            })
        },
        Err(e) => serde_json::json!({
            "available": false,
            "error": e.to_string()
        }),
    };

    // Get Polymarket balance (returns f64 in dollars)
    let poly_data = match service.polymarket_client.get_balance().await {
        Ok(balance) => {
            serde_json::json!({
                "available": true,
                "balance": balance,
                "pnl": "0",
                "trades": 0,
                "positions": 0
            })
        },
        Err(e) => serde_json::json!({
            "available": false,
            "error": e.to_string()
        }),
    };

    Json(serde_json::json!({
        "kalshi": kalshi_data,
        "polymarket": poly_data
    }))
}

/// Query params for orders
#[derive(Deserialize)]
pub struct OrdersQuery {
    status: Option<String>,
}

/// Get Kalshi orders
pub async fn get_kalshi_orders(
    State(state): State<Arc<AppState>>,
    Query(query): Query<OrdersQuery>,
) -> impl IntoResponse {
    let service = state.service.read().await;

    match service.kalshi_client.get_orders(query.status.as_deref()).await {
        Ok(orders) => Json(serde_json::json!({
            "orders": orders.get("orders").unwrap_or(&serde_json::json!([]))
        }))
        .into_response(),
        Err(e) => {
            error!("获取 Kalshi 订单失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "orders": [],
                    "error": e.to_string()
                })),
            )
                .into_response()
        }
    }
}

/// Get Polymarket orders
pub async fn get_polymarket_orders(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let service = state.service.read().await;

    match service.polymarket_client.get_open_orders().await {
        Ok(orders) => Json(serde_json::json!({
            "orders": orders
        }))
        .into_response(),
        Err(e) => {
            error!("获取 Polymarket 订单失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "orders": [],
                    "error": e.to_string()
                })),
            )
                .into_response()
        }
    }
}

/// Get Kalshi positions
pub async fn get_kalshi_positions(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let service = state.service.read().await;

    match service.kalshi_client.get_positions().await {
        Ok(positions) => Json(serde_json::json!({
            "positions": positions.get("market_positions").unwrap_or(&serde_json::json!([]))
        }))
        .into_response(),
        Err(e) => {
            error!("获取 Kalshi 持仓失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "positions": [],
                    "error": e.to_string()
                })),
            )
                .into_response()
        }
    }
}

/// Get Polymarket positions
pub async fn get_polymarket_positions(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let service = state.service.read().await;

    match service.polymarket_client.get_positions().await {
        Ok(positions) => Json(serde_json::json!({
            "positions": positions
        }))
        .into_response(),
        Err(e) => {
            error!("获取 Polymarket 持仓失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "positions": [],
                    "error": e.to_string()
                })),
            )
                .into_response()
        }
    }
}

/// Cancel Kalshi order
pub async fn cancel_kalshi_order(
    State(state): State<Arc<AppState>>,
    Path(order_id): Path<String>,
) -> impl IntoResponse {
    let service = state.service.read().await;

    match service.kalshi_client.cancel_order(&order_id).await {
        Ok(_) => Json(serde_json::json!({
            "success": true
        }))
        .into_response(),
        Err(e) => {
            error!("取消 Kalshi 订单失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "success": false,
                    "error": e.to_string()
                })),
            )
                .into_response()
        }
    }
}

/// Cancel Polymarket order
pub async fn cancel_polymarket_order(
    State(state): State<Arc<AppState>>,
    Path(order_id): Path<String>,
) -> impl IntoResponse {
    let service = state.service.read().await;

    match service.polymarket_client.cancel_order(&order_id).await {
        Ok(_) => Json(serde_json::json!({
            "success": true
        }))
        .into_response(),
        Err(e) => {
            error!("取消 Polymarket 订单失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "success": false,
                    "error": e.to_string()
                })),
            )
                .into_response()
        }
    }
}

/// Get tracking information (placeholder for now)
pub async fn get_tracking(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let service = state.service.read().await;
    let tracking = service.ws_manager.get_tracking_stats();
    Json(tracking)
}

/// Search history query params
#[derive(Deserialize)]
pub struct SearchHistoryQuery {
    min_profit: Option<f64>,
    max_profit: Option<f64>,
    min_duration: Option<f64>,
    max_duration: Option<f64>,
    event_name: Option<String>,
    team_name: Option<String>,
    sort_by: Option<String>,
    sort_order: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
    include_history: Option<bool>,
}

/// Search history records
pub async fn search_history(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SearchHistoryQuery>,
) -> impl IntoResponse {
    let service = state.service.read().await;

    match service.search_history(
        query.min_profit,
        query.max_profit,
        query.min_duration,
        query.max_duration,
        query.event_name,
        query.team_name,
        query.sort_by,
        query.sort_order,
        query.limit,
        query.offset,
        query.include_history,
    ) {
        Ok(result) => Json(result).into_response(),
        Err(e) => {
            error!("搜索历史记录失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "records": [],
                    "total": 0,
                    "error": e.to_string()
                })),
            )
                .into_response()
        }
    }
}

/// Get history statistics
pub async fn get_history_statistics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let service = state.service.read().await;

    match service.get_history_statistics() {
        Ok(stats) => Json(stats).into_response(),
        Err(e) => {
            error!("获取历史统计失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "total_records": 0,
                    "error": e.to_string()
                })),
            )
                .into_response()
        }
    }
}

// ==================== Authentication ====================

/// Login request
#[derive(Deserialize)]
pub struct LoginRequest {
    username: String,
    password: String,
}

/// Login response
#[derive(Serialize)]
pub struct LoginResponse {
    access_token: String,
    token_type: String,
    username: String,
}

/// JWT Claims
#[derive(Serialize, Deserialize)]
struct Claims {
    sub: String,
    exp: i64,
    iat: i64,
}

/// Login endpoint
pub async fn login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LoginRequest>,
) -> impl IntoResponse {
    let auth_config = &state.config.auth;

    // Validate credentials
    if req.username != auth_config.username || req.password != auth_config.password {
        error!("登录失败: 用户名或密码错误 (username: {})", req.username);
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "detail": "用户名或密码错误"
            })),
        )
            .into_response();
    }

    // Generate JWT token
    let now = Utc::now();
    let exp = now + Duration::hours(auth_config.token_expire_hours as i64);

    let claims = Claims {
        sub: req.username.clone(),
        exp: exp.timestamp(),
        iat: now.timestamp(),
    };

    let token = match encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(auth_config.secret_key.as_bytes()),
    ) {
        Ok(t) => t,
        Err(e) => {
            error!("生成 JWT 失败: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "detail": "生成 token 失败"
                })),
            )
                .into_response();
        }
    };

    info!("用户 {} 登录成功", req.username);

    Json(LoginResponse {
        access_token: token,
        token_type: "Bearer".to_string(),
        username: req.username,
    })
    .into_response()
}

/// Query params for orderbook depth
#[derive(Deserialize)]
pub struct OrderbookDepthQuery {
    pub kalshi_ticker: Option<String>,
    pub poly_token_id: Option<String>,
    pub poly_opponent_token_id: Option<String>,
}

/// Depth for a single side (Yes or No)
#[derive(Serialize, Default)]
pub struct SideDepth {
    pub price: Option<f64>,
    pub size: Option<f64>,
}

/// Orderbook depth for a single platform with Yes/No
#[derive(Serialize, Default)]
pub struct PlatformDepthDual {
    pub yes: SideDepth,
    pub no: SideDepth,
}

/// Full orderbook depth response
#[derive(Serialize)]
pub struct OrderbookDepthResponse {
    pub kalshi: Option<PlatformDepthDual>,
    pub polymarket: Option<PlatformDepthDual>,
}

/// Get orderbook depth for specified markets
pub async fn get_orderbook_depth(
    State(state): State<Arc<AppState>>,
    Query(query): Query<OrderbookDepthQuery>,
) -> impl IntoResponse {
    let service = state.service.read().await;
    
    // Get Kalshi orderbook depth (Yes and No)
    let kalshi_depth = query.kalshi_ticker.as_ref().and_then(|ticker| {
        service.kalshi_client.get_orderbook(ticker).map(|book| {
            // Kalshi orderbook stores bids for yes and no sides
            // Yes ask = 1 - No best bid, No ask = 1 - Yes best bid
            let yes_best_bid = book.yes.last();
            let no_best_bid = book.no.last();
            
            // Yes side: best ask price = 1 - no_best_bid, size from no side
            let yes = if let Some((price_cents, qty)) = no_best_bid {
                SideDepth {
                    price: Some(1.0 - (*price_cents as f64 / 100.0)),
                    size: Some(*qty as f64),
                }
            } else {
                SideDepth::default()
            };
            
            // No side: best ask price = 1 - yes_best_bid, size from yes side
            let no = if let Some((price_cents, qty)) = yes_best_bid {
                SideDepth {
                    price: Some(1.0 - (*price_cents as f64 / 100.0)),
                    size: Some(*qty as f64),
                }
            } else {
                SideDepth::default()
            };
            
            PlatformDepthDual { yes, no }
        })
    });
    
    // Get Polymarket orderbook depth (Yes from own token, No from opponent token)
    let poly_yes = query.poly_token_id.as_ref().and_then(|token_id| {
        service.polymarket_client.get_orderbook(token_id).and_then(|book| {
            book.best_ask().map(|(price, size)| SideDepth {
                price: Some(price),
                size: Some(size),
            })
        })
    }).unwrap_or_default();
    
    let poly_no = query.poly_opponent_token_id.as_ref().and_then(|token_id| {
        service.polymarket_client.get_orderbook(token_id).and_then(|book| {
            book.best_ask().map(|(price, size)| SideDepth {
                price: Some(price),
                size: Some(size),
            })
        })
    }).unwrap_or_default();
    
    let poly_depth = if query.poly_token_id.is_some() || query.poly_opponent_token_id.is_some() {
        Some(PlatformDepthDual { yes: poly_yes, no: poly_no })
    } else {
        None
    };
    
    Json(OrderbookDepthResponse {
        kalshi: kalshi_depth,
        polymarket: poly_depth,
    })
}

// ==================== Auto-Trade API ====================

/// Auto-trade status response
#[derive(Serialize)]
pub struct AutoTradeStatusResponse {
    pub enabled: bool,
    pub trade_count: i32,
    pub max_trade_count: i32,
    pub remaining: i32,
    pub max_amount: f64,
    pub min_duration_ms: i64,
    pub last_trade_time: Option<String>,
}

/// Get auto-trade status
pub async fn get_auto_trade_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let service = state.service.read().await;
    let auto_state = service.ws_manager.get_auto_trade_state();
    
    Json(AutoTradeStatusResponse {
        enabled: auto_state.enabled,
        trade_count: auto_state.trade_count,
        max_trade_count: auto_state.max_trade_count,
        remaining: auto_state.max_trade_count - auto_state.trade_count,
        max_amount: auto_state.max_amount,
        min_duration_ms: auto_state.min_duration_ms,
        last_trade_time: auto_state.last_trade_time,
    })
}

/// Enable auto-trade
pub async fn enable_auto_trade(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let service = state.service.read().await;
    
    match service.ws_manager.enable_auto_trade() {
        Ok(_) => Json(serde_json::json!({
            "success": true,
            "message": "自动下单已开启"
        })).into_response(),
        Err(e) => {
            error!("开启自动下单失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "success": false,
                    "error": e.to_string()
                }))
            ).into_response()
        }
    }
}

/// Disable auto-trade
pub async fn disable_auto_trade(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let service = state.service.read().await;
    
    match service.ws_manager.disable_auto_trade() {
        Ok(_) => Json(serde_json::json!({
            "success": true,
            "message": "自动下单已关闭"
        })).into_response(),
        Err(e) => {
            error!("关闭自动下单失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "success": false,
                    "error": e.to_string()
                }))
            ).into_response()
        }
    }
}

/// Reset auto-trade count
pub async fn reset_auto_trade(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let service = state.service.read().await;
    
    match service.ws_manager.reset_trade_count() {
        Ok(_) => Json(serde_json::json!({
            "success": true,
            "message": "下单次数已重置"
        })).into_response(),
        Err(e) => {
            error!("重置下单次数失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "success": false,
                    "error": e.to_string()
                }))
            ).into_response()
        }
    }
}

/// Auto-trade settings update request
#[derive(Deserialize)]
pub struct AutoTradeSettingsRequest {
    pub max_amount: Option<f64>,
    pub min_duration_ms: Option<i64>,
    pub max_trade_count: Option<i32>,
}

/// Update auto-trade settings
pub async fn update_auto_trade_settings(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AutoTradeSettingsRequest>,
) -> impl IntoResponse {
    let service = state.service.read().await;
    
    match service.ws_manager.update_auto_trade_settings(
        req.max_amount,
        req.min_duration_ms,
        req.max_trade_count,
    ) {
        Ok(_) => Json(serde_json::json!({
            "success": true,
            "message": "设置已更新"
        })).into_response(),
        Err(e) => {
            error!("更新自动下单设置失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "success": false,
                    "error": e.to_string()
                }))
            ).into_response()
        }
    }
}

/// Query params for auto-trade history
#[derive(Deserialize)]
pub struct AutoTradeHistoryQuery {
    pub limit: Option<usize>,
}

/// Get auto-trade execution history
pub async fn get_auto_trade_history(
    State(state): State<Arc<AppState>>,
    Query(query): Query<AutoTradeHistoryQuery>,
) -> impl IntoResponse {
    let service = state.service.read().await;
    let storage = service.ws_manager.get_storage();
    let limit = query.limit.unwrap_or(50);
    
    match storage.get_auto_trade_history(limit) {
        Ok(records) => Json(serde_json::json!({
            "records": records,
            "total": records.len()
        })).into_response(),
        Err(e) => {
            error!("获取自动下单历史失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "records": [],
                    "total": 0,
                    "error": e.to_string()
                }))
            ).into_response()
        }
    }
}

// ==================== App Settings API ====================

/// App settings response
#[derive(Serialize)]
pub struct AppSettingsResponse {
    /// 数据刷新间隔（秒）
    pub refresh_interval: u64,
    /// 显示套利机会的最小利润率（%）
    pub min_profit_margin: f64,
    /// 套利计算使用的默认金额（美元）
    pub default_bet_amount: f64,
    /// 开始追踪记录的利润率阈值（%）
    pub tracking_threshold: f64,
    pub updated_at: Option<String>,
}

/// Get application settings
pub async fn get_app_settings(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let service = state.service.read().await;
    let storage = service.ws_manager.get_storage();
    
    match storage.get_app_settings() {
        Ok(settings) => Json(AppSettingsResponse {
            refresh_interval: settings.refresh_interval,
            min_profit_margin: settings.min_profit_margin,
            default_bet_amount: settings.default_bet_amount,
            tracking_threshold: settings.tracking_threshold,
            updated_at: settings.updated_at,
        }).into_response(),
        Err(e) => {
            error!("获取应用设置失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": e.to_string()
                }))
            ).into_response()
        }
    }
}

/// App settings update request
#[derive(Deserialize)]
pub struct AppSettingsRequest {
    /// 数据刷新间隔（秒）
    pub refresh_interval: Option<u64>,
    /// 显示套利机会的最小利润率（%）
    pub min_profit_margin: Option<f64>,
    /// 套利计算使用的默认金额（美元）
    pub default_bet_amount: Option<f64>,
    /// 开始追踪记录的利润率阈值（%）
    pub tracking_threshold: Option<f64>,
}

/// Update application settings
pub async fn update_app_settings(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AppSettingsRequest>,
) -> impl IntoResponse {
    let service = state.service.read().await;
    
    // Update storage
    match service.ws_manager.update_app_settings(
        req.refresh_interval,
        req.min_profit_margin,
        req.default_bet_amount,
        req.tracking_threshold,
    ) {
        Ok(_) => Json(serde_json::json!({
            "success": true,
            "message": "应用设置已更新"
        })).into_response(),
        Err(e) => {
            error!("更新应用设置失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "success": false,
                    "error": e.to_string()
                }))
            ).into_response()
        }
    }
}

// ============================================================
// Market Exclusion APIs
// ============================================================

/// Market exclusion request
#[derive(Deserialize)]
pub struct MarketExclusionRequest {
    pub event_name: String,
    pub team_name: String,
}

/// Get list of excluded markets
pub async fn get_excluded_markets(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let service = state.service.read().await;
    let excluded = service.ws_manager.get_excluded_markets();
    
    Json(serde_json::json!({
        "excluded_markets": excluded,
        "count": excluded.len()
    }))
}

/// Exclude a market from auto-trade
pub async fn exclude_market(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MarketExclusionRequest>,
) -> impl IntoResponse {
    let service = state.service.read().await;
    let inserted = service.ws_manager.exclude_market(&req.event_name, &req.team_name);
    
    Json(serde_json::json!({
        "success": true,
        "message": if inserted { "市场已排除" } else { "市场已在排除列表中" },
        "event_name": req.event_name,
        "team_name": req.team_name
    }))
}

/// Remove a market from exclusion list
pub async fn unexclude_market(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MarketExclusionRequest>,
) -> impl IntoResponse {
    let service = state.service.read().await;
    let removed = service.ws_manager.unexclude_market(&req.event_name, &req.team_name);
    
    Json(serde_json::json!({
        "success": true,
        "message": if removed { "市场已取消排除" } else { "市场不在排除列表中" },
        "event_name": req.event_name,
        "team_name": req.team_name
    }))
}
