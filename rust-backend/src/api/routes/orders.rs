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
use crate::models::PolymarketPositionSide;

/// Kalshi order request
#[derive(Deserialize)]
pub struct KalshiOrderRequest {
    ticker: String,
    side: String,
    action: String,
    count: i32,
}

/// Place a Kalshi order
pub async fn place_kalshi_order(
    State(state): State<Arc<AppState>>,
    Json(req): Json<KalshiOrderRequest>,
) -> impl IntoResponse {
    let kalshi_client = {
        let service = state.service.read().await;
        service.kalshi_client.clone()
    };

    if !matches!(req.side.as_str(), "yes" | "no")
        || !matches!(req.action.as_str(), "buy" | "sell")
        || req.count <= 0
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "success": false,
                "error": "side must be yes or no, action must be buy or sell, and count must be positive"
            })),
        )
            .into_response();
    }

    let price = match kalshi_client
        .get_orderbook(&req.ticker)
        .and_then(|book| book.limit_price_for_order(&req.side, &req.action))
    {
        Some(price) if (1..=99).contains(&price) => price,
        _ => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "success": false,
                    "error": "No executable Kalshi quote is available for this order"
                })),
            )
                .into_response();
        }
    };

    match kalshi_client
        .place_order(&req.ticker, &req.action, &req.side, req.count, price)
        .await
    {
        Ok(response) => Json(serde_json::json!({
            "success": true,
            "order": response,
            "data": response
        }))
        .into_response(),
        Err(e) => {
            error!("Failed to place Kalshi order: {}", e);
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
    market_slug: String,
    position_side: PolymarketPositionSide,
    side: String,
    contracts: i32,
    price: f64,
}

/// Place a Polymarket order
pub async fn place_polymarket_order(
    State(state): State<Arc<AppState>>,
    Json(req): Json<PolymarketOrderRequest>,
) -> impl IntoResponse {
    let service = state.service.read().await;

    if req.market_slug.trim().is_empty()
        || !matches!(req.side.as_str(), "buy" | "sell")
        || req.contracts <= 0
        || !req.price.is_finite()
        || !(0.0..1.0).contains(&req.price)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "success": false,
                "error": "market_slug, position_side, buy or sell action, a positive whole contract count, and a current price are required"
            })),
        )
            .into_response();
    }

    match service
        .place_polymarket_order(
            &req.market_slug,
            req.position_side,
            &req.side,
            req.contracts,
            req.price,
        )
        .await
    {
        Ok(response) => Json(serde_json::json!({
            "success": true,
            "order_id": response.get("order_id"),
            "status": response.get("status"),
            "filled_contracts": response.get("filled_contracts"),
            "elapsed_ms": response.get("latency_ms"),
            "data": response.get("data")
        }))
        .into_response(),
        Err(e) => {
            error!("Failed to place Polymarket order: {}", e);
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
    polymarket_competitor: String,
    contracts: i32,
}

/// Execute an arbitrage trade
pub async fn execute_arbitrage(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ExecuteArbitrageRequest>,
) -> impl IntoResponse {
    if !matches!(req.kalshi_side.as_str(), "yes" | "no") || req.contracts <= 0 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "success": false,
                "error": "kalshi_side must be yes or no and contract count must be positive"
            })),
        )
            .into_response();
    }

    let service = state.service.read().await;
    let matched_market = service
        .get_matched_markets()
        .iter()
        .find(|market| market.event_name == req.event_name && market.team_name == req.team_name);
    let Some(matched_market) = matched_market else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"success": false, "error": "Market not found"})),
        )
            .into_response();
    };

    // Resolve the user-selected competitor against the matched US gateway
    // market. This yields the authoritative market slug and LONG/SHORT side;
    // neither is inferred from a yes/no array position.
    let Some(execution) = matched_market
        .polymarket_market
        .us_execution_for_competitor(&req.polymarket_competitor)
    else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "success": false,
                "error": "Polymarket competitor is not part of the matched market"
            })),
        )
            .into_response();
    };

    // The US quote feed has no verified executable order-book size. Do not
    // place the Kalshi leg first, or either leg, for a manual paired trade.
    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({
            "success": false,
            "error": "Polymarket US executable liquidity is unavailable; paired arbitrage was not submitted",
            "polymarket_market_slug": execution.market_slug,
            "polymarket_position_side": execution.position_side,
        })),
    )
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

    match service
        .kalshi_client
        .get_orders(query.status.as_deref())
        .await
    {
        Ok(orders) => Json(serde_json::json!({
            "orders": orders.get("orders").unwrap_or(&serde_json::json!([]))
        }))
        .into_response(),
        Err(e) => {
            error!("Failed to get Kalshi orders: {}", e);
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
            error!("Failed to get Polymarket orders: {}", e);
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
            error!("Failed to cancel Kalshi order: {}", e);
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
            error!("Failed to cancel Polymarket order: {}", e);
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

/// Get auto-trade queue status
pub async fn get_auto_trade_queue(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let service = state.service.read().await;
    let queue: Vec<String> = service
        .ws_manager
        .auto_trade_queue
        .read()
        .iter()
        .cloned()
        .collect();
    let is_executing = service
        .ws_manager
        .is_auto_trading
        .load(std::sync::atomic::Ordering::Relaxed);

    Json(serde_json::json!({
        "queue": queue,
        "queue_length": queue.len(),
        "is_executing": is_executing
    }))
}
