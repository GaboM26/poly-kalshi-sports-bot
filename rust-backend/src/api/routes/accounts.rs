//! Account balance and position endpoints

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::{Duration, Utc};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use crate::api::AppState;

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

/// Get unified account balance
pub async fn get_account_balance(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let service = state.service.read().await;

    let kalshi_data = match service.kalshi_client.get_balance().await {
        Ok(balance) => {
            serde_json::json!({
                "available": true,
                "balance": balance,
                "portfolio_value": 0.0,
                "updated_ts": 0
            })
        },
        Err(e) => serde_json::json!({
            "available": false,
            "error": e.to_string()
        }),
    };

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
