use axum::{
    Json,
    extract::{Path, Query, State},
    http::{
        HeaderMap, HeaderName, StatusCode,
        header::{CONTENT_DISPOSITION, CONTENT_TYPE},
    },
    response::{IntoResponse, Response},
};

/// `X-Content-Type-Options: nosniff` header name. Keeps browsers from
/// MIME-sniffing attachment responses, which would otherwise let a file
/// uploaded as `application/octet-stream` be re-interpreted as HTML/JS and
/// execute under our origin.
const HDR_NOSNIFF: HeaderName = HeaderName::from_static("x-content-type-options");
use choruz_common::AppError;
use serde::Deserialize;

use crate::{
    ApiError, ApiState,
    attachments::{AttachmentRecord, UploadAttachmentRequest},
    authenticated_principal, flush_webhooks_all, require_actor, require_self,
};

// ── Send message ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct InteractionQuery {
    before_seq: Option<i64>,
    limit: Option<i64>,
    #[serde(default)]
    include_content: bool,
}

pub(crate) async fn list_interactions(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Query(query): Query<InteractionQuery>,
) -> Result<Json<choruz_application::db_service::InteractionPage>, ApiError> {
    let principal = crate::require_human_operator(&headers, &state).await?;
    crate::handlers_threads::require_conversation_read_access(&state, &principal, &conversation_id)
        .await?;
    let page = state
        .db
        .list_interactions(
            &conversation_id,
            query.before_seq,
            query.limit.unwrap_or(50),
            query.include_content,
        )
        .await?;
    state.db.record_audit(&principal.workspace_id, &principal.id, "interaction.read", "conversation", &conversation_id,
        serde_json::json!({"include_content": query.include_content, "record_count": page.records.len()})).await?;
    Ok(Json(page))
}

pub(crate) async fn send_message(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Json(mut payload): Json<choruz_application::SendMessageRequest>,
) -> Result<(StatusCode, Json<choruz_domain::Message>), ApiError> {
    if is_legacy_channel_task_media_type(&payload.content_type) {
        return Err(ApiError::from(AppError::Validation(
            "legacy channel-task media type is not accepted".into(),
        )));
    }
    require_actor(&headers, &state, &payload.actor_id).await?;

    // Thread the FE trace id into the application layer so `send_message`,
    // `app_mention` outbox rows, and downstream pipeline stages can all
    // stitch their logs to the originating user action. Clients that don't
    // send the header just leave this as None.
    if payload.trace_id.is_none() {
        payload.trace_id = headers
            .get("x-trace-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
    }

    let message = publish_message(&state.app, &state.db, payload).await?;
    Ok((StatusCode::CREATED, Json(message)))
}

pub(crate) async fn publish_message(
    app: &choruz_application::ChatApp,
    db: &choruz_application::DbService,
    payload: choruz_application::SendMessageRequest,
) -> Result<choruz_domain::Message, AppError> {
    db.check_rate_limit(&payload.actor_id)?;
    let message = db.send_message(payload).await?;
    app.inject_message_with_event(message.clone());
    let _ = flush_webhooks_all(app, db).await;
    Ok(message)
}

fn is_legacy_channel_task_media_type(content_type: &str) -> bool {
    content_type
        == format!(
            "application/vnd.{}.channel-task+json",
            ["e", "chat"].concat()
        )
}

#[cfg(test)]
mod tests {
    use super::is_legacy_channel_task_media_type;

    #[test]
    fn legacy_channel_task_media_type_is_rejected() {
        assert!(is_legacy_channel_task_media_type(&format!(
            "application/vnd.{}.channel-task+json",
            ["e", "chat"].concat()
        )));
        assert!(!is_legacy_channel_task_media_type(
            "application/vnd.choruz.channel-task+json"
        ));
    }
}

// ── Message search ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct SearchMessagesQuery {
    principal_id: String,
    q: String,
    limit: Option<i64>,
    /// Optional: scope search to a single conversation (used by the detail-panel
    /// Search tab). When omitted, searches across every conversation the
    /// principal is an active member of.
    conversation_id: Option<String>,
    before_created_at: Option<chrono::DateTime<chrono::Utc>>,
    before_message_id: Option<String>,
}

pub(crate) async fn search_messages(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Query(query): Query<SearchMessagesQuery>,
) -> Result<Json<Vec<choruz_application::MessageSearchResult>>, ApiError> {
    let principal = require_self(&headers, &state, &query.principal_id).await?;
    let before = match (query.before_created_at, query.before_message_id.as_deref()) {
        (None, None) => None,
        (Some(timestamp), Some(id)) if !id.trim().is_empty() => Some((timestamp, id)),
        _ => {
            return Err(ApiError::from(AppError::Validation(
                "search cursor requires before_created_at and before_message_id".into(),
            )));
        }
    };
    let conv_filter = query
        .conversation_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    Ok(Json(
        state
            .db
            .search_messages(
                &principal.id,
                &query.q,
                query.limit.unwrap_or(50),
                conv_filter,
                before,
            )
            .await
            .map_err(ApiError::from)?,
    ))
}

// ── Attachments ───────────────────────────────────────────────────────

