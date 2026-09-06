use axum::{
    Json,
    extract::{
        Path, Query, State,
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
    },
    http::HeaderMap,
    response::IntoResponse,
};
use choruz_agent_runtime::{
    BindingState, CodexTerminalCaptureInput, CodexTerminalCaptureMetadata, DriverType,
    RuntimeBinding, RuntimeStore, TerminalSessionAnchorInput,
};
use choruz_common::AppError;
use choruz_domain::{ConversationType, Principal, PrincipalType};
use choruz_host_runtime::{
    CodexHomeReady, CodexSessionFileMeta, HostRequest, TerminalSpec, is_terminal_driver,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use crate::{
    ApiError, ApiState, authenticated_principal, bearer_token_value,
    handlers_runtime::accessible_workspace_ids, host_runtime::RuntimeHost,
};

// ── Codex session attribution ─────────────────────────────────────────

/// Copy a user-selected native Codex session into the binding-owned store on
/// the binding's device and return the terminal anchor that proves the copied
/// history belongs to this binding.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn import_codex_terminal_session(
    host: &RuntimeHost,
    binding_id: &str,
    conversation_id: &str,
    agent_principal_id: &str,
    company_id: &str,
    workspace_id: &str,
    workspace_path: &str,
    native_session_id: &str,
    harness_account: serde_json::Value,
) -> Result<serde_json::Value, AppError> {
    let imported: choruz_host_runtime::ImportedCodexSession = host
        .call(HostRequest::CodexImportSession {
            binding_id: binding_id.into(),
            workspace_path: workspace_path.into(),
            native_session_id: native_session_id.into(),
            harness_account,
        })
        .await?;
    Ok(json!({
        "driver_type": DriverType::CodexTerminal.as_str(),
        "session_id": native_session_id,
        "source": "native_cli",
        "provenance": "workspace_scan_imported",
        "binding_id": binding_id,
        "conversation_id": conversation_id,
        "agent_principal_id": agent_principal_id,
        "company_id": company_id,
        "workspace_id": workspace_id,
        "workspace_path": workspace_path,
        "native_home_path": imported.home_path,
        "native_session_path": imported.native_session_path,
        "binding_generation": 0,
        "captured_at": chrono::Utc::now().to_rfc3339(),
        "last_verified_at": chrono::Utc::now().to_rfc3339(),
    }))
}

fn codex_capture_metadata_matches_binding(
    binding: &RuntimeBinding,
    capture: &CodexTerminalCaptureMetadata,
) -> bool {
    let workspace_id_matches = binding
        .config_json
        .get("agent_workspace_id")
        .and_then(|v| v.as_str())
        .is_some_and(|workspace_id| workspace_id == capture.workspace_id);
    let company_id_matches = binding
        .config_json
        .get("conversation_workspace_id")
        .and_then(|v| v.as_str())
        .is_some_and(|company_id| company_id == capture.company_id);

    capture.binding_id == binding.id
        && capture.conversation_id == binding.conversation_id
        && capture.agent_principal_id == binding.agent_principal_id
        && capture.driver_type == binding.driver_type.as_str()
        && capture.workspace_path == binding.workspace_path
        && capture.binding_generation == binding.terminal_generation()
        && workspace_id_matches
        && company_id_matches
}

/// The one session file this binding's Codex terminal wrote since spawn, or
/// `None` when the capture no longer describes the binding.
async fn unique_codex_session_candidate(
    host: &RuntimeHost,
    binding: &RuntimeBinding,
    capture: &CodexTerminalCaptureMetadata,
) -> Result<Option<CodexSessionFileMeta>, AppError> {
    if !codex_capture_metadata_matches_binding(binding, capture) {
        return Ok(None);
    }
    host.call(HostRequest::CodexNewSession {
        home_path: capture.native_home_path.clone(),
        sessions_path: capture.sessions_path.clone(),
        baseline_session_files: capture.baseline_session_files.clone(),
        workspace_path: capture.workspace_path.clone(),
    })
    .await
}

async fn reconcile_codex_terminal_session_from_capture(
    host: &RuntimeHost,
    runtime: &RuntimeStore,
    binding: &RuntimeBinding,
) -> Result<Option<RuntimeBinding>, AppError> {
    let Some(capture) = binding.codex_terminal_capture_metadata() else {
        return Ok(None);
    };
    let Some(candidate) = unique_codex_session_candidate(host, binding, &capture).await? else {
        return Ok(None);
    };

    runtime
        .write_terminal_session_anchor(
            &binding.id,
            TerminalSessionAnchorInput {
                session_id: candidate.session_id,
                source: "native_cli".into(),
                provenance: "terminal_process_captured".into(),
                binding_id: binding.id.clone(),
                conversation_id: binding.conversation_id.clone(),
                agent_principal_id: binding.agent_principal_id.clone(),
                company_id: capture.company_id,
                driver_type: binding.driver_type.as_str().into(),
                workspace_id: capture.workspace_id,
                workspace_path: binding.workspace_path.clone(),
                native_home_path: capture.native_home_path,
                native_session_path: candidate.path.to_string_lossy().to_string(),
                binding_generation: capture.binding_generation,
                binding_updated_at: binding.updated_at,
            },
        )
        .await
        .map(Some)
}

fn terminal_capture_error_is_permanent(error: &AppError) -> bool {
    matches!(error, AppError::NotFound(_))
}

