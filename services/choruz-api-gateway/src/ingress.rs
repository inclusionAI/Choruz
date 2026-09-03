//! Ingress API: accepts inbound messages and writes them to the
//! conversation event store + outbox in a single transaction.
//!
//! This is Phase B3 of the message pipeline.

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use choruz_common::AppError;
use choruz_ids::MessageId;
use choruz_store::conversation_events::ConversationEvent;
use serde::{Deserialize, Serialize};

use crate::{ApiError, ApiState, authenticated_principal};

/// Request body for the ingress endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IngestRequest {
    /// Which conversation this message belongs to.
    pub conversation_id: String,
    /// Message content.
    pub content: String,
    /// MIME content type (defaults to `text/plain`).
    #[serde(default = "default_content_type")]
    pub content_type: String,
    /// Client-generated message ID for retry dedup.
    pub client_msg_id: Option<String>,
    /// Arbitrary metadata.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

fn default_content_type() -> String {
    "text/plain".into()
}

/// Response body for the ingress endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct IngestResponse {
    pub message_id: String,
    pub seq: i64,
    /// `true` if this was a duplicate (deduped by client_msg_id).
    pub deduplicated: bool,
}

/// Ingest a message: writes to conversation_events + event_outbox in a
/// single database transaction.
///
/// Route: `POST /v2/ingest` — **authenticated**. The sender is derived from
/// the session (Bearer token or `choruz_session` cookie). Callers must be a
/// current member of the target conversation.
pub(crate) async fn ingest_message(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Json(req): Json<IngestRequest>,
) -> Result<(StatusCode, Json<IngestResponse>), ApiError> {
    // 1. Authenticate. This is the canonical sender_id — never the body.
    let principal = authenticated_principal(&headers, &state).await?;
    // 2. Rate limit per-principal (same window as the other write paths).
    state.db.check_rate_limit(&principal.id)?;

    // 3. Access check. Callers must be a live member of the target
    //    conversation and still have access to its workspace.
    let store = &state.event_store;
    let client = store.connect().await.map_err(ApiError::from)?;
    let access_row = client
        .query_opt(
            "SELECT 1
             FROM conversation c
             LEFT JOIN company co ON co.id = c.workspace_id
             LEFT JOIN company_member com
               ON com.company_id = co.id AND com.principal_id = $2
             JOIN conversation_member cm
               ON cm.conv_id = c.id AND cm.principal_id = $2 AND cm.removed_at IS NULL
             WHERE c.id = $1
               AND ((co.id IS NULL AND c.workspace_id = $3)
                    OR (co.deleted_at IS NULL
                        AND (c.workspace_id = $3 OR com.principal_id IS NOT NULL)))",
            &[&req.conversation_id, &principal.id, &principal.workspace_id],
        )
        .await
        .map_err(|e| ApiError::from(AppError::Internal(format!("ingest access check: {e}"))))?;
    if access_row.is_none() {
        return Err(ApiError::from(AppError::Forbidden(
            "sender cannot access this conversation".into(),
        )));
    }

    // 4. Idempotency check: if client_msg_id is provided, look for existing event.
    if let Some(ref client_msg_id) = req.client_msg_id {
        match store.find_event_by_client_msg_id(client_msg_id).await {
            Ok(Some(existing)) => {
                return Ok((
                    StatusCode::OK,
                    Json(IngestResponse {
                        message_id: existing.event_id,
                        seq: existing.seq,
                        deduplicated: true,
                    }),
                ));
            }
            Ok(None) => {} // Not a duplicate, proceed.
            Err(e) => {
                return Err(ApiError::from(AppError::Internal(format!(
                    "ingest dedup check: {e}"
                ))));
            }
        }
    }

    let sender_id = principal.id.clone();

    // 2. Generate a server-side message_id (UUIDv7).
    let message_id = MessageId::new();
    let message_id_str = message_id.to_string();

    // 5. Open a transaction and write both tables atomically.
    let mut client = store
        .connect()
        .await
        .map_err(|e| ApiError::from(AppError::Internal(format!("ingest connect: {e}"))))?;
    let tx = client
        .transaction()
        .await
        .map_err(|e| ApiError::from(AppError::Internal(format!("ingest begin tx: {e}"))))?;

    // 5a. Insert conversation event (struct retained so ConversationEvent
    //     stays exercised — the actual INSERT goes through the SQL below
    //     to share the advisory-lock seq allocator).
    let _event = ConversationEvent {
        conversation_id: req.conversation_id.clone(),
        event_id: message_id_str.clone(),
        event_type: "message".into(),
        sender_id: sender_id.clone(),
        content: Some(req.content.clone()),
        content_type: req.content_type.clone(),
        metadata: req.metadata.clone(),
        client_msg_id: req.client_msg_id.clone(),
        turn_id: None,
        reply_event_id: None,
    };

    let content: Option<String> = Some(req.content.clone());
    let event_type_str = "message".to_string();
    let turn_id: Option<String> = None;
    let reply_event_id: Option<String> = None;

    // Serialize concurrent writers targeting the same conversation so the
    // COALESCE(MAX(seq), 0) + 1 allocation below cannot race to the same
    // value and collide on the (conversation_id, seq) unique constraint.
    tx.execute(
        "SELECT pg_advisory_xact_lock(hashtext($1)::bigint)",
        &[&req.conversation_id],
    )
    .await
    .map_err(|e| ApiError::from(AppError::Internal(format!("ingest advisory lock: {e}"))))?;

    let row = tx
        .query_one(
            "INSERT INTO conversation_events
                (conversation_id, seq, event_id, event_type, sender_id,
                 content, content_type, metadata, client_msg_id, turn_id,
                 reply_event_id, created_at)
             VALUES (
                $1,
                COALESCE((SELECT MAX(seq) FROM conversation_events WHERE conversation_id = $1), 0) + 1,
                $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW()
             )
             RETURNING event_id, seq",
            &[
                &req.conversation_id,
                &message_id_str,
                &event_type_str,
                &sender_id,
                &content,
                &req.content_type,
                &req.metadata,
                &req.client_msg_id,
                &turn_id,
                &reply_event_id,
            ],
        )
        .await
        .map_err(|e| {
            let is_unique_violation = e.as_db_error().is_some_and(|db_err| {
                *db_err.code() == tokio_postgres::error::SqlState::UNIQUE_VIOLATION
            });
            if is_unique_violation {
                ApiError::from(AppError::Conflict("duplicate message".into()))
            } else {
                ApiError::from(AppError::Internal(format!(
                    "insert conversation event: {e}"
                )))
            }
        })?;

    let seq: i64 = row.get(1);

    // 5b. Insert outbox entry.
    let outbox_payload = serde_json::json!({
        "message_id": message_id_str,
        "conversation_id": req.conversation_id,
        "sender_id": sender_id,
        "content": req.content,
        "content_type": req.content_type,
        "seq": seq,
        "metadata": req.metadata,
    });

    let aggregate_type = "conversation_event".to_string();
    let outbox_event_type = "message".to_string();

    tx.execute(
        "INSERT INTO event_outbox
            (aggregate_type, aggregate_id, event_type, payload, created_at, published)
         VALUES ($1, $2, $3, $4, NOW(), FALSE)",
        &[
            &aggregate_type,
            &req.conversation_id,
            &outbox_event_type,
            &outbox_payload,
        ],
    )
    .await
    .map_err(|e| ApiError::from(AppError::Internal(format!("insert outbox entry: {e}"))))?;

    // 5c. Increment conversation.total_msg_count for unread tracking
    // (Mattermost pattern — keeps /v1/unreads and console snapshot
    // counts correct for writes that bypass DbService::send_message).
    tx.execute(
        "UPDATE conversation SET total_msg_count = total_msg_count + 1 WHERE id = $1",
        &[&req.conversation_id],
    )
    .await
    .map_err(|e| {
        ApiError::from(AppError::Internal(format!(
            "increment total_msg_count: {e}"
        )))
    })?;

    // 6. Commit the transaction.
    tx.commit()
        .await
        .map_err(|e| ApiError::from(AppError::Internal(format!("ingest commit: {e}"))))?;

    tracing::info!(
        message_id = %message_id_str,
        conversation_id = %req.conversation_id,
        seq,
        "ingest: message accepted"
    );

    Ok((
        StatusCode::CREATED,
        Json(IngestResponse {
            message_id: message_id_str,
            seq,
            deduplicated: false,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ingest_request_deserialize() {
        let json = serde_json::json!({
            "conversation_id": "conv-1",
            "content": "hello",
            "client_msg_id": "client-abc"
        });
        let req: IngestRequest = serde_json::from_value(json).expect("deserialize");
        assert_eq!(req.content_type, "text/plain"); // default
        assert_eq!(req.client_msg_id, Some("client-abc".into()));
    }

    #[test]
    fn ingest_request_without_optional_fields() {
        let json = serde_json::json!({
            "conversation_id": "conv-1",
            "content": "hello"
        });
        let req: IngestRequest = serde_json::from_value(json).expect("deserialize");
        assert!(req.client_msg_id.is_none());
        assert_eq!(req.metadata, serde_json::json!(null));
    }

    #[test]
    fn ingest_request_rejects_sender_id() {
        let json = serde_json::json!({
            "sender_id": "spoofed",
            "conversation_id": "conv-1",
            "content": "hello"
        });
        assert!(serde_json::from_value::<IngestRequest>(json).is_err());
    }

    #[test]
    fn ingest_response_serialize() {
        let resp = IngestResponse {
            message_id: "msg-1".into(),
            seq: 42,
            deduplicated: false,
        };
        let json = serde_json::to_value(&resp).expect("serialize");
        assert_eq!(json["message_id"], "msg-1");
        assert_eq!(json["seq"], 42);
        assert_eq!(json["deduplicated"], false);
    }
}
