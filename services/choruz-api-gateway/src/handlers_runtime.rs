use std::collections::HashSet;

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use choruz_agent_runtime::{
    AuditActor, AutoMode, BindingState, ConversationRuntimePolicy, CreateBindingInput, DriverType,
    RuntimeBinding, UntaggedHumanMode, UpsertPolicyInput,
};
use choruz_application::DbService;
use choruz_common::AppError;
use choruz_domain::{Conversation, ConversationType, Principal, PrincipalType};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_json::json;

use crate::{
    ApiError, ApiState, authenticated_principal, redact_sensitive_text, require_human_operator,
};

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RuntimeBindingView {
    id: String,
    workspace_id: String,
    conversation_id: String,
    conversation_name: String,
    conversation_type: &'static str,
    agent_principal_id: String,
    agent_name: String,
    driver_type: DriverType,
    interaction_mode: Option<String>,
    runtime_host_id: Option<String>,
    harness_account_id: Option<String>,
    harness_account_name: Option<String>,
    workspace_path: String,
    git_worktree_path: Option<String>,
    external_session_id: Option<String>,
    external_thread_id: Option<String>,
    last_event_cursor: i64,
    last_acked_event_cursor: i64,
    last_seen_server_seq: i64,
    state: BindingState,
    last_error: Option<String>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateRuntimeBindingRequest {
    conversation_id: String,
    agent_principal_id: String,
    driver_type: DriverType,
    workspace_path: String,
    git_worktree_path: Option<String>,
    config_json: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RebindRuntimeBindingRequest {
    workspace_path: String,
}

pub(crate) fn runtime_audit_actor(principal: &Principal) -> AuditActor {
    AuditActor {
        actor_id: principal.id.clone(),
        workspace_id: principal.workspace_id.clone(),
    }
}

pub(crate) fn runtime_binding_config_for_agent(
    agent: &Principal,
    config_json: Option<serde_json::Value>,
) -> serde_json::Value {
    let mut config = config_json
        .unwrap_or_else(|| json!({}))
        .as_object()
        .cloned()
        .unwrap_or_default();
    let mut mention_aliases = config
        .get("mention_aliases")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    if !mention_aliases
        .iter()
        .filter_map(serde_json::Value::as_str)
        .any(|alias| alias == agent.name)
    {
        mention_aliases.push(json!(agent.name));
    }
    config.insert(
        "mention_aliases".into(),
        serde_json::Value::Array(mention_aliases),
    );
    config
        .entry("agent_name")
        .or_insert_with(|| json!(agent.name));
    serde_json::Value::Object(config)
}

fn validate_runtime_binding_config(
    config_json: Option<&serde_json::Value>,
) -> Result<(), AppError> {
    let Some(model) = config_json.and_then(|config| config.get("model")) else {
        return Ok(());
    };
    let model = model
        .as_str()
        .ok_or_else(|| AppError::Validation("config_json.model must be a string".into()))?
        .trim();
    if model.is_empty() {
        return Ok(());
    }
    if model.len() > 256 {
        return Err(AppError::Validation(
            "config_json.model must be 256 bytes or fewer".into(),
        ));
    }
    if model.starts_with('-') || model.chars().any(char::is_control) {
        return Err(AppError::Validation(
            "config_json.model contains unsafe characters".into(),
        ));
    }
    Ok(())
}

async fn validate_runtime_host(
    state: &ApiState,
    workspace_id: &str,
    config_json: Option<&serde_json::Value>,
) -> Result<Option<String>, ApiError> {
    let Some(value) = config_json.and_then(|config| config.get("runtime_host_id")) else {
        return Ok(None);
    };
    let runtime_host_id = value.as_str().ok_or_else(|| {
        ApiError(AppError::Validation(
            "config_json.runtime_host_id must be a string".into(),
        ))
    })?;
    let runtime_host_id = runtime_host_id.trim();
    if runtime_host_id.is_empty() {
        return Err(ApiError(AppError::Validation(
            "config_json.runtime_host_id must not be empty".into(),
        )));
    }
    let client = state.event_store.connect().await.map_err(ApiError::from)?;
    let exists = client
        .query_opt(
            "SELECT 1 FROM runtime_host
             WHERE id = $1 AND company_id = $2 AND revoked_at IS NULL",
            &[&runtime_host_id, &workspace_id],
        )
        .await
        .map_err(|error| {
            ApiError(AppError::Internal(format!(
                "validate runtime host binding: {error}"
            )))
        })?
        .is_some();
    if !exists {
        return Err(ApiError(AppError::Validation(
            "runtime host does not belong to the agent's company".into(),
        )));
    }
    Ok(Some(runtime_host_id.to_owned()))
}

async fn validate_harness_account(
    state: &ApiState,
    workspace_id: &str,
    driver_type: DriverType,
    runtime_host_id: Option<&str>,
    config_json: Option<&serde_json::Value>,
) -> Result<Option<(String, String, String)>, ApiError> {
    let Some(value) = config_json.and_then(|config| config.get("harness_account_id")) else {
        if config_json.is_some_and(|config| {
            config.get("harness_account_name").is_some()
                || config.get("harness_account_profile_kind").is_some()
        }) {
            return Err(ApiError(AppError::Validation(
                "harness account metadata requires harness_account_id".into(),
            )));
        }
        return Ok(None);
    };
    let id = value.as_str().ok_or_else(|| {
        ApiError(AppError::Validation(
            "config_json.harness_account_id must be a string".into(),
        ))
    })?;
    let model = config_json
        .and_then(|config| config.get("model"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let client = state.event_store.connect().await.map_err(ApiError::from)?;
    let row = client
        .query_opt(
            "SELECT name, profile_kind FROM harness_account
              WHERE id = $1 AND company_id = $2 AND driver_type = $3
                AND runtime_host_id IS NOT DISTINCT FROM $4
                AND status = 'active' AND disabled_at IS NULL
                AND ($5::text IS NULL OR models_json @> jsonb_build_array(jsonb_build_object('id', $5::text)))",
            &[&id, &workspace_id, &driver_type.as_str(), &runtime_host_id, &model],
        )
        .await
        .map_err(|error| {
            ApiError(AppError::Internal(format!(
                "validate harness account: {error}"
            )))
        })?;
    let Some(row) = row else {
        return Err(ApiError(AppError::Validation(
            "harness account is inactive, belongs to a different company/device/driver, or does not offer the selected model"
                .into(),
        )));
    };
    Ok(Some((
        id.to_owned(),
        row.get("name"),
        row.get("profile_kind"),
    )))
}

/// Collect all workspace IDs that `principal_id` may access (own workspace +
/// every company they belong to).
pub(crate) async fn accessible_workspace_ids(
    db: &DbService,
    principal_id: &str,
) -> Result<HashSet<String>, ApiError> {
    let mut ids = HashSet::new();
    if let Ok(p) = db.get_principal(principal_id).await {
        ids.insert(p.workspace_id.clone());
    }
    if let Ok(companies) = db.list_companies(principal_id).await {
        for c in companies {
            ids.insert(c.id);
        }
    }
    Ok(ids)
}

pub(crate) async fn binding_view_for_workspace(
    db: &DbService,
    binding: RuntimeBinding,
    allowed_workspaces: &HashSet<String>,
) -> Result<RuntimeBindingView, ApiError> {
    let agent = db.get_principal(&binding.agent_principal_id).await?;
    let conversation = db.get_conversation(&binding.conversation_id).await?;
    if !allowed_workspaces.contains(&agent.workspace_id)
        || !allowed_workspaces.contains(&conversation.workspace_id)
    {
        return Err(ApiError(AppError::Forbidden(
            "cross-workspace access denied".into(),
        )));
    }
    let conversation_name = conversation_label(db, &conversation).await;
    let conversation_type = conversation_type_label(&conversation.conversation_type);

    let interaction_mode = Some(effective_interaction_mode(
        &binding.driver_type,
        &binding.config_json,
    ));
    let runtime_host_id = binding
        .config_json
        .get("runtime_host_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let harness_account_id = binding
        .config_json
        .get("harness_account_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let harness_account_name = binding
        .config_json
        .get("harness_account_name")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    Ok(RuntimeBindingView {
        id: binding.id,
        workspace_id: agent.workspace_id.clone(),
        conversation_id: conversation.id,
        conversation_name,
        conversation_type,
        agent_principal_id: agent.id,
        agent_name: agent.name,
        driver_type: binding.driver_type,
        interaction_mode,
        runtime_host_id,
        harness_account_id,
        harness_account_name,
        workspace_path: binding.workspace_path,
        git_worktree_path: binding.git_worktree_path,
        external_session_id: binding.external_session_id,
        external_thread_id: binding.external_thread_id,
        last_event_cursor: binding.last_event_cursor,
        last_acked_event_cursor: binding.last_acked_event_cursor,
        last_seen_server_seq: binding.last_seen_server_seq,
        state: binding.state,
        last_error: binding.last_error.as_deref().map(redact_sensitive_text),
        updated_at: binding.updated_at,
    })
}

/// The transcript a binding's conversation shows: the stored
/// `interaction_mode`, else `terminal` for a driver the gateway serves over a
/// PTY and `message` otherwise. Clients never derive this from the driver
/// name, so a plugin driver needs no client-side list.
pub(crate) fn effective_interaction_mode(driver_type: &DriverType, config_json: &Value) -> String {
    match config_json
        .get("interaction_mode")
        .and_then(Value::as_str)
        .filter(|mode| !mode.trim().is_empty())
    {
        Some(mode) => mode.to_owned(),
        None if crate::handlers_terminals::is_terminal_driver(driver_type) => "terminal".to_owned(),
        None => "message".to_owned(),
    }
}

const BINDING_VIEW_SQL: &str =
    "SELECT b.id, b.conversation_id, b.agent_principal_id, b.driver_type,
            b.workspace_path, b.git_worktree_path, b.external_session_id,
            b.external_thread_id, b.last_event_cursor, b.last_acked_event_cursor,
            b.last_seen_server_seq, b.state, b.last_error, b.config_json, b.updated_at,
            p.workspace_id AS agent_workspace_id, p.name AS agent_name,
            c.type AS conversation_type,
            COALESCE(NULLIF(btrim(c.name), ''), (
                SELECT string_agg(COALESCE(mp.name, cm.principal_id), ' / '
                                  ORDER BY COALESCE(mp.name, cm.principal_id))
                  FROM conversation_member cm
                  LEFT JOIN principal mp ON mp.id = cm.principal_id
                 WHERE cm.conv_id = c.id AND cm.removed_at IS NULL
            ), '') AS conversation_name
       FROM agent_runtime_bindings b
       JOIN principal p ON p.id = b.agent_principal_id
       JOIN conversation c ON c.id = b.conversation_id
      WHERE p.workspace_id = ANY($1) AND c.workspace_id = ANY($1)
        AND ($2::text IS NULL OR b.id = $2)
      ORDER BY b.created_at ASC, b.id ASC";

/// Every binding the caller may see, or the one named by `binding_id`, in a
/// single query. The same rows back `GET /v1/runtime/bindings`, the by-id
/// read and the dashboard bootstrap, so the three cannot disagree.
pub(crate) async fn list_binding_views(
    state: &ApiState,
    allowed_workspaces: &HashSet<String>,
    binding_id: Option<&str>,
) -> Result<Vec<RuntimeBindingView>, ApiError> {
    if allowed_workspaces.is_empty() {
        return Ok(Vec::new());
    }
    let workspaces: Vec<&str> = allowed_workspaces.iter().map(String::as_str).collect();
    let client = state.event_store.connect().await.map_err(ApiError::from)?;
    let rows = client
        .query(BINDING_VIEW_SQL, &[&workspaces, &binding_id])
        .await
        .map_err(|error| {
            ApiError(AppError::Internal(format!(
                "list runtime binding views: {error}"
            )))
        })?;
    rows.iter().map(binding_view_from_row).collect()
}

fn binding_view_from_row(row: &tokio_postgres::Row) -> Result<RuntimeBindingView, ApiError> {
    let driver_type: DriverType = serde_json::from_value(Value::String(row.get("driver_type")))
        .map_err(|error| {
            ApiError(AppError::Internal(format!(
                "runtime binding driver_type: {error}"
            )))
        })?;
    let state: BindingState =
        serde_json::from_value(Value::String(row.get("state"))).map_err(|error| {
            ApiError(AppError::Internal(format!(
                "runtime binding state: {error}"
            )))
        })?;
    let config_json: Value = row.get("config_json");
    let conversation_type = match row.get::<_, String>("conversation_type").as_str() {
        "group" => "group",
        _ => "direct",
    };
    let string_config = |key: &str| {
        config_json
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    Ok(RuntimeBindingView {
        id: row.get("id"),
        workspace_id: row.get("agent_workspace_id"),
        conversation_id: row.get("conversation_id"),
        conversation_name: row.get("conversation_name"),
        conversation_type,
        agent_principal_id: row.get("agent_principal_id"),
        agent_name: row.get("agent_name"),
        interaction_mode: Some(effective_interaction_mode(&driver_type, &config_json)),
        driver_type,
        runtime_host_id: string_config("runtime_host_id"),
        harness_account_id: string_config("harness_account_id"),
        harness_account_name: string_config("harness_account_name"),
        workspace_path: row.get("workspace_path"),
        git_worktree_path: row.get("git_worktree_path"),
        external_session_id: row.get("external_session_id"),
        external_thread_id: row.get("external_thread_id"),
        last_event_cursor: row.get("last_event_cursor"),
        last_acked_event_cursor: row.get("last_acked_event_cursor"),
        last_seen_server_seq: row.get("last_seen_server_seq"),
        state,
        last_error: row
            .get::<_, Option<String>>("last_error")
            .as_deref()
            .map(redact_sensitive_text),
        updated_at: row.get("updated_at"),
    })
}

pub(crate) async fn conversation_label(db: &DbService, conversation: &Conversation) -> String {
    if let Some(name) = conversation
        .name
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        return name.clone();
    }

    let mut members = Vec::new();
    for pid in conversation.members.keys() {
        let name = match db.get_principal(pid).await {
            Ok(p) => p.name,
            Err(_) => pid.clone(),
        };
        members.push(name);
    }
    members.sort();
    members.join(" / ")
}

pub(crate) fn conversation_type_label(kind: &ConversationType) -> &'static str {
    match kind {
        ConversationType::Direct => "direct",
        ConversationType::Group => "group",
    }
}

pub(crate) async fn validate_runtime_target(
    db: &DbService,
    allowed_workspaces: &HashSet<String>,
    conversation_id: &str,
    agent_id: &str,
) -> Result<(), ApiError> {
    let conversation = db.get_conversation(conversation_id).await?;
    if !allowed_workspaces.contains(&conversation.workspace_id) {
        return Err(ApiError(AppError::Forbidden(
            "cross-workspace access denied".into(),
        )));
    }

    let agent = db.get_principal(agent_id).await?;
    if !allowed_workspaces.contains(&agent.workspace_id) {
        return Err(ApiError(AppError::Forbidden(
            "cross-workspace access denied".into(),
        )));
    }
    if !matches!(agent.principal_type, PrincipalType::Agent) {
        return Err(ApiError(AppError::Validation(
            "runtime bindings can only target agents".into(),
        )));
    }
    if !conversation.members.contains_key(agent_id) {
        return Err(ApiError(AppError::Validation(
            "agent must be a conversation member before binding".into(),
        )));
    }

    Ok(())
}

pub(crate) async fn validate_runtime_conversation_access(
    db: &DbService,
    allowed_workspaces: &HashSet<String>,
    conversation_id: &str,
) -> Result<(), ApiError> {
    let conversation = db.get_conversation(conversation_id).await?;
    if !allowed_workspaces.contains(&conversation.workspace_id) {
        return Err(ApiError(AppError::Forbidden(
            "cross-workspace access denied".into(),
        )));
    }

    Ok(())
}

// ── Handler functions ─────────────────────────────────────────────────

pub(crate) async fn list_runtime_bindings(
    headers: HeaderMap,
    State(state): State<ApiState>,
) -> Result<Json<Vec<RuntimeBindingView>>, ApiError> {
    let operator = require_human_operator(&headers, &state).await?;
    let allowed_ws = accessible_workspace_ids(&state.db, &operator.id).await?;
    Ok(Json(list_binding_views(&state, &allowed_ws, None).await?))
}

pub(crate) async fn get_runtime_binding(
    headers: HeaderMap,
    Path(binding_id): Path<String>,
    State(state): State<ApiState>,
) -> Result<Json<RuntimeBindingView>, ApiError> {
    let operator = require_human_operator(&headers, &state).await?;
    let allowed_ws = accessible_workspace_ids(&state.db, &operator.id).await?;
    // The binding must exist before workspace scoping decides the status.
    state.runtime.get_binding(&binding_id).await?;
    let view = list_binding_views(&state, &allowed_ws, Some(&binding_id))
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| ApiError(AppError::Forbidden("cross-workspace access denied".into())))?;
    Ok(Json(view))
}

pub(crate) async fn create_runtime_binding(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Json(payload): Json<CreateRuntimeBindingRequest>,
) -> Result<(StatusCode, Json<RuntimeBindingView>), ApiError> {
    validate_runtime_binding_config(payload.config_json.as_ref()).map_err(ApiError)?;
    let actor = authenticated_principal(&headers, &state).await?;
    if !matches!(actor.principal_type, PrincipalType::Human) {
        return Err(ApiError(AppError::Forbidden(
            "only humans can create runtime bindings".into(),
        )));
    }
    let allowed_ws = accessible_workspace_ids(&state.db, &actor.id).await?;
    validate_runtime_target(
        &state.db,
        &allowed_ws,
        &payload.conversation_id,
        &payload.agent_principal_id,
    )
    .await?;
    let agent = state.db.get_principal(&payload.agent_principal_id).await?;
    let conversation = state.db.get_conversation(&payload.conversation_id).await?;
    if !conversation.members.contains_key(&actor.id) {
        return Err(ApiError(AppError::Forbidden(
            "must be a conversation member to create a runtime binding".into(),
        )));
    }
    // Retries must return the canonical binding even if the account has since
    // been disabled or its catalog has changed. The new request is not applied
    // to an existing binding.
    let existing = state
        .runtime
        .list_bindings_by_agent(&payload.agent_principal_id)
        .await?;
    if let Some(active) = existing
        .into_iter()
        .find(|b| !matches!(b.state, BindingState::Disabled))
    {
        let view = binding_view_for_workspace(&state.db, active, &allowed_ws).await?;
        return Ok((StatusCode::OK, Json(view)));
    }
    let normalized_runtime_host_id =
        validate_runtime_host(&state, &agent.workspace_id, payload.config_json.as_ref()).await?;
    let normalized_harness_account = validate_harness_account(
        &state,
        &agent.workspace_id,
        payload.driver_type.clone(),
        normalized_runtime_host_id.as_deref(),
        payload.config_json.as_ref(),
    )
    .await?;
    let mut config_json = payload.config_json;
    if let Some(runtime_host_id) = normalized_runtime_host_id {
        config_json
            .as_mut()
            .and_then(serde_json::Value::as_object_mut)
            .expect("validated runtime config must be an object")
            .insert(
                "runtime_host_id".into(),
                serde_json::Value::String(runtime_host_id),
            );
    }
    if let Some((id, name, profile_kind)) = normalized_harness_account {
        let config = config_json
            .as_mut()
            .and_then(serde_json::Value::as_object_mut)
            .expect("validated runtime config must be an object");
        config.insert("harness_account_id".into(), serde_json::Value::String(id));
        config.insert(
            "harness_account_name".into(),
            serde_json::Value::String(name),
        );
        config.insert(
            "harness_account_profile_kind".into(),
            serde_json::Value::String(profile_kind),
        );
    }
    // Upsert FK target rows so that audit_log / outbox_event INSERTs
    // referencing these IDs will not violate foreign key constraints.
    {
        let client = state.runtime.connect().await.map_err(ApiError)?;
        let now = chrono::Utc::now();

        let agent_type_str = match agent.principal_type {
            PrincipalType::Human => "human",
            PrincipalType::Agent => "agent",
        };
        client
            .execute(
                "INSERT INTO principal (id, workspace_id, type, name, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $5)
                 ON CONFLICT (id) DO UPDATE
                 SET workspace_id = EXCLUDED.workspace_id,
                     name = EXCLUDED.name,
                     updated_at = EXCLUDED.updated_at",
                &[
                    &agent.id,
                    &agent.workspace_id,
                    &agent_type_str,
                    &agent.name,
                    &now,
                ],
            )
            .await
            .map_err(|e| ApiError(AppError::Internal(format!("upsert principal: {e}"))))?;

        let conv_type_str = match conversation.conversation_type {
            ConversationType::Direct => "direct",
            ConversationType::Group => "group",
        };
        client
            .execute(
                "INSERT INTO conversation (id, workspace_id, type, name, creator_id, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $6)
                 ON CONFLICT (id) DO UPDATE
                 SET workspace_id = EXCLUDED.workspace_id,
                     name = EXCLUDED.name,
                     updated_at = EXCLUDED.updated_at",
                &[
                    &conversation.id,
                    &conversation.workspace_id,
                    &conv_type_str,
                    &conversation.name,
                    &conversation.creator_id,
                    &now,
                ],
            )
            .await
            .map_err(|e| ApiError(AppError::Internal(format!("upsert conversation: {e}"))))?;
    }

    let binding = state
        .runtime
        .create_binding(CreateBindingInput {
            conversation_id: payload.conversation_id,
            agent_principal_id: payload.agent_principal_id,
            driver_type: payload.driver_type,
            workspace_path: payload.workspace_path,
            git_worktree_path: payload.git_worktree_path,
            config_json: runtime_binding_config_for_agent(&agent, config_json),
            audit_actor: Some(runtime_audit_actor(&actor)),
        })
        .await?;
    let view = binding_view_for_workspace(&state.db, binding, &allowed_ws).await?;
    Ok((StatusCode::CREATED, Json(view)))
}

#[cfg(test)]
mod tests {
    use super::validate_runtime_binding_config;
    use serde_json::json;

    #[test]
    fn runtime_binding_model_validation_rejects_cli_option_injection() {
        assert!(validate_runtime_binding_config(Some(&json!({ "model": "--help" }))).is_err());
        assert!(validate_runtime_binding_config(Some(&json!({ "model": "bad\nmodel" }))).is_err());
        assert!(
            validate_runtime_binding_config(Some(&json!({ "model": "x".repeat(257) }))).is_err()
        );
    }

    #[test]
    fn runtime_binding_model_validation_accepts_provider_ids() {
        assert!(
            validate_runtime_binding_config(Some(&json!({
                "model": "openrouter/anthropic/claude-sonnet-5:fast"
            })))
            .is_ok()
        );
        assert!(validate_runtime_binding_config(Some(&json!({}))).is_ok());
    }
}

pub(crate) async fn rebind_runtime_binding(
    headers: HeaderMap,
    Path(binding_id): Path<String>,
    State(state): State<ApiState>,
    Json(payload): Json<RebindRuntimeBindingRequest>,
) -> Result<Json<RuntimeBindingView>, ApiError> {
    let operator = require_human_operator(&headers, &state).await?;
    let allowed_ws = accessible_workspace_ids(&state.db, &operator.id).await?;
    let current = state.runtime.get_binding(&binding_id).await?;
    let _ = binding_view_for_workspace(&state.db, current, &allowed_ws).await?;
    let binding = state
        .runtime
        .rebind_workspace(
            &binding_id,
            &payload.workspace_path,
            &runtime_audit_actor(&operator),
        )
        .await?;
    let view = binding_view_for_workspace(&state.db, binding, &allowed_ws).await?;
    Ok(Json(view))
}

// ── Runtime policy ────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub(crate) struct RuntimePolicyView {
    conversation_id: String,
    auto_mode: String,
    max_auto_turns: i32,
    max_workflow_turns: i32,
    require_human_after_n_turns: i32,
    allow_agent_to_agent: bool,
    allow_file_write: bool,
    default_reviewer_agent_id: Option<String>,
    default_coordinator_agent_id: Option<String>,
    untagged_human_mode: String,
}

impl RuntimePolicyView {
    fn from_policy(policy: ConversationRuntimePolicy) -> Self {
        Self {
            conversation_id: policy.conversation_id,
            auto_mode: policy.auto_mode.as_str().into(),
            max_auto_turns: policy.max_auto_turns,
            max_workflow_turns: policy.max_workflow_turns,
            require_human_after_n_turns: policy.require_human_after_n_turns,
            allow_agent_to_agent: policy.allow_agent_to_agent,
            allow_file_write: policy.allow_file_write,
            default_reviewer_agent_id: policy.default_reviewer_agent_id,
            default_coordinator_agent_id: policy.default_coordinator_agent_id,
            untagged_human_mode: policy.untagged_human_mode.as_str().into(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpsertRuntimePolicyRequest {
    auto_mode: Option<String>,
    max_auto_turns: Option<i32>,
    max_workflow_turns: Option<i32>,
    require_human_after_n_turns: Option<i32>,
    allow_agent_to_agent: Option<bool>,
    allow_file_write: Option<bool>,
    default_reviewer_agent_id: Option<String>,
    default_coordinator_agent_id: Option<String>,
    untagged_human_mode: Option<String>,
}

pub(crate) async fn get_runtime_policy(
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    State(state): State<ApiState>,
) -> Result<Json<RuntimePolicyView>, ApiError> {
    let operator = require_human_operator(&headers, &state).await?;
    let allowed_ws = accessible_workspace_ids(&state.db, &operator.id).await?;
    validate_runtime_conversation_access(&state.db, &allowed_ws, &conversation_id).await?;
    let policy = state.runtime.get_policy(&conversation_id).await?;
    Ok(Json(RuntimePolicyView::from_policy(policy)))
}

pub(crate) async fn upsert_runtime_policy(
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    State(state): State<ApiState>,
    Json(payload): Json<UpsertRuntimePolicyRequest>,
) -> Result<Json<RuntimePolicyView>, ApiError> {
    let operator = require_human_operator(&headers, &state).await?;
    let allowed_ws = accessible_workspace_ids(&state.db, &operator.id).await?;
    validate_runtime_conversation_access(&state.db, &allowed_ws, &conversation_id).await?;
    if let Some(coordinator_agent_id) = payload.default_coordinator_agent_id.as_deref() {
        validate_runtime_target(
            &state.db,
            &allowed_ws,
            &conversation_id,
            coordinator_agent_id,
        )
        .await?;
    }
    let current = state.runtime.get_policy(&conversation_id).await?;
    let auto_mode = match payload.auto_mode.as_deref() {
        Some("disabled") => AutoMode::Disabled,
        Some("mentioned_only") => AutoMode::MentionedOnly,
        Some("metadata_only") => AutoMode::MetadataOnly,
        Some(_) => {
            return Err(ApiError(AppError::Validation(
                "auto_mode must be disabled, mentioned_only, or metadata_only".into(),
            )));
        }
        None => current.auto_mode,
    };
    let untagged_human_mode = match payload.untagged_human_mode.as_deref() {
        Some("mentioned_only") => UntaggedHumanMode::MentionedOnly,
        Some("coordinator_only") => UntaggedHumanMode::CoordinatorOnly,
        Some("all_agents") => UntaggedHumanMode::AllAgents,
        Some(_) => {
            return Err(ApiError(AppError::Validation(
                "untagged_human_mode must be mentioned_only, coordinator_only, or all_agents"
                    .into(),
            )));
        }
        None => current.untagged_human_mode,
    };
    let policy = state
        .runtime
        .upsert_policy(UpsertPolicyInput {
            conversation_id,
            auto_mode,
            max_auto_turns: payload.max_auto_turns.unwrap_or(current.max_auto_turns),
            max_workflow_turns: payload
                .max_workflow_turns
                .unwrap_or(current.max_workflow_turns),
            require_human_after_n_turns: payload
                .require_human_after_n_turns
                .unwrap_or(current.require_human_after_n_turns),
            allow_agent_to_agent: payload
                .allow_agent_to_agent
                .unwrap_or(current.allow_agent_to_agent),
            allow_file_write: payload.allow_file_write.unwrap_or(current.allow_file_write),
            default_reviewer_agent_id: payload
                .default_reviewer_agent_id
                .or(current.default_reviewer_agent_id),
            default_coordinator_agent_id: payload
                .default_coordinator_agent_id
                .or(current.default_coordinator_agent_id),
            untagged_human_mode,
            audit_actor: Some(runtime_audit_actor(&operator)),
        })
        .await?;
    Ok(Json(RuntimePolicyView::from_policy(policy)))
}

// ── Session management ────────────────────────────────────────────────

/// Clear session IDs for all agents in a company.
pub(crate) async fn reset_company_sessions(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(company_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let operator = require_human_operator(&headers, &state).await?;
    let allowed_ws = accessible_workspace_ids(&state.db, &operator.id).await?;
    if !allowed_ws.contains(&company_id) {
        return Err(ApiError(AppError::Forbidden(
            "cross-workspace access denied".into(),
        )));
    }

    let client = state.event_store.connect().await.map_err(ApiError::from)?;
    let updated = client
        .execute(
            "UPDATE agent_runtime_bindings
                 SET external_session_id = NULL,
                 config_json = jsonb_set(
                   config_json
                     - 'terminal_session'
                     - 'terminal_capture'
                     - 'external_session_provenance'
                     - 'external_session_driver_type'
                     - 'external_session_binding_id'
                     - 'external_session_mode'
                     - 'external_session_captured_at',
                   '{terminal_generation}',
                   to_jsonb(COALESCE((config_json->>'terminal_generation')::bigint, 0) + 1),
                   true
                 ),
                 updated_at = clock_timestamp()
             WHERE agent_principal_id IN (
                SELECT id FROM principal WHERE workspace_id = $1 AND type = 'agent'
             )",
            &[&company_id],
        )
        .await
        .map_err(|e| ApiError::from(AppError::Internal(format!("reset company sessions: {e}"))))?;

    tracing::info!(company_id = %company_id, bindings_cleared = updated, "company sessions reset");
    Ok(Json(
        serde_json::json!({ "company_id": company_id, "bindings_cleared": updated }),
    ))
}
