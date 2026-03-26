//! WebSocket handlers for real-time updates.

use crate::events::{EVENT_ALERT_RAISED, EVENT_POSITION_UPDATED};
use crate::state::AppState;
use axum::{
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::Response,
};
use futures::{SinkExt, StreamExt};
use tracing::{debug, error, info};

/// WebSocket handler for position updates.
pub async fn positions_ws(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(|socket| handle_positions_ws(socket, state))
}

/// Handles position WebSocket connection.
async fn handle_positions_ws(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    // Subscribe to position updates via async event bus.
    let mut rx = state.event_bus.subscribe(EVENT_POSITION_UPDATED);

    info!("Position WebSocket client connected");

    // Spawn task to forward updates to client
    let send_task = tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            let msg = serde_json::to_string(&event.payload).unwrap_or_default();
            if sender.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
    });

    // Handle incoming messages (ping/pong, close)
    let recv_task = tokio::spawn(async move {
        while let Some(msg) = receiver.next().await {
            match msg {
                Ok(Message::Ping(_data)) => {
                    debug!("Received ping");
                    // Pong is handled automatically by axum
                }
                Ok(Message::Close(_)) => {
                    debug!("Client closed connection");
                    break;
                }
                Err(e) => {
                    error!(error = %e, "WebSocket error");
                    break;
                }
                _ => {}
            }
        }
    });

    // Wait for either task to complete
    tokio::select! {
        _ = send_task => {},
        _ = recv_task => {},
    }

    info!("Position WebSocket client disconnected");
}

/// WebSocket handler for alert updates.
pub async fn alerts_ws(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(|socket| handle_alerts_ws(socket, state))
}

/// Handles alerts WebSocket connection.
async fn handle_alerts_ws(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    // Subscribe to alert updates via async event bus.
    let mut rx = state.event_bus.subscribe(EVENT_ALERT_RAISED);

    info!("Alerts WebSocket client connected");

    // Spawn task to forward alerts to client
    let send_task = tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            let msg = serde_json::to_string(&event.payload).unwrap_or_default();
            if sender.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
    });

    // Handle incoming messages
    let recv_task = tokio::spawn(async move {
        while let Some(msg) = receiver.next().await {
            match msg {
                Ok(Message::Close(_)) => {
                    debug!("Client closed connection");
                    break;
                }
                Err(e) => {
                    error!(error = %e, "WebSocket error");
                    break;
                }
                _ => {}
            }
        }
    });

    // Wait for either task to complete
    tokio::select! {
        _ = send_task => {},
        _ = recv_task => {},
    }

    info!("Alerts WebSocket client disconnected");
}
