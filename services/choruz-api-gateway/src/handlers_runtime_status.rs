use std::collections::{HashMap, HashSet};

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use choruz_common::AppError;
use choruz_domain::{Principal, PrincipalType};
use choruz_session::{AgentRuntimeStatus, RuntimeStatusCommand};
use serde::Serialize;

use crate::handlers_runtime::accessible_workspace_ids;
use crate::{ApiError, ApiState, redact_sensitive_text, require_human_operator};

#[derive(Debug, Serialize)]
pub(crate) struct RuntimeStatusCommandView {
    command_id: String,
    message_id: String,
    turn_id: String,
    status: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    lease_age_seconds: Option<i64>,
    attempt_count: i32,
    last_error: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ConversationRuntimeStatusView {
    conversation_id: String,
    agent_principal_id: String,
    agent_name: String,
    status: String,
    queued_count: i64,
    active_command: Option<RuntimeStatusCommandView>,
    last_error: Option<String>,
}

pub(crate) async fn get_conversation_runtime_status(
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    State(state): State<ApiState>,
) -> Result<Json<Vec<ConversationRuntimeStatusView>>, ApiError> {
    let operator = require_human_operator(&headers, &state).await?;
    let allowed_ws = accessible_workspace_ids(&state.db, &operator.id).await?;
    let conversation = state.db.get_conversation(&conversation_id).await?;
    if !allowed_ws.contains(&conversation.workspace_id) {
        return Err(ApiError(AppError::Forbidden(
            "cross-workspace access denied".into(),
        )));
    }

    let member_ids = conversation.members.keys().cloned().collect::<Vec<_>>();
    let agents = active_agent_members(&state, &member_ids).await?;
    let agent_ids = agents
        .iter()
        .map(|agent| agent.id.clone())
        .collect::<Vec<_>>();
    let statuses = state
        .session
        .list_runtime_status_for_agents(&conversation_id, &agent_ids, chrono::Utc::now())
        .await
        .map_err(map_session_error)?;
    let mut statuses_by_agent = statuses
        .into_iter()
        .map(|status| (status.agent_id.clone(), status))
        .collect::<HashMap<_, _>>();

    let mut views = Vec::with_capacity(agents.len());
    for agent in agents {
        let status = statuses_by_agent.remove(&agent.id);
        views.push(runtime_status_view(&conversation_id, agent, status));
    }
    Ok(Json(views))
}

async fn active_agent_members(
    state: &ApiState,
    member_ids: &[String],
) -> Result<Vec<Principal>, ApiError> {
    let mut agents = Vec::new();
    let mut seen = HashSet::new();
    for member_id in member_ids {
        if !seen.insert(member_id.clone()) {
            continue;
        }
        match state.db.get_principal(member_id).await {
            Ok(principal) if matches!(principal.principal_type, PrincipalType::Agent) => {
                agents.push(principal);
            }
            Ok(_) | Err(AppError::NotFound(_) | AppError::Forbidden(_)) => {}
            Err(error) => return Err(ApiError(error)),
        }
    }
    agents.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(agents)
}

fn runtime_status_view(
    conversation_id: &str,
    agent: Principal,
    status: Option<AgentRuntimeStatus>,
) -> ConversationRuntimeStatusView {
    match status {
        Some(status) => {
            let last_error = status.last_error.as_deref().or_else(|| {
                status
                    .active_command
                    .as_ref()
                    .and_then(|command| command.last_error.as_deref())
            });
            let last_error = last_error.map(redact_sensitive_text);
            let active_command = status.active_command.map(command_view);
            ConversationRuntimeStatusView {
                conversation_id: status.conversation_id,
                agent_principal_id: agent.id,
                agent_name: agent.name,
                status: status.status,
                queued_count: status.queued_count,
                active_command,
                last_error,
            }
        }
        None => ConversationRuntimeStatusView {
            conversation_id: conversation_id.to_string(),
            agent_principal_id: agent.id,
            agent_name: agent.name,
            status: "idle".into(),
            queued_count: 0,
            active_command: None,
            last_error: None,
        },
    }
}

fn command_view(command: RuntimeStatusCommand) -> RuntimeStatusCommandView {
    RuntimeStatusCommandView {
        command_id: command.command_id,
        message_id: command.message_id,
        turn_id: command.turn_id,
        status: command.status,
        created_at: command.created_at,
        updated_at: command.updated_at,
        lease_age_seconds: Some(command.lease_age_seconds),
        attempt_count: command.attempt_count,
        last_error: command.last_error.as_deref().map(redact_sensitive_text),
    }
}

fn map_session_error(error: choruz_session::SessionError) -> ApiError {
    ApiError(AppError::Internal(format!(
        "runtime status query failed: {error}"
    )))
}
