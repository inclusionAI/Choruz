//! Structured direct conversations use the same authorization and device owner as terminals.
use crate::{
    ApiError, ApiState, authenticated_principal,
    handlers_terminals::{
        authorize_terminal_binding, capture_codex_terminal_before_cleanup, lock_terminal_launch,
        prepare_codex_spawn_if_needed, terminal_spec,
    },
    host_runtime::RuntimeHost,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use choruz_agent_runtime::{BindingState, DriverType, RuntimeBinding};
use choruz_common::AppError;
use choruz_host_runtime::{
    session::{SessionCommand, SessionRequest},
    session_protocol::SessionSnapshot,
};

async fn context(
    headers: &HeaderMap,
    state: &ApiState,
    id: &str,
) -> Result<(RuntimeHost, RuntimeBinding), ApiError> {
    let principal = authenticated_principal(headers, state).await?;
    let binding = authorize_terminal_binding(state, &principal, id).await?;
    if !matches!(
        binding.driver_type,
        DriverType::ClaudeTerminal | DriverType::CodexTerminal
    ) {
        return Err(AppError::Validation(
            "Structured conversations support Claude Code and Codex".into(),
        )
        .into());
    }
    Ok((RuntimeHost::for_binding(state, &binding)?, binding))
}

fn binding_owner(binding: &RuntimeBinding) -> String {
    choruz_host_runtime::session::owner_key(&terminal_spec(binding, 120, 40, None, None))
}

pub(crate) async fn open(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<SessionSnapshot>, ApiError> {
    let mut client = state.runtime.connect().await?;
    let transaction = client
        .transaction()
        .await
        .map_err(|e| AppError::Internal(format!("begin session launch: {e}")))?;
    lock_terminal_launch(&transaction, &id).await?;
    let (host, binding) = context(&headers, &state, &id).await?;
    if binding.state == BindingState::Running || binding.in_flight_turn_id.is_some() {
        return Err(AppError::Conflict(
            "The Agent has an active turn. Wait for it to finish.".into(),
        )
        .into());
    }
    match host
        .session(SessionRequest::Read {
            binding_id: id.clone(),
            owner: binding_owner(&binding),
            after: 0,
        })
        .await
    {
        Ok(snapshot) if !matches!(snapshot.status.as_str(), "closed" | "failed") => {
            return Ok(Json(snapshot));
        }
        Ok(_) | Err(AppError::NotFound(_)) => {}
        Err(error) => return Err(error.into()),
    }
    let resume = binding
        .valid_terminal_session_anchor_for_context(
            None,
            None,
            Some(binding.terminal_generation()),
            None,
        )
        .map(|anchor| anchor.session_id)
        .or_else(|| binding.external_session_id.clone());
    let (mut binding, codex_resume, codex_home) =
        prepare_codex_spawn_if_needed(&host, &state.runtime, binding, &id).await?;
    let resume = if binding.driver_type == DriverType::CodexTerminal {
        codex_resume
    } else {
        resume
    };
    let mut spec = terminal_spec(&binding, 120, 40, resume, codex_home);
    if binding.driver_type == DriverType::ClaudeTerminal {
        let prepared = host
            .session(SessionRequest::Prepare {
                spec: Box::new(spec.clone()),
            })
            .await?;
        capture(&state, &host, binding.clone(), &prepared).await?;
        // Re-read the reserved anchor rather than returning a process whose
        // durable identity might have lost a concurrent binding update.
        let (_, reserved) = context(&headers, &state, &id).await?;
        if binding_owner(&reserved) != binding_owner(&binding) {
            return Err(AppError::Conflict(
                "Agent configuration changed during session launch".into(),
            )
            .into());
        }
        spec.resume_session_id = prepared.session_id;
        spec.harness_account = reserved.config_json.clone();
        binding = reserved;
    }
    let snapshot = host
        .session(SessionRequest::Ensure {
            spec: Box::new(spec),
        })
        .await?;
    transaction
        .commit()
        .await
        .map_err(|e| AppError::Internal(format!("commit session launch: {e}")))?;
    tracing::info!(binding_id = %id, driver = binding.driver_type.as_str(), "structured session opened");
    capture(&state, &host, binding, &snapshot).await?;
    Ok(Json(snapshot))
}

async fn capture(
    state: &ApiState,
    host: &RuntimeHost,
    binding: RuntimeBinding,
    snapshot: &SessionSnapshot,
) -> Result<(), ApiError> {
    if binding.driver_type == DriverType::CodexTerminal {
        let id = binding.id.clone();
        capture_codex_terminal_before_cleanup(host, &state.runtime, &id, binding).await?;
    } else if let Some(id) = &snapshot.session_id
        && let Some(home) = &snapshot.native_home_path
        && binding.terminal_session_anchor().is_none_or(|anchor| {
            anchor.session_id != *id
                || anchor.native_session_path
                    != snapshot.native_session_path.clone().unwrap_or_default()
        })
    {
        let field = |key: &str| {
            binding.config_json[key]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        };
        state
            .runtime
            .write_direct_session_anchor(
                &binding,
                choruz_agent_runtime::TerminalSessionAnchorInput {
                    session_id: id.clone(),
                    source: "native_cli".into(),
                    provenance: if snapshot.native_session_path.is_some() {
                        "terminal_process_captured"
                    } else {
                        "direct_session_reserved"
                    }
                    .into(),
                    binding_id: binding.id.clone(),
                    conversation_id: binding.conversation_id.clone(),
                    agent_principal_id: binding.agent_principal_id.clone(),
                    company_id: field("conversation_workspace_id"),
                    driver_type: binding.driver_type.as_str().into(),
                    workspace_id: field("agent_workspace_id"),
                    workspace_path: binding.workspace_path.clone(),
                    native_home_path: home.clone(),
                    native_session_path: snapshot.native_session_path.clone().unwrap_or_default(),
                    binding_generation: binding.terminal_generation(),
                    binding_updated_at: binding.updated_at,
                },
            )
            .await?;
    }
    Ok(())
}

#[derive(serde::Deserialize, Default)]
pub(crate) struct SessionCursor {
    #[serde(default)]
    after: u64,
}

pub(crate) async fn read(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Query(cursor): Query<SessionCursor>,
) -> Result<Json<SessionSnapshot>, ApiError> {
    let (host, binding) = context(&headers, &state, &id).await?;
    let snapshot = host
        .session(SessionRequest::Read {
            binding_id: id,
            owner: binding_owner(&binding),
            after: cursor.after,
        })
        .await?;
    capture(&state, &host, binding, &snapshot).await?;
    Ok(Json(snapshot))
}

#[derive(serde::Deserialize)]
pub(crate) struct SessionAction {
    instance: String,
    #[serde(flatten)]
    command: SessionCommand,
}

pub(crate) async fn command(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(action): Json<SessionAction>,
) -> Result<Json<SessionSnapshot>, ApiError> {
    let command = action.command;
    let instance = action.instance;
    let (host, binding) = context(&headers, &state, &id).await?;
    let principal = authenticated_principal(&headers, &state).await?;
    let action = match &command {
        SessionCommand::Send { .. } => "session.send",
        SessionCommand::Respond { .. } => "session.respond",
        SessionCommand::Interrupt => "session.interrupt",
        SessionCommand::Close => "session.close",
    };
    let result = host
        .session(SessionRequest::Command {
            binding_id: id.clone(),
            owner: binding_owner(&binding),
            instance,
            command,
        })
        .await;
    crate::handlers_terminals::record_terminal_activity(
        &state,
        &principal,
        &binding,
        action,
        serde_json::json!({"outcome":if result.is_ok() {"succeeded"} else {"failed"},
        "trace_id":headers.get("x-trace-id").and_then(|value|value.to_str().ok())}),
    )
    .await;
    let snapshot = result?;
    capture(&state, &host, binding, &snapshot).await?;
    tracing::info!(binding_id = %id, action, "structured session command accepted");
    Ok(Json(snapshot))
}