async fn try_capture_codex_terminal_session(
    host: &RuntimeHost,
    runtime: &RuntimeStore,
    binding: &mut RuntimeBinding,
    captured_session_id: &mut Option<String>,
    capture_disabled: &mut bool,
) {
    if captured_session_id.is_some() || *capture_disabled {
        return;
    }
    match reconcile_codex_terminal_session_from_capture(host, runtime, binding).await {
        Ok(Some(updated)) => {
            *captured_session_id = updated.valid_terminal_session_id();
            *binding = updated;
        }
        Ok(None) => {}
        Err(error) => {
            *capture_disabled = terminal_capture_error_is_permanent(&error);
            tracing::warn!(
                binding_id = %binding.id,
                disabled = *capture_disabled,
                error = %error,
                "Codex terminal session capture failed"
            );
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct TerminalQuery {
    token: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
}

/// The session id a non-Codex terminal resumes: the binding's own, else the
/// newest one the device finds for the workspace, which is then recorded.
async fn terminal_resume_session_id(
    host: &RuntimeHost,
    runtime: &RuntimeStore,
    binding: &RuntimeBinding,
    binding_id: &str,
    context: &str,
) -> Option<String> {
    if binding.driver_type == DriverType::CodexTerminal {
        if binding
            .external_session_id
            .as_deref()
            .is_some_and(|s| !s.is_empty())
        {
            tracing::warn!(
                binding_id = %binding.id,
                context,
                "not resuming Codex terminal session from generic resume path"
            );
        }
        return None;
    }

    if let Some(session_id) = binding
        .external_session_id
        .clone()
        .filter(|s| !s.is_empty())
    {
        return Some(session_id);
    }
    match sync_session_id_from_device(host, runtime, binding_id).await {
        Ok(Some(sid)) => {
            tracing::info!(binding_id = %binding_id, session_id = %sid, context, "PTY: backfilled session_id from disk");
            Some(sid)
        }
        Ok(None) => None,
        Err(e) => {
            tracing::warn!(binding_id = %binding_id, error = %e, context, "PTY: session_id disk backfill failed");
            None
        }
    }
}

/// Discover the newest native session for the binding on its device and
/// record it; the device answers, the database keeps the identity check.
async fn sync_session_id_from_device(
    host: &RuntimeHost,
    runtime: &RuntimeStore,
    binding_id: &str,
) -> Result<Option<String>, AppError> {
    let Some(target) = runtime.session_sync_target(binding_id).await? else {
        return Ok(None);
    };
    let session_id: Option<String> = host
        .call(HostRequest::LatestSession {
            workspace_path: target.workspace_path.clone(),
            driver_type: target.driver_type.clone(),
        })
        .await?;
    let Some(session_id) = session_id else {
        return Ok(None);
    };
    if runtime
        .record_discovered_session_id(binding_id, &target, &session_id)
        .await?
    {
        Ok(Some(session_id))
    } else {
        Ok(None)
    }
}

async fn codex_resume_session_id_from_anchor(
    host: &RuntimeHost,
    binding: &RuntimeBinding,
    managed_home: &CodexHomeReady,
    workspace_id: &str,
    company_id: &str,
) -> Option<String> {
    let home = managed_home.home_path.to_string_lossy();
    let anchor = binding.valid_terminal_session_anchor_for_context(
        Some(workspace_id),
        Some(company_id),
        Some(binding.terminal_generation()),
        Some(home.as_ref()),
    )?;
    let matches: bool = host
        .call(HostRequest::CodexAnchorMatches {
            sessions_path: managed_home.sessions_path.to_string_lossy().to_string(),
            native_session_path: anchor.native_session_path.clone(),
            session_id: anchor.session_id.clone(),
            workspace_path: binding.workspace_path.clone(),
        })
        .await
        .ok()?;
    matches.then_some(anchor.session_id)
}

/// Prepare a Codex terminal's managed home on its device and start the
/// capture window that attributes the session file it will write. Other
/// drivers pass through unchanged.
async fn prepare_codex_spawn_if_needed(
    host: &RuntimeHost,
    runtime: &RuntimeStore,
    binding: RuntimeBinding,
    binding_id: &str,
) -> Result<(RuntimeBinding, Option<String>, Option<String>), AppError> {
    if binding.driver_type != DriverType::CodexTerminal || host.terminal_is_live(binding_id).await {
        return Ok((binding, None, None));
    }

    let managed: CodexHomeReady = host
        .call(HostRequest::CodexPrepareHome {
            binding_id: binding_id.into(),
            workspace_path: binding.workspace_path.clone(),
            harness_account: binding.config_json.clone(),
        })
        .await?;
    let workspace_id = binding
        .config_json
        .get("agent_workspace_id")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let company_id = binding
        .config_json
        .get("conversation_workspace_id")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    let mut binding = match binding.valid_terminal_session_anchor_for_context(
        Some(&workspace_id),
        Some(&company_id),
        Some(binding.terminal_generation()),
        Some(managed.home_path.to_string_lossy().as_ref()),
    ) {
        Some(_) => binding,
        None => {
            match reconcile_codex_terminal_session_from_capture(host, runtime, &binding).await {
                Ok(Some(updated)) => updated,
                Ok(None) => binding,
                Err(error) => {
                    tracing::warn!(
                        binding_id = %binding.id,
                        error = %error,
                        "Codex terminal capture reconciliation failed"
                    );
                    binding
                }
            }
        }
    };

    let resume_session_id =
        codex_resume_session_id_from_anchor(host, &binding, &managed, &workspace_id, &company_id)
            .await;
    binding = runtime
        .begin_codex_terminal_capture(
            &binding.id,
            CodexTerminalCaptureInput {
                binding_id: binding.id.clone(),
                conversation_id: binding.conversation_id.clone(),
                agent_principal_id: binding.agent_principal_id.clone(),
                company_id,
                driver_type: binding.driver_type.as_str().into(),
                workspace_id,
                workspace_path: binding.workspace_path.clone(),
                native_home_path: managed.home_path.to_string_lossy().to_string(),
                sessions_path: managed.sessions_path.to_string_lossy().to_string(),
                spawn_started_at: chrono::Utc::now(),
                baseline_session_files: managed.baseline_session_files,
                binding_updated_at: binding.updated_at,
            },
        )
        .await?;

    Ok((
        binding,
        resume_session_id,
        Some(managed.home_path.to_string_lossy().to_string()),
    ))
}

pub(crate) async fn capture_codex_terminal_before_cleanup(
    host: &RuntimeHost,
    runtime: &RuntimeStore,
    binding_id: &str,
    fallback_binding: RuntimeBinding,
) -> Result<Option<RuntimeBinding>, AppError> {
    let latest_binding = runtime
        .get_binding(binding_id)
        .await
        .unwrap_or(fallback_binding);
    reconcile_codex_terminal_session_from_capture(host, runtime, &latest_binding).await
}

fn terminal_spec(
    binding: &RuntimeBinding,
    cols: u16,
    rows: u16,
    resume_session_id: Option<String>,
    codex_home: Option<String>,
) -> TerminalSpec {
    TerminalSpec {
        terminal_id: binding.id.clone(),
        driver_type: binding.driver_type.as_str().to_string(),
        binary_path: binding
            .config_json
            .get("binary_path")
            .and_then(|value| value.as_str())
            .map(str::to_owned),
        workspace_path: binding.workspace_path.clone(),
        cols,
        rows,
        resume_session_id,
        codex_home,
        model: binding
            .config_json
            .get("model")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        harness_account: binding.config_json.clone(),
    }
}

// ── Authorization ─────────────────────────────────────────────────────

async fn authorize_terminal_binding(
    state: &ApiState,
    principal: &Principal,
    binding_id: &str,
) -> Result<RuntimeBinding, ApiError> {
    if !matches!(principal.principal_type, PrincipalType::Human) {
        return Err(ApiError(AppError::Forbidden(
            "terminal bindings are only available to human callers".into(),
        )));
    }

    let binding = state.runtime.get_binding(binding_id).await?;
    if !is_terminal_driver(&binding.driver_type) {
        return Err(ApiError(AppError::Forbidden(
            "runtime binding is not a terminal binding".into(),
        )));
    }
    if matches!(binding.state, BindingState::Disabled) {
        return Err(ApiError(AppError::Forbidden(
            "runtime binding is disabled".into(),
        )));
    }

    let conversation = state.db.get_conversation(&binding.conversation_id).await?;
    if conversation.conversation_type != ConversationType::Direct {
        return Err(ApiError(AppError::Forbidden(
            "terminal bindings are only available for direct conversations".into(),
        )));
    }
    if !conversation.members.contains_key(&principal.id) {
        return Err(ApiError(AppError::Forbidden(
            "caller is not a member of the terminal conversation".into(),
        )));
    }
    if !conversation
        .members
        .contains_key(&binding.agent_principal_id)
    {
        return Err(ApiError(AppError::Forbidden(
            "agent is not a member of the terminal conversation".into(),
        )));
    }

    let agent = state.db.get_principal(&binding.agent_principal_id).await?;
    if agent.disabled {
        return Err(ApiError(AppError::Forbidden(
            "agent principal is disabled".into(),
        )));
    }
    let allowed_workspaces = accessible_workspace_ids(&state.db, &principal.id).await?;
    if !allowed_workspaces.contains(&conversation.workspace_id)
        || !allowed_workspaces.contains(&agent.workspace_id)
    {
        return Err(ApiError(AppError::Forbidden(
            "cross-workspace access denied".into(),
        )));
    }
    ensure_terminal_workspace_active(state, &conversation.workspace_id).await?;
    ensure_terminal_workspace_active(state, &agent.workspace_id).await?;

    let mut binding = binding;
    let mut config = binding.config_json.as_object().cloned().unwrap_or_default();
    config.insert("agent_workspace_id".into(), json!(agent.workspace_id));
    config.insert(
        "conversation_workspace_id".into(),
        json!(conversation.workspace_id),
    );
    binding.config_json = serde_json::Value::Object(config);
    Ok(binding)
}

async fn ensure_terminal_workspace_active(
    state: &ApiState,
    workspace_id: &str,
) -> Result<(), ApiError> {
    match state.db.get_company(workspace_id).await {
        Ok(company) if company.deleted_at.is_some() || company.archived_at.is_some() => {
            Err(ApiError(AppError::Forbidden(
                "terminal workspace is not active".into(),
            )))
        }
        Ok(_) | Err(AppError::NotFound(_)) => Ok(()),
        Err(error) => Err(ApiError(error)),
    }
}

#[cfg(test)]
fn codex_session_provenance_matches(
    binding: &RuntimeBinding,
    binding_id: &str,
    expected_mode: &str,
) -> bool {
    binding
        .config_json
        .get("external_session_provenance")
        .and_then(|v| v.as_str())
        == Some("process_captured")
        && binding
            .config_json
            .get("external_session_binding_id")
            .and_then(|v| v.as_str())
            == Some(binding_id)
        && binding
            .config_json
            .get("external_session_driver_type")
            .and_then(|v| v.as_str())
            == Some(binding.driver_type.as_str())
        && binding
            .config_json
            .get("external_session_mode")
            .and_then(|v| v.as_str())
            == Some(expected_mode)
}

// ── WebSocket terminal proxy ──────────────────────────────────────────

/// Import recovery takes the session-identity lock before this launch lock;
/// terminal launch never takes a session-identity lock.
pub(crate) async fn lock_terminal_launch(
    transaction: &tokio_postgres::Transaction<'_>,
    binding_id: &str,
) -> Result<(), ApiError> {
    transaction
        .batch_execute("SET LOCAL lock_timeout = '90s'")
        .await
        .map_err(|error| {
            ApiError(AppError::Internal(format!(
                "bound terminal launch lock: {error}"
            )))
        })?;
    transaction
        .query_one(
            "SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0::bigint))",
            &[&format!("terminal-launch:{binding_id}")],
        )
        .await
        .map_err(|error| ApiError(AppError::Internal(format!("lock terminal launch: {error}"))))?;
    Ok(())
}

pub(crate) async fn websocket_terminal(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(binding_id): Path<String>,
    Query(query): Query<TerminalQuery>,
) -> Result<impl IntoResponse, ApiError> {
    // A browser WebSocket cannot set headers, so the token arrives as a query
    // parameter and is folded into the usual Authorization header.
    let auth_headers = if query.token.is_some() && bearer_token_value(&headers).is_none() {
        let mut h = headers.clone();
        if let Some(ref tok) = query.token {
            let val = format!("Bearer {tok}");
            h.insert(
                axum::http::header::AUTHORIZATION,
                val.parse().map_err(|_| {
                    ApiError::from(AppError::Unauthorized("invalid token encoding".into()))
                })?,
            );
        }
        h
    } else {
        headers.clone()
    };

    let principal = authenticated_principal(&auth_headers, &state).await?;
    let binding = authorize_terminal_binding(&state, &principal, &binding_id).await?;
    RuntimeHost::for_binding(&state, &binding)?;

    let cols = query.cols.unwrap_or(120);
    let rows = query.rows.unwrap_or(40);

    Ok(ws.on_upgrade(move |socket| {
        terminal_bridge(socket, state, principal, binding_id, cols, rows)
    }))
}

/// Bridge a WebSocket to the binding's terminal on its device. Bytes go down
/// as binary frames; text frames come up as keystrokes except the JSON
/// `{"type":"resize","cols":N,"rows":N}` message.
async fn terminal_bridge(
    socket: WebSocket,
    state: ApiState,
    principal: Principal,
    binding_id: String,
    cols: u16,
    rows: u16,
) {
    use futures_util::{SinkExt, StreamExt};

    let bridge_started = std::time::Instant::now();
    let (host, binding, newly_created, _) =
        match ensure_authorized_terminal(&state, &principal, &binding_id, cols, rows).await {
            Ok(ready) => ready,
            Err(error) => {
                tracing::error!(binding_id, error = ?error, "failed to ensure pty session");
                return;
            }
        };
    let runtime = state.runtime.clone();
    let attachment = match host.attach_terminal(&binding_id).await {
        Ok(attachment) => attachment,
        Err(error) => {
            tracing::error!(error = %error, "failed to attach to pty session");
            return;
        }
    };

    let (mut ws_sender, mut ws_receiver) = socket.split();
    let attachment_id = choruz_common::new_id();
    record_terminal_activity(
        &state,
        &principal,
        &binding,
        "terminal.attached",
        json!({
            "attachment_id": attachment_id, "cols": cols, "rows": rows,
            "newly_created": newly_created,
        }),
    )
    .await;
    let input_bytes = Arc::new(AtomicU64::new(0));
    let output_bytes = Arc::new(AtomicU64::new(0));
    let resize_count = Arc::new(AtomicU64::new(0));
    tracing::info!(
        binding_id = %binding_id,
        driver_type = binding.driver_type.as_str(),
        newly_created,
        spawn_ms = bridge_started.elapsed().as_millis() as u64,
        replay_frames = attachment.replay.len(),
        "PTY: session ready for websocket"
    );

    // Task A: terminal output -> WebSocket. The output stream ends when the
    // child exits, and the Close frame lets the client reconnect.
    let is_codex_terminal = binding.driver_type == DriverType::CodexTerminal;
    let host_for_capture = host.clone();
    let runtime_for_capture = runtime.clone();
    let binding_for_capture = binding.clone();
    let binding_id_for_first_output = binding_id.clone();
    let sent_bytes = output_bytes.clone();
    let mut send_task = tokio::spawn(async move {
        let mut binding_for_capture = binding_for_capture;
        let capture_workspace_id = binding_for_capture
            .config_json
            .get("agent_workspace_id")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let capture_company_id = binding_for_capture
            .config_json
            .get("conversation_workspace_id")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let mut captured_terminal_session_id = binding_for_capture
            .valid_terminal_session_anchor_for_context(
                capture_workspace_id.as_deref(),
                capture_company_id.as_deref(),
                Some(binding_for_capture.terminal_generation()),
                binding_for_capture
                    .codex_terminal_capture_metadata()
                    .as_ref()
                    .map(|capture| capture.native_home_path.as_str()),
            )
            .map(|anchor| anchor.session_id)
            .filter(|s| !s.is_empty());
        let mut terminal_capture_disabled = false;
        // The first byte the browser receives after "Connecting to terminal..."
        // is the CLI's own startup output; a slow CLI shows up here, not in
        // spawn_ms.
        let mut first_output_logged = false;
        let mut log_first_output = |bytes: usize| {
            if !first_output_logged {
                first_output_logged = true;
                tracing::info!(
                    binding_id = %binding_id_for_first_output,
                    first_output_ms = bridge_started.elapsed().as_millis() as u64,
                    bytes,
                    "PTY: first output forwarded to websocket"
                );
            }
        };
        let mut output = attachment.output;
        let mut replay = attachment.replay.into_iter();
        loop {
            let data = match replay.next() {
                Some(data) => data,
                None => match output.recv().await {
                    Some(data) => data,
                    None => {
                        tracing::info!(
                            "PTY child exited; closing WebSocket so client can reconnect"
                        );
                        let _ = ws_sender.send(WsMessage::Close(None)).await;
                        break;
                    }
                },
            };
            if is_codex_terminal {
                try_capture_codex_terminal_session(
                    &host_for_capture,
                    &runtime_for_capture,
                    &mut binding_for_capture,
                    &mut captured_terminal_session_id,
                    &mut terminal_capture_disabled,
                )
                .await;
            }
            log_first_output(data.len());
            let size = data.len();
            if ws_sender
                .send(WsMessage::Binary(data.into()))
                .await
                .is_err()
            {
                break;
            }
            sent_bytes.fetch_add(size as u64, Ordering::Relaxed);
        }
    });

    // Task B: WebSocket -> terminal input and resize.
    let host_for_input = host.clone();
    let binding_id_for_input = binding_id.clone();
    let written_bytes = input_bytes.clone();
    let resized = resize_count.clone();
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_receiver.next().await {
            let data = match msg {
                WsMessage::Text(text) => {
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text)
                        && parsed.get("type").and_then(|v| v.as_str()) == Some("resize")
                    {
                        let new_cols =
                            parsed.get("cols").and_then(|v| v.as_u64()).unwrap_or(120) as u16;
                        let new_rows =
                            parsed.get("rows").and_then(|v| v.as_u64()).unwrap_or(40) as u16;
                        if let Err(e) = host_for_input
                            .resize_terminal(&binding_id_for_input, new_cols, new_rows)
                            .await
                        {
                            tracing::warn!(binding_id = %binding_id_for_input, error = %e, "PTY resize failed");
                        } else {
                            resized.fetch_add(1, Ordering::Relaxed);
                        }
                        continue;
                    }
                    text.as_bytes().to_vec()
                }
                WsMessage::Binary(data) => data.to_vec(),
                WsMessage::Close(_) => break,
                _ => continue,
            };
            if let Err(e) = host_for_input
                .write_terminal(&binding_id_for_input, &data)
                .await
            {
                tracing::warn!(binding_id = %binding_id_for_input, error = %e, "PTY stdin write failed");
                break;
            }
            written_bytes.fetch_add(data.len() as u64, Ordering::Relaxed);
        }
    });

    // Stop and join the other leg before recording final counters or closing the PTY.
    let ended_leg = tokio::select! {
        _ = &mut send_task => {
            recv_task.abort();
            let _ = recv_task.await;
            "output"
        }
        _ = &mut recv_task => {
            send_task.abort();
            let _ = send_task.await;
            "input"
        }
    };
    record_terminal_activity(
        &state,
        &principal,
        &binding,
        "terminal.detached",
        json!({
            "attachment_id": attachment_id, "ended_leg": ended_leg,
            "duration_ms": bridge_started.elapsed().as_millis() as u64,
            "input_bytes": input_bytes.load(Ordering::Relaxed),
            "output_bytes": output_bytes.load(Ordering::Relaxed),
            "resize_count": resize_count.load(Ordering::Relaxed),
        }),
    )
    .await;

    if is_codex_terminal {
        match capture_codex_terminal_before_cleanup(&host, &runtime, &binding_id, binding).await {
            Ok(Some(_)) => tracing::info!(
                binding_id = %binding_id,
                "captured Codex terminal session before PTY cleanup"
            ),
            Ok(None) => {}
            Err(error) => tracing::warn!(
                binding_id = %binding_id,
                error = %error,
                "Codex terminal session capture before PTY cleanup failed"
            ),
        }
    } else {
        // Persist the active CLI's exact-workspace session before killing the
        // PTY, while its process context is still intact.
        match sync_session_id_from_device(&host, &runtime, &binding_id).await {
            Ok(Some(sid)) => tracing::info!(
                binding_id = %binding_id,
                session_id = %sid,
                "PTY: captured session_id before disconnect cleanup"
            ),
            Ok(None) => {}
            Err(error) => tracing::warn!(
                binding_id = %binding_id,
                error = %error,
                "PTY: pre-cleanup session_id capture failed"
            ),
        }
    }

    // WS disconnected → kill the terminal so the next connection does a fresh
    // --resume and the user always sees the full CLI on reconnect.
    if let Err(error) = host.close_terminal(&binding_id).await {
        tracing::warn!(binding_id, %error, "terminal disconnect cleanup failed");
    }
}

