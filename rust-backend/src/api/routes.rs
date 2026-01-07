//! HTTP API Routes

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::error;

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
    Json(serde_json::json!({
        "total_markets": coverage.total_markets,
        "kalshi_coverage": coverage.kalshi_coverage,
        "poly_coverage": coverage.poly_coverage,
        "full_coverage": coverage.full_coverage
    }))
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
    let markets = service.get_matched_markets().to_vec();
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
            error!("Failed to get history: {}", e);
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
            error!("Failed to get Kalshi balance: {}", e);
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
            error!("Failed to get Polymarket balance: {}", e);
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
    let poly_token = if req.polymarket_side == "yes" {
        mm.polymarket_market.get_token_for_team(&mm.team_name)
    } else {
        let opponent = mm.polymarket_market.get_opponent(&mm.team_name);
        opponent.and_then(|o| mm.polymarket_market.get_token_for_team(o))
    };

    let poly_result = match poly_token {
        Some(token) => service
            .place_polymarket_order(token, "buy", poly_amount)
            .await,
        None => Err(anyhow::anyhow!("Polymarket token not found")),
    };

    Json(serde_json::json!({
        "success": kalshi_result.is_ok() && poly_result.is_ok(),
        "kalshi": kalshi_result.map(|r| serde_json::json!({"success": true, "data": r}))
            .unwrap_or_else(|e| serde_json::json!({"success": false, "error": e.to_string()})),
        "polymarket": poly_result.map(|r| serde_json::json!({"success": true, "data": r}))
            .unwrap_or_else(|e| serde_json::json!({"success": false, "error": e.to_string()}))
    }))
    .into_response()
}
