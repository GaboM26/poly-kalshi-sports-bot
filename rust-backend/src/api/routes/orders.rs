//! Manual order endpoints: place, list and cancel orders on either exchange.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use tracing::{error, info, warn};

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
    let kalshi_client = state.platform.kalshi.clone();

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

    // The WebSocket book only covers tickers the scanner is currently
    // tracking/subscribed to. A held position's market can fall out of that
    // set (e.g. the match ended and it was unsubscribed) while still being
    // perfectly sellable on the exchange, so a manual order - already a
    // known, user-specified quantity, not something auto-trade sizes off of
    // depth - falls back to a fresh REST quote rather than failing outright.
    let ws_price = kalshi_client
        .get_orderbook(&req.ticker)
        .and_then(|book| book.limit_price_for_order(&req.side, &req.action));

    let price = match ws_price {
        Some(price) if (1..=99).contains(&price) => Some(price),
        _ => match kalshi_client.get_market_quotes(&[req.ticker.clone()]).await {
            Ok(quotes) => quotes
                .iter()
                .find(|quote| quote.market_id == req.ticker)
                .and_then(|quote| quote.limit_price_for_order(&req.side, &req.action)),
            Err(e) => {
                warn!(
                    "REST quote fallback failed for manual Kalshi order on {}: {:#}",
                    req.ticker, e
                );
                None
            }
        },
    };

    let Some(price) = price.filter(|price| (1..=99).contains(price)) else {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "success": false,
                "error": "No executable Kalshi quote is available for this order"
            })),
        )
            .into_response();
    };

    match kalshi_client
        .place_order(&req.ticker, &req.action, &req.side, req.count, price)
        .await
    {
        Ok(response) => {
            info!(
                "Manual Kalshi order placed: ticker={} side={} action={} count={} price={}c response={:?}",
                req.ticker, req.side, req.action, req.count, price, response
            );
            Json(serde_json::json!({
                "success": true,
                "order": response,
                "data": response
            }))
            .into_response()
        }
        Err(e) => {
            error!("Failed to place Kalshi order: {:#}", e);
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
    let service = &state.platform;

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
        .polymarket
        .place_limit_order(
            &req.market_slug,
            req.position_side,
            &req.side,
            req.contracts,
            req.price,
        )
        .await
    {
        Ok(response) => {
            info!(
                "Manual Polymarket order placed: market_slug={} side={} contracts={} price={} order_id={:?} status={:?} filled_contracts={:?}",
                req.market_slug,
                req.side,
                req.contracts,
                req.price,
                response.get("order_id"),
                response.get("status"),
                response.get("filled_contracts"),
            );
            Json(serde_json::json!({
                "success": true,
                "order_id": response.get("order_id"),
                "status": response.get("status"),
                "filled_contracts": response.get("filled_contracts"),
                "elapsed_ms": response.get("latency_ms"),
                "data": response.get("data")
            }))
            .into_response()
        }
        Err(e) => {
            error!("Failed to place Polymarket order: {:#}", e);
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
    let service = &state.platform;

    match service
        .kalshi
        .get_orders(query.status.as_deref())
        .await
    {
        Ok(orders) => Json(serde_json::json!({
            "orders": orders.get("orders").unwrap_or(&serde_json::json!([]))
        }))
        .into_response(),
        Err(e) => {
            error!("Failed to get Kalshi orders: {:#}", e);
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
    let service = &state.platform;

    match service.polymarket.get_open_orders().await {
        Ok(orders) => Json(serde_json::json!({
            "orders": orders
        }))
        .into_response(),
        Err(e) => {
            error!("Failed to get Polymarket orders: {:#}", e);
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
    let service = &state.platform;

    match service.kalshi.cancel_order(&order_id).await {
        Ok(_) => Json(serde_json::json!({
            "success": true
        }))
        .into_response(),
        Err(e) => {
            error!("Failed to cancel Kalshi order: {:#}", e);
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
    let service = &state.platform;

    match service.polymarket.cancel_order(&order_id).await {
        Ok(_) => Json(serde_json::json!({
            "success": true
        }))
        .into_response(),
        Err(e) => {
            error!("Failed to cancel Polymarket order: {:#}", e);
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
