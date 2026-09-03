//! Real WebSocket server for the Fanout Gateway.
//!
//! Provides an axum-based WebSocket endpoint that clients connect to for
//! real-time event push.  A single WS connection is **user-scoped**: it
//! receives every event for every conversation the user is a member of.
//!
//! # Protocol
//!
//! 1. Client opens `GET /ws/fanout?user_id=U&client_id=Y`
//! 2. Server subscribes the client for user U via `FanoutGateway::subscribe_user`.
//!    This looks up all conversations U is an active member of and seeds the
//!    per-conversation cursor map from the persisted `client_cursors` table.
//! 3. Server spawns a reader task (drain pings/pongs/close) and a writer task
//!    (forward events from the mpsc receiver to the WS sink).
//! 4. On disconnect, the server unsubscribes.  Persisted cursors stay intact
//!    so the next reconnect replays missed events.
//!
use std::sync::Arc;

use axum::{
    Router,
    extract::{Query, State, WebSocketUpgrade, ws},
    response::IntoResponse,
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::cursor::CursorStore;
use crate::gateway::{EventSource, FanoutGateway};
use crate::models::FanoutEvent;

// ---------------------------------------------------------------------------
// Query parameters
// ---------------------------------------------------------------------------

/// Query parameters for the WebSocket handshake.
///
/// `user_id` is the canonical subscription key.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WsParams {
    /// The user to subscribe.  One WS per user; it receives events for every
    /// conversation the user is an active member of.
    pub user_id: String,
    /// Client identifier (tab_id / device_id).
    pub client_id: String,
}

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

/// Shared state for the WebSocket handler, wrapping the FanoutGateway.
pub struct WsFanoutState<
    E: EventSource + Send + Sync + 'static,
    C: CursorStore + Send + Sync + 'static,
> {
    pub gateway: Arc<FanoutGateway<E, C>>,
}

impl<E: EventSource + Send + Sync + 'static, C: CursorStore + Send + Sync + 'static> Clone
    for WsFanoutState<E, C>
{
    fn clone(&self) -> Self {
        Self {
            gateway: Arc::clone(&self.gateway),
        }
    }
}

// ---------------------------------------------------------------------------
// Router factory
// ---------------------------------------------------------------------------

/// Build axum routes for the WebSocket fanout endpoint.
///
/// Mount this at `/ws/fanout` (or wherever appropriate) in the pipeline's
/// HTTP server.
pub fn ws_fanout_routes<E, C>(state: WsFanoutState<E, C>) -> Router
where
    E: EventSource + Send + Sync + 'static,
    C: CursorStore + Send + Sync + 'static,
{
    Router::new()
        .route("/ws/fanout", get(ws_handler))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

async fn ws_handler<E, C>(
    State(state): State<WsFanoutState<E, C>>,
    Query(params): Query<WsParams>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse
where
    E: EventSource + Send + Sync + 'static,
    C: CursorStore + Send + Sync + 'static,
{
    tracing::info!(
        client_id = %params.client_id,
        user_id = %params.user_id,
        "WS fanout: client connecting"
    );

    ws.on_upgrade(move |socket| handle_socket(socket, state, params))
}

async fn handle_socket<E, C>(socket: ws::WebSocket, state: WsFanoutState<E, C>, params: WsParams)
where
    E: EventSource + Send + Sync + 'static,
    C: CursorStore + Send + Sync + 'static,
{
    let (mut ws_tx, mut ws_rx) = socket.split();
    let gateway = &state.gateway;

    // User-scoped subscription: opens a channel and seeds cursors for every
    // conversation the user belongs to.
    let buffer_size = 256;
    let mut event_rx: mpsc::Receiver<FanoutEvent> = match gateway
        .subscribe_user(&params.client_id, &params.user_id, buffer_size)
        .await
    {
        Ok(rx) => rx,
        Err(e) => {
            tracing::error!(
                client_id = %params.client_id,
                user_id = %params.user_id,
                error = %e,
                "WS fanout: failed to subscribe user"
            );
            return;
        }
    };

    let client_id = params.client_id.clone();
    let user_id = params.user_id.clone();

    // Writer task: forward events from the subscription channel to the WS.
    let write_task = tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            match serde_json::to_string(&event) {
                Ok(json) => {
                    if ws_tx.send(ws::Message::Text(json.into())).await.is_err() {
                        break; // Client disconnected
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "WS fanout: failed to serialize event");
                }
            }
        }
    });

    // Reader task: drain incoming messages (pings, close frames).
    let read_task = tokio::spawn(async move {
        while let Some(msg) = ws_rx.next().await {
            match msg {
                Ok(ws::Message::Close(_)) | Err(_) => break,
                _ => {} // Ignore pings, pongs, text from client
            }
        }
    });

    // Wait for either task to finish (= disconnect).
    tokio::select! {
        _ = write_task => {}
        _ = read_task => {}
    }

    // Unsubscribe on disconnect.
    gateway.unsubscribe(&client_id, &user_id).await;

    tracing::info!(
        client_id = %client_id,
        user_id = %user_id,
        "WS fanout: client disconnected"
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_params_deserialize_minimal() {
        let json = r#"{"user_id":"user-1","client_id":"c1"}"#;
        let params: WsParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.user_id, "user-1");
        assert_eq!(params.client_id, "c1");
    }

    #[test]
    fn ws_params_reject_removed_replay_fields() {
        let json = r#"{"user_id":"user-1","client_id":"c1","conversation_id":"conv-1","last_seen_seq":42}"#;
        assert!(serde_json::from_str::<WsParams>(json).is_err());
    }
}
