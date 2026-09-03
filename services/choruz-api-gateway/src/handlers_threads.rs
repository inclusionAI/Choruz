//! Thread read/view endpoints.
//!
//! Threaded replies are WRITTEN through the existing `POST /v1/messages`
//! with `metadata: {reply_to_id, thread: true, broadcast?}` — no new
//! write endpoint. These handlers cover the read side:
//!
//!   GET  /v1/conversations/{id}/threads/{root}        — root + replies
//!   POST /v1/conversations/{id}/threads/{root}/view   — receipt upsert
//!
//! Authorization mirrors `list_messages`: active conversation membership,
//! checked against the live conversation. Thread visibility
//! equals conversation visibility — there is no thread-level membership.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use choruz_common::AppError;
use serde::Deserialize;

use crate::{ApiError, ApiState, authenticated_principal};

#[derive(Debug, Deserialize)]
pub(crate) struct ThreadQuery {
    #[serde(default)]
    pub since_seq: Option<u64>,
    #[serde(default)]
    pub limit: Option<u64>,
}

/// Shared conversation read gate: the conversation must be live, and a
/// caller must hold an active membership row reachable
/// through a non-deleted company (or the legacy same-workspace path).
/// Used by the thread endpoints here AND by
/// `handlers_messages::list_messages` — one definition so the two
/// read surfaces cannot drift (access-control drift across phases is
/// the failure mode this guards).
pub(crate) async fn require_conversation_read_access(
    state: &ApiState,
    principal: &choruz_domain::Principal,
    conversation_id: &str,
) -> Result<(), ApiError> {
    let client = state.event_store.connect().await.map_err(ApiError::from)?;
    let live = client
        .query_opt(
            "SELECT 1
             FROM conversation c
             LEFT JOIN company co ON co.id = c.workspace_id
             WHERE c.id = $1
               AND (co.id IS NULL OR co.deleted_at IS NULL)",
            &[&conversation_id],
        )
        .await
        .map_err(|e| ApiError::from(AppError::Internal(format!("thread conv check: {e}"))))?;
    if live.is_none() {
        return Err(ApiError::from(AppError::Forbidden(
            "conversation is not accessible".into(),
        )));
    }
    let member = client
        .query_opt(
            "SELECT 1
                 FROM conversation_member cm
                 JOIN conversation c ON c.id = cm.conv_id
                 LEFT JOIN company co ON co.id = c.workspace_id
                 LEFT JOIN company_member com
                   ON com.company_id = co.id AND com.principal_id = $2
                 WHERE cm.conv_id = $1
                   AND cm.principal_id = $2
                   AND cm.removed_at IS NULL
                   AND ((co.id IS NULL AND c.workspace_id = $3) OR (co.deleted_at IS NULL
                        AND (c.workspace_id = $3 OR com.principal_id IS NOT NULL)))",
            &[&conversation_id, &principal.id, &principal.workspace_id],
        )
        .await
        .map_err(|e| ApiError::from(AppError::Internal(format!("thread member check: {e}"))))?;
    if member.is_none() {
        return Err(ApiError::from(AppError::Forbidden(
            "not a member of this conversation".into(),
        )));
    }
    Ok(())
}

/// `GET /v1/conversations/{conversation_id}/threads/{thread_root_id}` —
/// the root message plus its threaded replies in seq order.
pub(crate) async fn get_thread(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path((conversation_id, thread_root_id)): Path<(String, String)>,
    Query(query): Query<ThreadQuery>,
) -> Result<Json<choruz_application::ThreadDetail>, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    require_conversation_read_access(&state, &principal, &conversation_id).await?;
    let detail = state
        .db
        .list_thread_replies(
            &conversation_id,
            &thread_root_id,
            query.limit,
            query.since_seq,
        )
        .await?;
    Ok(Json(detail))
}

/// `POST /v1/conversations/{conversation_id}/threads/{thread_root_id}/view`
/// — upsert the caller's thread read receipt (clears the thread's unread
/// dot). Mirrors `POST /v1/conversations/{id}/view`.
pub(crate) async fn view_thread(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path((conversation_id, thread_root_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    state.db.check_rate_limit(&principal.id)?;
    require_conversation_read_access(&state, &principal, &conversation_id).await?;
    state
        .db
        .mark_thread_viewed(&conversation_id, &thread_root_id, &principal.id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
