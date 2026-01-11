//! Market data query endpoints

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::api::AppState;

/// Get current arbitrage opportunities
pub async fn get_opportunities(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let service = state.service.read().await;
    let opportunities = service.get_opportunities();
    Json(opportunities)
}

/// Get matched markets
pub async fn get_matched_markets(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let service = state.service.read().await;
    let markets = service.ws_manager.get_matched_markets_for_frontend();
    Json(markets)
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
            let yes_best_bid = book.yes.last();
            let no_best_bid = book.no.last();
            
            let yes = if let Some((price_cents, qty)) = no_best_bid {
                SideDepth {
                    price: Some(1.0 - (*price_cents as f64 / 100.0)),
                    size: Some(*qty as f64),
                }
            } else {
                SideDepth::default()
            };
            
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
    
    // Get Polymarket orderbook depth
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
