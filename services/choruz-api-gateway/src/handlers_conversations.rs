use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;

use crate::{
    ApiError, ApiState, authenticated_principal, db_persist, flush_webhooks, require_actor,
    require_human_operator, require_self,
};

// ── List conversations ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct ConversationsQuery {
    principal_id: String,
}

pub(crate) async fn list_conversations(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Query(query): Query<ConversationsQuery>,
) -> Result<Json<Vec<choruz_domain::Conversation>>, ApiError> {
    require_self(&headers, &state, &query.principal_id).await?;
    let conversations = state.db.list_conversations(&query.principal_id).await?;
    Ok(Json(conversations))
}

// ── Pin conversation ─────────────────────────────────────────────────

pub(crate) async fn pin_conversation(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    if matches!(
        principal.principal_type,
        choruz_domain::PrincipalType::Agent
    ) {
        return Err(ApiError(choruz_common::AppError::Forbidden(
            "agents cannot pin conversations".into(),
        )));
    }

    state
        .db
        .pin_conversation(&principal.id, &conversation_id)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn unpin_conversation(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    if matches!(
        principal.principal_type,
        choruz_domain::PrincipalType::Agent
    ) {
        return Err(ApiError(choruz_common::AppError::Forbidden(
            "agents cannot unpin conversations".into(),
        )));
    }

    state
        .db
        .unpin_conversation(&principal.id, &conversation_id)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

// ── Archive conversation ─────────────────────────────────────────────

pub(crate) async fn archive_conversation(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    if matches!(
        principal.principal_type,
        choruz_domain::PrincipalType::Agent
    ) {
        return Err(ApiError(choruz_common::AppError::Forbidden(
            "agents cannot archive conversations".into(),
        )));
    }

    state
        .db
        .archive_conversation(&principal.id, &conversation_id)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn unarchive_conversation(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    if matches!(
        principal.principal_type,
        choruz_domain::PrincipalType::Agent
    ) {
        return Err(ApiError(choruz_common::AppError::Forbidden(
            "agents cannot unarchive conversations".into(),
        )));
    }

    state
        .db
        .unarchive_conversation(&principal.id, &conversation_id)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

// ── Hide direct Agent session ─────────────────────────────────────────

pub(crate) async fn hide_agent_session(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    if matches!(
        principal.principal_type,
        choruz_domain::PrincipalType::Agent
    ) {
        return Err(ApiError(choruz_common::AppError::Forbidden(
            "agents cannot hide conversations".into(),
        )));
    }
    state
        .db
        .hide_agent_session(&principal.id, &conversation_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn restore_hidden_agent_session(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    if matches!(
        principal.principal_type,
        choruz_domain::PrincipalType::Agent
    ) {
        return Err(ApiError(choruz_common::AppError::Forbidden(
            "agents cannot restore hidden conversations".into(),
        )));
    }
    state
        .db
        .restore_hidden_agent_session(&principal.id, &conversation_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Create direct conversation ────────────────────────────────────────

pub(crate) async fn create_direct_conversation(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Json(payload): Json<choruz_application::CreateDirectConversationRequest>,
) -> Result<(StatusCode, Json<choruz_domain::Conversation>), ApiError> {
    require_actor(&headers, &state, &payload.actor_id).await?;
    state.db.check_rate_limit(&payload.actor_id)?;
    let conversation = state.db.create_direct_conversation(payload).await?;
    let _ = flush_webhooks(&state.app).await;

    Ok((StatusCode::CREATED, Json(conversation)))
}

/// Auto-create proxy runtime bindings for agent members in a group conversation.
///
/// When agents are added to a group, they need a runtime binding so the runner
/// can deliver messages to them. This creates a proxy binding for each agent
/// member that doesn't already have one.
async fn auto_create_proxy_bindings(
    _state: &ApiState,
    _conversation: &choruz_domain::Conversation,
) {
    tracing::debug!("auto_create_proxy_bindings: no-op (proxy bindings eliminated)");
}

// ── Create group ──────────────────────────────────────────────────────

pub(crate) async fn create_group(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Json(payload): Json<choruz_application::CreateGroupRequest>,
) -> Result<(StatusCode, Json<choruz_domain::Conversation>), ApiError> {
    require_actor(&headers, &state, &payload.actor_id).await?;
    state.db.check_rate_limit(&payload.actor_id)?;
    let conversation = state.db.create_group(payload).await?;
    // Auto-create proxy bindings for agent members
    auto_create_proxy_bindings(&state, &conversation).await;
    let _ = flush_webhooks(&state.app).await;

    Ok((StatusCode::CREATED, Json(conversation)))
}

// ── Update group ──────────────────────────────────────────────────────

pub(crate) async fn update_group(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(payload): Json<choruz_application::UpdateGroupRequest>,
) -> Result<Json<choruz_domain::Conversation>, ApiError> {
    require_actor(&headers, &state, &payload.actor_id).await?;
    let conversation = state.db.update_group(&conversation_id, payload).await?;

    Ok(Json(conversation))
}

// ── Migrate conversation workspace ────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct MigrateWorkspaceRequest {
    pub(crate) actor_id: String,
    pub(crate) workspace_id: String,
}

pub(crate) async fn migrate_conversation_workspace(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(payload): Json<MigrateWorkspaceRequest>,
) -> Result<Json<choruz_domain::Conversation>, ApiError> {
    let operator = require_human_operator(&headers, &state).await?;
    if payload.actor_id != operator.id {
        return Err(ApiError(choruz_common::AppError::Forbidden(
            "authenticated principal does not match actor_id".into(),
        )));
    }
    let conversation = state.app.migrate_conversation_workspace(
        &conversation_id,
        &payload.workspace_id,
        &operator.id,
    )?;

    // Persist workspace_id change to DB
    db_persist(
        &state.event_store,
        "UPDATE conversation SET workspace_id = $1 WHERE id = $2",
        &[&payload.workspace_id, &conversation_id],
        "migrate_conversation_workspace",
    )
    .await;

    Ok(Json(conversation))
}

// ── Add group members ─────────────────────────────────────────────────

pub(crate) async fn add_group_members(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(payload): Json<choruz_application::AddGroupMembersRequest>,
) -> Result<Json<choruz_domain::Conversation>, ApiError> {
    require_actor(&headers, &state, &payload.actor_id).await?;
    let conversation = state
        .db
        .add_group_members(&conversation_id, payload)
        .await?;
    // Auto-create proxy bindings for newly added agent members
    auto_create_proxy_bindings(&state, &conversation).await;
    let _ = flush_webhooks(&state.app).await;

    Ok(Json(conversation))
}

// ── Remove group member ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct RemoveGroupMemberQuery {
    actor_id: String,
}

pub(crate) async fn remove_group_member(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path((conversation_id, principal_id)): Path<(String, String)>,
    Query(query): Query<RemoveGroupMemberQuery>,
) -> Result<Json<choruz_domain::Conversation>, ApiError> {
    require_actor(&headers, &state, &query.actor_id).await?;
    let conversation = state
        .db
        .remove_group_member(&conversation_id, &query.actor_id, &principal_id)
        .await?;
    let _ = flush_webhooks(&state.app).await;

    Ok(Json(conversation))
}