// ── REST terminal endpoints ───────────────────────────────────────────

/// Record transport activity, never terminal contents or reconstructed keystrokes.
/// A telemetry storage failure is logged without changing the terminal's outcome.
async fn record_terminal_activity(
    state: &ApiState,
    principal: &Principal,
    binding: &RuntimeBinding,
    action: &str,
    mut metadata: serde_json::Value,
) {
    metadata["conversation_id"] = json!(binding.conversation_id);
    metadata["agent_id"] = json!(binding.agent_principal_id);
    metadata["company_id"] = binding
        .config_json
        .get("conversation_workspace_id")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    metadata["runtime_host_id"] = binding
        .config_json
        .get("runtime_host_id")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    metadata["harness"] = json!(binding.driver_type.as_str());
    metadata["harness_account_id"] = binding
        .config_json
        .get("harness_account_id")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    if let Err(error) = state
        .db
        .record_audit(
            &principal.workspace_id,
            &principal.id,
            action,
            "runtime_binding",
            &binding.id,
            metadata,
        )
        .await
    {
        tracing::error!(action, binding_id = %binding.id, %error, "terminal activity persistence failed");
    }
}

pub(crate) async fn ensure_terminal(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(binding_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    let (_, _, newly_created, resumed) =
        ensure_authorized_terminal(&state, &principal, &binding_id, 120, 40).await?;
    Ok(Json(json!({
        "status": "ok",
        "binding_id": binding_id,
        "newly_created": newly_created,
        "resumed_session": resumed && newly_created,
    })))
}

async fn ensure_authorized_terminal(
    state: &ApiState,
    principal: &Principal,
    binding_id: &str,
    cols: u16,
    rows: u16,
) -> Result<(RuntimeHost, RuntimeBinding, bool, bool), ApiError> {
    let mut client = state.runtime.connect().await?;
    let transaction = client.transaction().await.map_err(|error| {
        ApiError(AppError::Internal(format!(
            "begin terminal launch: {error}"
        )))
    })?;
    lock_terminal_launch(&transaction, binding_id).await?;
    let binding = authorize_terminal_binding(state, principal, binding_id).await?;
    let host = RuntimeHost::for_binding(state, &binding)?;

    let session_id_owned = terminal_resume_session_id(
        &host,
        &state.runtime,
        &binding,
        binding_id,
        "ensure_terminal",
    )
    .await;
    let (binding, codex_resume_session_id, codex_home) =
        prepare_codex_spawn_if_needed(&host, &state.runtime, binding, binding_id)
            .await
            .map_err(ApiError::from)?;
    let effective_session_id = if codex_home.is_some() {
        codex_resume_session_id
    } else {
        session_id_owned
    };
    let resumed = effective_session_id.is_some();
    let spec = terminal_spec(&binding, cols, rows, effective_session_id, codex_home);
    let newly_created = host.ensure_terminal(spec).await.map_err(ApiError::from)?;
    transaction.commit().await.map_err(|error| {
        ApiError(AppError::Internal(format!(
            "finish terminal launch: {error}"
        )))
    })?;
    Ok((host, binding, newly_created, resumed))
}

#[derive(Debug, Deserialize)]
pub(crate) struct TerminalInput {
    data: String,
}

pub(crate) async fn terminal_input(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(binding_id): Path<String>,
    Json(payload): Json<TerminalInput>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    let binding = authorize_terminal_binding(&state, &principal, &binding_id).await?;
    let host = RuntimeHost::for_binding(&state, &binding)?;

    let submission_id = choruz_common::new_id();
    let trace_id = headers
        .get("x-trace-id")
        .and_then(|value| value.to_str().ok());
    record_terminal_activity(
        &state,
        &principal,
        &binding,
        "terminal.submit_started",
        json!({
            "submission_id": submission_id, "input_bytes": payload.data.len(), "trace_id": trace_id,
        }),
    )
    .await;
    let result = submit_terminal_input(&host, &binding_id, &payload.data).await;
    record_terminal_activity(&state, &principal, &binding, "terminal.submit_finished", json!({
            "submission_id": submission_id, "outcome": if result.is_ok() { "succeeded" } else { "failed" }, "trace_id": trace_id,
    })).await;
    result.map_err(ApiError::from)?;
    Ok(Json(json!({"status": "ok"})))
}

async fn submit_terminal_input(
    host: &RuntimeHost,
    binding_id: &str,
    text: &str,
) -> Result<(), AppError> {
    // Multi-line input goes in as one bracketed paste so the CLI does not
    // submit on the first newline; the trailing carriage return submits it.
    let needs_bracketed_paste = text.contains('\n');
    let mut data = Vec::with_capacity(text.len() + 16);
    if needs_bracketed_paste {
        data.extend_from_slice(b"\x1b[200~");
    }
    data.extend_from_slice(text.as_bytes());
    if needs_bracketed_paste {
        data.extend_from_slice(b"\x1b[201~");
    }
    host.write_terminal(binding_id, &data).await?;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    host.write_terminal(binding_id, b"\r").await
}

#[cfg(test)]
mod tests {
    use super::{
        codex_resume_session_id_from_anchor, codex_session_provenance_matches,
        import_codex_terminal_session, terminal_capture_error_is_permanent,
        terminal_resume_session_id,
    };
    use crate::host_runtime::{LocalHost, RuntimeHost};
    use choruz_agent_runtime::{BindingState, DriverType, RuntimeBinding, RuntimeStore};
    use choruz_host_runtime::CodexHomeReady;
    use chrono::Utc;
    use serde_json::json;
    use std::fs;

    fn local_host() -> RuntimeHost {
        RuntimeHost::Local(LocalHost::new())
    }

    #[test]
    fn terminal_capture_stops_retrying_when_binding_context_is_gone() {
        assert!(terminal_capture_error_is_permanent(
            &choruz_common::AppError::NotFound("runtime binding context changed".into())
        ));
        assert!(!terminal_capture_error_is_permanent(
            &choruz_common::AppError::Internal("temporary database failure".into())
        ));
    }

    fn isolated_test_dir(name: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("choruz-{name}-{unique}"));
        fs::create_dir_all(&dir).expect("create isolated test dir");
        dir
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set_path(key: &'static str, value: &std::path::Path) -> Self {
            let guard = Self {
                key,
                previous: std::env::var_os(key),
            };
            unsafe {
                std::env::set_var(key, value);
            }
            guard
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            unsafe {
                if let Some(previous) = &self.previous {
                    std::env::set_var(self.key, previous);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    fn write_codex_session_meta(path: &std::path::Path, session_id: &str, cwd: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create session parent");
        }
        fs::write(
            path,
            format!(r#"{{"type":"session_meta","payload":{{"id":"{session_id}","cwd":"{cwd}"}}}}"#)
                + "\n",
        )
        .expect("write session meta");
    }

    fn runtime_binding_with_config(config_json: serde_json::Value) -> RuntimeBinding {
        RuntimeBinding {
            id: "binding-1".into(),
            conversation_id: "conversation-1".into(),
            agent_principal_id: "agent-1".into(),
            driver_type: DriverType::CodexTerminal,
            workspace_path: "/workspace".into(),
            git_worktree_path: None,
            external_session_id: Some("session-1".into()),
            external_thread_id: None,
            last_event_cursor: 0,
            last_acked_event_cursor: 0,
            last_seen_server_seq: 0,
            state: BindingState::Idle,
            last_error: None,
            in_flight_turn_id: None,
            last_trigger_message_id: None,
            config_json,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn codex_terminal_provenance_accepts_matching_terminal_binding() {
        let binding = runtime_binding_with_config(json!({
            "external_session_provenance": "process_captured",
            "external_session_driver_type": "codex_terminal",
            "external_session_binding_id": "binding-1",
            "external_session_mode": "terminal",
            "external_session_captured_at": "2026-05-11T00:00:00Z"
        }));

        assert!(codex_session_provenance_matches(
            &binding,
            "binding-1",
            "terminal"
        ));
    }

    #[test]
    fn codex_terminal_resume_uses_terminal_session_anchor() {
        let binding = runtime_binding_with_config(json!({
            "terminal_session": {
                "driver_type": "codex_terminal",
                "session_id": "00000000-0000-0000-0000-000000000001",
                "source": "native_cli",
                "provenance": "terminal_process_captured",
                "binding_id": "binding-1",
                "conversation_id": "conversation-1",
                "agent_principal_id": "agent-1",
                "workspace_id": "workspace-1",
                "workspace_path": "/workspace",
                "captured_at": "2026-05-28T00:00:00Z"
            },
            "external_session_provenance": "process_captured",
            "external_session_driver_type": "codex_terminal",
            "external_session_binding_id": "binding-1",
            "external_session_mode": "headless",
            "external_session_captured_at": "2026-05-11T00:00:00Z"
        }));

        assert_eq!(
            binding
                .valid_terminal_session_id_for_workspace(Some("workspace-1"))
                .as_deref(),
            Some("00000000-0000-0000-0000-000000000001")
        );
    }

    #[test]
    fn codex_terminal_anchor_rejects_stale_context() {
        let binding = runtime_binding_with_config(json!({
            "terminal_session": {
                "driver_type": "codex_terminal",
                "session_id": "00000000-0000-0000-0000-000000000001",
                "source": "native_cli",
                "provenance": "terminal_process_captured",
                "binding_id": "binding-1",
                "conversation_id": "other-conversation",
                "agent_principal_id": "agent-1",
                "workspace_id": "workspace-1",
                "workspace_path": "/workspace",
                "captured_at": "2026-05-28T00:00:00Z"
            }
        }));

        assert_eq!(binding.valid_terminal_session_id(), None);
    }

    #[test]
    fn codex_terminal_anchor_rejects_stale_workspace_id() {
        let binding = runtime_binding_with_config(json!({
            "terminal_session": {
                "driver_type": "codex_terminal",
                "session_id": "00000000-0000-0000-0000-000000000001",
                "source": "native_cli",
                "provenance": "terminal_process_captured",
                "binding_id": "binding-1",
                "conversation_id": "conversation-1",
                "agent_principal_id": "agent-1",
                "workspace_id": "workspace-1",
                "workspace_path": "/workspace",
                "captured_at": "2026-05-28T00:00:00Z"
            }
        }));

        assert_eq!(
            binding.valid_terminal_session_id_for_workspace(Some("workspace-2")),
            None
        );
    }

    #[test]
    fn codex_terminal_anchor_rejects_owner_tuple_mismatches() {
        let base_anchor = json!({
            "driver_type": "codex_terminal",
            "session_id": "00000000-0000-0000-0000-000000000001",
            "source": "native_cli",
            "provenance": "terminal_process_captured",
            "binding_id": "binding-1",
            "conversation_id": "conversation-1",
            "agent_principal_id": "agent-1",
            "workspace_id": "workspace-1",
            "workspace_path": "/workspace",
            "company_id": "company-1",
            "native_home_path": "/runtime/codex-homes/binding-1",
            "binding_generation": 7,
            "captured_at": "2026-05-28T00:00:00Z"
        });

        for (field, value) in [
            ("binding_id", json!("other-binding")),
            ("conversation_id", json!("other-conversation")),
            ("agent_principal_id", json!("other-agent")),
            ("driver_type", json!("claude_terminal")),
            ("workspace_path", json!("/other-workspace")),
        ] {
            let mut anchor = base_anchor.clone();
            anchor[field] = value;
            let binding = runtime_binding_with_config(json!({
                "terminal_generation": 7,
                "terminal_session": anchor
            }));

            assert_eq!(
                binding.valid_terminal_session_anchor_for_context(
                    Some("workspace-1"),
                    Some("company-1"),
                    Some(7),
                    Some("/runtime/codex-homes/binding-1"),
                ),
                None,
                "anchor with mismatched {field} must not validate"
            );
        }

        let binding = runtime_binding_with_config(json!({
            "terminal_generation": 7,
            "terminal_session": base_anchor
        }));
        assert!(
            binding
                .valid_terminal_session_anchor_for_context(
                    Some("other-workspace"),
                    Some("company-1"),
                    Some(7),
                    Some("/runtime/codex-homes/binding-1"),
                )
                .is_none(),
            "anchor with mismatched workspace id must not validate"
        );
    }

    #[test]
    fn codex_terminal_provenance_rejects_headless_mode() {
        let binding = runtime_binding_with_config(json!({
            "external_session_provenance": "process_captured",
            "external_session_driver_type": "codex_terminal",
            "external_session_binding_id": "binding-1",
            "external_session_mode": "headless",
            "external_session_captured_at": "2026-05-11T00:00:00Z"
        }));

        assert!(!codex_session_provenance_matches(
            &binding,
            "binding-1",
            "terminal"
        ));
    }

    #[tokio::test]
    async fn codex_terminal_resume_path_rejects_wrong_binding_session_id() {
        let binding = runtime_binding_with_config(json!({
            "external_session_provenance": "process_captured",
            "external_session_driver_type": "codex_terminal",
            "external_session_binding_id": "binding-from-another-agent",
            "external_session_mode": "terminal",
            "external_session_captured_at": "2026-05-11T00:00:00Z"
        }));
        let runtime = RuntimeStore::new("host=127.0.0.1 port=1 user=unused dbname=unused");

        assert!(!codex_session_provenance_matches(
            &binding,
            "binding-1",
            "terminal"
        ));
        assert_eq!(
            terminal_resume_session_id(&local_host(), &runtime, &binding, "binding-1", "test")
                .await
                .as_deref(),
            None
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn imported_codex_session_becomes_a_binding_scoped_terminal_anchor() {
        let _env_guard = crate::test_support::api_test_env_lock().lock().await;
        let root = isolated_test_dir("imported-codex-terminal-session");
        let runtime_dir = root.join("runtime");
        let source_home = root.join("source-codex");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).expect("create workspace");
        let source_session = source_home.join("sessions/2026/09/03/imported.jsonl");
        write_codex_session_meta(
            &source_session,
            "imported-session",
            workspace.to_str().unwrap(),
        );

        let _runtime_env = EnvVarGuard::set_path("CHORUZ_RUNTIME_DIR", &runtime_dir);
        let _codex_home_env = EnvVarGuard::set_path("CODEX_HOME", &source_home);
        let host = local_host();
        let anchor = import_codex_terminal_session(
            &host,
            "binding-1",
            "conversation-1",
            "agent-1",
            "company-1",
            "workspace-1",
            workspace.to_str().unwrap(),
            "imported-session",
            json!({}),
        )
        .await
        .expect("register imported Codex session");

        let managed_home = anchor["native_home_path"].as_str().expect("managed home");
        let copied_path = std::path::PathBuf::from(
            anchor["native_session_path"]
                .as_str()
                .expect("copied session path"),
        );
        assert!(copied_path.starts_with(managed_home));
        assert_eq!(
            fs::read_to_string(&copied_path).unwrap(),
            fs::read_to_string(source_session).unwrap()
        );

        let mut binding = runtime_binding_with_config(json!({
            "terminal_generation": 0,
            "native_session_import": {
                "native_session_id": "imported-session",
                "workspace_path": workspace,
            },
            "terminal_session": anchor,
        }));
        binding.workspace_path = workspace.to_string_lossy().into_owned();
        let managed = CodexHomeReady {
            home_path: fs::canonicalize(managed_home).expect("canonical managed home"),
            sessions_path: fs::canonicalize(
                std::path::PathBuf::from(managed_home).join("sessions"),
            )
            .expect("canonical managed sessions"),
            baseline_session_files: Vec::new(),
        };
        assert!(
            binding
                .valid_terminal_session_anchor_for_context(
                    Some("workspace-1"),
                    Some("company-1"),
                    Some(0),
                    Some(managed.home_path.to_string_lossy().as_ref()),
                )
                .is_some(),
            "imported anchor must validate before the file check: {:#?}",
            binding.terminal_session_anchor(),
        );
        assert_eq!(
            codex_resume_session_id_from_anchor(
                &host,
                &binding,
                &managed,
                "workspace-1",
                "company-1"
            )
            .await
            .as_deref(),
            Some("imported-session")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn codex_resume_anchor_requires_managed_home_session_file() {
        let root = isolated_test_dir("codex-resume-anchor");
        let home = root.join("home");
        let sessions = home.join("sessions");
        fs::create_dir_all(&sessions).expect("create sessions");
        let session_path = sessions.join("2026/05/29/session.jsonl");
        write_codex_session_meta(&session_path, "session-1", "/workspace");
        let session_path = fs::canonicalize(session_path).unwrap();
        let managed = CodexHomeReady {
            home_path: fs::canonicalize(&home).unwrap(),
            sessions_path: fs::canonicalize(&sessions).unwrap(),
            baseline_session_files: Vec::new(),
        };
        let binding = runtime_binding_with_config(json!({
            "terminal_generation": 7,
            "terminal_session": {
                "driver_type": "codex_terminal",
                "session_id": "session-1",
                "source": "native_cli",
                "provenance": "terminal_process_captured",
                "binding_id": "binding-1",
                "conversation_id": "conversation-1",
                "agent_principal_id": "agent-1",
                "company_id": "company-1",
                "workspace_id": "workspace-1",
                "workspace_path": "/workspace",
                "native_home_path": managed.home_path.to_string_lossy(),
                "native_session_path": session_path.to_string_lossy(),
                "binding_generation": 7,
                "captured_at": "2026-05-29T00:00:00Z"
            }
        }));
        let host = local_host();

        assert_eq!(
            codex_resume_session_id_from_anchor(
                &host,
                &binding,
                &managed,
                "workspace-1",
                "company-1"
            )
            .await
            .as_deref(),
            Some("session-1")
        );

        let other_home = root.join("other-home");
        fs::create_dir_all(other_home.join("sessions")).expect("create other sessions");
        let other_managed = CodexHomeReady {
            home_path: fs::canonicalize(&other_home).unwrap(),
            sessions_path: fs::canonicalize(other_home.join("sessions")).unwrap(),
            baseline_session_files: Vec::new(),
        };
        assert_eq!(
            codex_resume_session_id_from_anchor(
                &host,
                &binding,
                &other_managed,
                "workspace-1",
                "company-1"
            )
            .await,
            None
        );
    }
}