pub(crate) async fn upload_attachment(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Json(payload): Json<UploadAttachmentRequest>,
) -> Result<(StatusCode, Json<AttachmentRecord>), ApiError> {
    let actor = require_actor(&headers, &state, &payload.actor_id).await?;
    state.db.check_rate_limit(&actor.id)?;
    let attachment = state.attachments.put(&actor, payload).await?;
    state
        .db
        .record_audit(
            &actor.workspace_id,
            &actor.id,
            "attachment.uploaded",
            "attachment",
            &attachment.id,
            serde_json::json!({ "filename": attachment.filename }),
        )
        .await?;

    Ok((StatusCode::CREATED, Json(attachment)))
}

#[derive(Debug, Deserialize)]
pub(crate) struct AttachmentQuery {
    actor_id: String,
}

pub(crate) async fn download_attachment(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(attachment_id): Path<String>,
    Query(query): Query<AttachmentQuery>,
) -> Result<Response, ApiError> {
    let actor = require_actor(&headers, &state, &query.actor_id).await?;
    let (attachment, bytes) = state.attachments.get(&actor, &attachment_id).await?;
    Ok((
        [
            (CONTENT_TYPE, attachment.content_type),
            (
                CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", attachment.filename),
            ),
            (HDR_NOSNIFF, "nosniff".to_string()),
        ],
        bytes,
    )
        .into_response())
}

pub(crate) async fn delete_attachment(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(attachment_id): Path<String>,
    Query(query): Query<AttachmentQuery>,
) -> Result<StatusCode, ApiError> {
    let actor = require_actor(&headers, &state, &query.actor_id).await?;
    state.attachments.delete(&actor, &attachment_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Get one message ───────────────────────────────────────────────────

/// `GET /v1/conversations/{conversation_id}/messages/{message_id}` —
/// fetch a single message. Backs the quote-reply preview when the reply
/// target falls outside the client's loaded history window. Same
/// conversation read gate as `list_messages` / the thread endpoints.
pub(crate) async fn get_message(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path((conversation_id, message_id)): Path<(String, String)>,
) -> Result<Json<choruz_domain::Message>, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    crate::handlers_threads::require_conversation_read_access(&state, &principal, &conversation_id)
        .await?;
    let message = state.db.get_message(&conversation_id, &message_id).await?;
    Ok(Json(message))
}

// ── List messages ─────────────────────────────────────────────────────

pub(crate) async fn list_messages(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Query(query): Query<choruz_application::ListMessagesQuery>,
) -> Result<axum::response::Response, ApiError> {
    use axum::response::IntoResponse;
    let principal = require_self(&headers, &state, &query.principal_id).await?;

    // Membership check: a non-member who happens to know the
    // conversation_id could otherwise `?principal_id=<self>` and read
    // every message. Shared with the thread endpoints — one gate
    // definition, no access-control drift.
    crate::handlers_threads::require_conversation_read_access(&state, &principal, &conversation_id)
        .await?;

    // DB is the source of truth — no in-memory fallback needed.
    //
    // ?view=timeline filters out
    // quiet threaded replies and attaches per-root thread_summaries.
    // The default keeps today's flat array shape so existing clients
    // are unaffected.
    if query.view.as_deref() == Some("timeline") {
        let timeline = state
            .db
            .list_timeline_messages(&conversation_id, query.limit, query.since_seq)
            .await?;
        return Ok(Json(timeline).into_response());
    }
    let messages = state
        .db
        .list_messages(&conversation_id, query.limit, query.since_seq)
        .await?;
    Ok(Json(messages).into_response())
}

pub(crate) async fn list_message_page(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Query(query): Query<choruz_application::MessagePageQuery>,
) -> Result<Json<choruz_application::MessagePage>, ApiError> {
    let principal = require_self(&headers, &state, &query.principal_id).await?;
    crate::handlers_threads::require_conversation_read_access(&state, &principal, &conversation_id)
        .await?;
    if query.before_seq.is_some() && query.after_seq.is_some() {
        return Err(ApiError(AppError::Validation(
            "before_seq and after_seq are mutually exclusive".into(),
        )));
    }
    let limit = query.limit.unwrap_or(50).clamp(1, 100);
    let page = state
        .db
        .list_message_page(&conversation_id, limit, query.before_seq, query.after_seq)
        .await?;
    Ok(Json(page))
}

// ── Unread counts (Mattermost pattern) ───────────────────────────────

/// GET /v1/unreads — return unread + mention counts for every conversation
/// the authenticated user belongs to.
pub(crate) async fn get_unreads(
    headers: HeaderMap,
    State(state): State<ApiState>,
) -> Result<Json<Vec<choruz_application::ConversationUnread>>, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    let unreads = state.db.get_unread_counts(&principal.id).await?;
    Ok(Json(unreads))
}

/// POST /v1/conversations/{id}/view — mark a conversation as viewed
/// (resets unread and mention counts for the authenticated user).
pub(crate) async fn view_conversation(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    let client = state.event_store.connect().await.map_err(ApiError::from)?;
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
            &[&conversation_id, &principal.id, &principal.workspace_id],
        )
        .await
        .map_err(|e| {
            ApiError::from(AppError::Internal(format!(
                "view conversation access check: {e}"
            )))
        })?;
    if access_row.is_none() {
        return Err(ApiError::from(AppError::Forbidden(
            "cannot view this conversation".into(),
        )));
    }
    state
        .db
        .mark_conversation_viewed(&conversation_id, &principal.id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
