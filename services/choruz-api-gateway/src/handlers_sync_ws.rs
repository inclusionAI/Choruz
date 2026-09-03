use std::time::Duration;

use axum::{
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::HeaderMap,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};

use crate::{ApiError, ApiState, authenticated_principal, sync_wakeup::is_relevant_wakeup};

const PAGE_SIZE: u32 = 200;

#[derive(Debug, Deserialize)]
pub(crate) struct SyncSocketQuery {
    device_id: String,
    #[serde(default)]
    cursor: u64,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientFrame {
    SyncAck { cursor: u64 },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerFrame {
    #[serde(rename = "sync_ready")]
    Ready {
        device_id: String,
        cursor: u64,
        head_cursor: u64,
    },
    #[serde(rename = "sync_changes")]
    Changes {
        changes: Vec<choruz_application::SyncChange>,
        next_cursor: u64,
        head_cursor: u64,
        has_more: bool,
    },
    #[serde(rename = "sync_acked")]
    Acked { cursor: u64 },
    #[serde(rename = "sync_error")]
    Error { detail: String },
}

pub(crate) async fn sync_socket(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    State(state): State<ApiState>,
    Query(query): Query<SyncSocketQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    state.sync_wakeups.wait_ready().await.map_err(|error| {
        ApiError(choruz_common::AppError::Internal(format!(
            "dashboard sync unavailable: {error}"
        )))
    })?;
    let cursor = state
        .db
        .register_sync_device(&principal.id, &query.device_id, query.cursor)
        .await?;
    let head_cursor = state.db.current_sync_cursor(&principal.id).await?;
    let wakeups = state.sync_wakeups.subscribe();
    Ok(ws.on_upgrade(move |socket| {
        serve_socket(
            socket,
            state,
            principal.id,
            query.device_id,
            cursor,
            head_cursor,
            wakeups,
        )
    }))
}

async fn send_frame(socket: &mut WebSocket, frame: &ServerFrame) -> Result<(), ()> {
    let text = serde_json::to_string(frame).map_err(|_| ())?;
    socket
        .send(Message::Text(text.into()))
        .await
        .map_err(|_| ())
}

async fn drain_changes(
    socket: &mut WebSocket,
    state: &ApiState,
    principal_id: &str,
    last_sent: &mut u64,
) -> Result<(), ()> {
    loop {
        let page = state
            .db
            .list_sync_changes(principal_id, *last_sent, PAGE_SIZE)
            .await
            .map_err(|error| {
                tracing::warn!(%error, %principal_id, "dashboard sync read failed");
            })?;
        if page.changes.is_empty() {
            return Ok(());
        }
        let has_more = page.has_more;
        *last_sent = page.next_cursor;
        send_frame(
            socket,
            &ServerFrame::Changes {
                changes: page.changes,
                next_cursor: page.next_cursor,
                head_cursor: page.head_cursor,
                has_more,
            },
        )
        .await?;
        if !has_more {
            return Ok(());
        }
    }
}

async fn serve_socket(
    mut socket: WebSocket,
    state: ApiState,
    principal_id: String,
    device_id: String,
    acknowledged: u64,
    head_cursor: u64,
    mut wakeups: tokio::sync::broadcast::Receiver<String>,
) {
    let mut last_sent = acknowledged;
    if send_frame(
        &mut socket,
        &ServerFrame::Ready {
            device_id: device_id.clone(),
            cursor: acknowledged,
            head_cursor,
        },
    )
    .await
    .is_err()
        || drain_changes(&mut socket, &state, &principal_id, &mut last_sent)
            .await
            .is_err()
    {
        return;
    }

    let mut heartbeat = tokio::time::interval(Duration::from_secs(30));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    heartbeat.tick().await;

    loop {
        tokio::select! {
            incoming = socket.recv() => {
                let Some(Ok(incoming)) = incoming else { break };
                match incoming {
                    Message::Text(text) => {
                        let Ok(ClientFrame::SyncAck { cursor }) = serde_json::from_str(&text) else {
                            let _ = send_frame(&mut socket, &ServerFrame::Error {
                                detail: "invalid sync frame".into(),
                            }).await;
                            continue;
                        };
                        if cursor > last_sent {
                            let _ = send_frame(&mut socket, &ServerFrame::Error {
                                detail: "cannot acknowledge an unsent cursor".into(),
                            }).await;
                            break;
                        }
                        match state.db.acknowledge_sync_device(&principal_id, &device_id, cursor).await {
                            Ok(cursor) => {
                                if send_frame(&mut socket, &ServerFrame::Acked { cursor }).await.is_err() {
                                    break;
                                }
                            }
                            Err(error) => {
                                tracing::warn!(%error, %principal_id, %device_id, "dashboard sync ACK failed");
                                let _ = send_frame(&mut socket, &ServerFrame::Error {
                                    detail: "sync acknowledgement failed".into(),
                                }).await;
                                break;
                            }
                        }
                    }
                    Message::Ping(bytes) => {
                        if socket.send(Message::Pong(bytes)).await.is_err() { break; }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            wakeup = wakeups.recv() => {
                match wakeup {
                    Ok(payload) if is_relevant_wakeup(&payload, &principal_id) => {
                        if drain_changes(&mut socket, &state, &principal_id, &mut last_sent).await.is_err() { break; }
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        if drain_changes(&mut socket, &state, &principal_id, &mut last_sent).await.is_err() { break; }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            _ = heartbeat.tick() => {
                if socket.send(Message::Ping(Vec::new().into())).await.is_err() { break; }
            }
        }
    }
}
