//! WebSocket server for real-time updates to frontend

use std::sync::Arc;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast;
use tracing::{error, info};

use super::AppState;
use crate::models::{ArbitrageOpportunity, WsMessage};

/// WebSocket handler
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// Handle WebSocket connection
async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();

    info!("新的 WebSocket 客户端已连接");

    // Subscribe to opportunity updates
    let service = state.service.read().await;
    let mut opportunity_rx = service.ws_manager.subscribe();
    drop(service);

    // Send initial data
    {
        let service = state.service.read().await;
        let opportunities = service.get_opportunities();
        let stats = service.get_stats();

        // Send current opportunities
        if !opportunities.is_empty() {
            let msg = WsMessage::Opportunities(opportunities);
            if let Ok(json) = serde_json::to_string(&msg) {
                let _ = sender.send(Message::Text(json)).await;
            }
        }

        // Send stats
        let msg = WsMessage::Stats(stats);
        if let Ok(json) = serde_json::to_string(&msg) {
            let _ = sender.send(Message::Text(json)).await;
        }
    }

    // Spawn task to send opportunity updates
    let mut send_task = tokio::spawn(async move {
        while let Ok(opportunity) = opportunity_rx.recv().await {
            let msg = WsMessage::Opportunity(opportunity);
            if let Ok(json) = serde_json::to_string(&msg) {
                if sender.send(Message::Text(json)).await.is_err() {
                    break;
                }
            }
        }
    });

    // Handle incoming messages (ping/pong, close)
    let mut recv_task = tokio::spawn(async move {
        while let Some(result) = receiver.next().await {
            match result {
                Ok(Message::Ping(data)) => {
                    // Pong is handled automatically by axum
                }
                Ok(Message::Close(_)) => {
                    break;
                }
                Err(e) => {
                    error!("WebSocket 错误: {}", e);
                    break;
                }
                _ => {}
            }
        }
    });

    // Wait for either task to complete
    tokio::select! {
        _ = (&mut send_task) => {
            recv_task.abort();
        }
        _ = (&mut recv_task) => {
            send_task.abort();
        }
    }

    info!("WebSocket 客户端已断开连接");
}
