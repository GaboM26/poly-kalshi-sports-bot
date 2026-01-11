//! Order operations endpoints

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use tracing::error;

use crate::api::AppState;

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
    let kalshi_amount = (total_bet / 2.0 * 100.0) as i32;
    let poly_amount = total_bet / 2.0;

    // Get Polymarket token
    let poly_token = if req.polymarket_side == "yes" {
        mm.polymarket_market.get_token_for_team(&mm.team_name)
    } else {
        let opponent = mm.polymarket_market.get_opponent(&mm.team_name);
        opponent.and_then(|o| mm.polymarket_market.get_token_for_team(o))
    };

    // Check Polymarket depth
    let poly_depth = poly_token
        .and_then(|token| service.polymarket_client.get_orderbook(token))
        .map(|book| book.ask_depth(poly_amount))
        .unwrap_or(0.0);

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
    let kalshi_contracts = kalshi_amount / 100;
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

    // Execute orders
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
            kalshi_amount / kalshi_price,
            kalshi_price,
        )
        .await;

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
