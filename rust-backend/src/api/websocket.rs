//! WebSocket server for real-time updates to frontend

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tracing::{error, info};

use super::AppState;
use crate::models::WsMessage;
use crate::strategies::send_ws;

/// How often each client gets the performance metrics report.
const METRICS_PUSH: Duration = Duration::from_secs(10);

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

    info!("New WebSocket client connected");

    // Create channel for outgoing messages
    let (tx, mut rx) = mpsc::channel::<String>(100);

    // Every strategy sends its initial state, then streams its own messages.
    let mut tasks = Vec::new();
    for strategy in &state.strategies {
        tasks.extend(strategy.clone().feed_client(tx.clone()).await);
    }

    // Spawn task for periodic metrics updates (every 10 seconds)
    let metrics = state.platform.metrics.clone();
    let tx_metrics = tx.clone();
    tasks.push(tokio::spawn(async move {
        let mut interval = tokio::time::interval(METRICS_PUSH);
        loop {
            interval.tick().await;

            let msg = WsMessage::Metrics { data: metrics.report() };
            if !send_ws(&tx_metrics, &msg).await {
                break;
            }
        }
    }));
    drop(tx);

    // Spawn task to send messages to WebSocket
    let send_task = tokio::spawn(async move {
        while let Some(json) = rx.recv().await {
            if sender.send(Message::Text(json)).await.is_err() {
                break;
            }
        }
    });

    // Handle incoming messages (ping/pong, close)
    let recv_task = tokio::spawn(async move {
        while let Some(result) = receiver.next().await {
            match result {
                Ok(Message::Ping(_data)) => {
                    // Pong is handled automatically by axum
                }
                Ok(Message::Close(_)) => {
                    break;
                }
                Err(e) => {
                    error!("WebSocket error: {:#}", e);
                    break;
                }
                _ => {}
            }
        }
    });

    // Wait for recv_task to complete (client disconnected)
    let _ = recv_task.await;

    // Cleanup: abort all other tasks
    for task in tasks {
        task.abort();
    }
    send_task.abort();

    info!("WebSocket client disconnected");
}
