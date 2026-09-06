use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
};

use axum::{Json, extract::State, http::HeaderMap};
use choruz_agent_runtime::{HarnessKind, NativeSessionSummary, SessionAccount, SessionScanResult};
use choruz_common::AppError;
use choruz_host_runtime::HostRequest;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    ApiError, ApiState,
    handlers_terminals::{import_codex_terminal_session, lock_terminal_launch},
    host_runtime::RuntimeHost,
    require_human_operator,
};

/// A connected runtime host that belongs to `company_id`.
async fn company_host(
    state: &ApiState,
    company_id: &str,
    host_id: &str,
) -> Result<RuntimeHost, ApiError> {
    let client = state.event_store.connect().await.map_err(ApiError::from)?;
    let owned = client
        .query_opt(
            "SELECT 1 FROM runtime_host WHERE id = $1 AND company_id = $2 AND revoked_at IS NULL",
            &[&host_id, &company_id],
        )
        .await
        .map_err(|error| ApiError(AppError::Internal(format!("find runtime host: {error}"))))?
        .is_some();
    if !owned {
        return Err(ApiError(AppError::NotFound(format!(
            "runtime host {host_id}"
        ))));
    }
    RuntimeHost::for_host(state, host_id).map_err(ApiError)
}

pub(crate) async fn session_accounts(
    state: &ApiState,
    company_id: Option<&str>,
    host_id: Option<&str>,
) -> Result<Vec<SessionAccount>, ApiError> {
    let Some(company_id) = company_id else {
        return Ok(Vec::new());
    };
    let client = state.event_store.connect().await.map_err(ApiError::from)?;
    let rows = client
        .query(
            "SELECT id, driver_type, name FROM harness_account WHERE company_id = $1
         AND runtime_host_id IS NOT DISTINCT FROM $2 AND profile_kind = 'isolated'
         AND disabled_at IS NULL AND status = 'active' ORDER BY id",
            &[&company_id, &host_id],
        )
        .await
        .map_err(|error| {
            ApiError(AppError::Internal(format!(
                "list session profiles: {error}"
            )))
        })?;
    Ok(rows
        .into_iter()
        .map(|row| SessionAccount {
            account_id: row.get(0),
            name: row.get(2),
            harness: if row.get::<_, String>(1) == "claude_terminal" {
                HarnessKind::Claude
            } else {
                HarnessKind::Codex
            },
        })
        .collect())
}

#[derive(Debug, Deserialize)]
pub(crate) struct ScanWorkspaceSessionsRequest {
    company_id: Option<String>,
    workspace_path: String,
    harnesses: BTreeSet<HarnessKind>,
    runtime_host_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ImportWorkspaceSessionsRequest {
    company_id: String,
    workspace_path: String,
    sessions: Vec<ImportSessionSelection>,
    runtime_host_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ImportSessionSelection {
    harness: HarnessKind,
    #[serde(default)]
    harness_account_id: Option<String>,
    native_session_id: String,
    #[serde(default)]
    workspace_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ImportedWorkspaceSession {
    harness: HarnessKind,
    native_session_id: String,
    agent_principal_id: String,
    conversation_id: String,
    binding_id: String,
    agent_name: String,
    already_imported: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct ImportWorkspaceSessionsResponse {
    imported: Vec<ImportedWorkspaceSession>,
}

pub(crate) async fn scan_workspace_sessions(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Json(payload): Json<ScanWorkspaceSessionsRequest>,
) -> Result<Json<SessionScanResult>, ApiError> {
    require_human_operator(&headers, &state).await?;
    if payload.harnesses.is_empty() {
        return Err(ApiError(AppError::Validation(
            "select at least one harness".into(),
        )));
    }

    if let Some(company_id) = payload.company_id.as_deref() {
        crate::handlers_companies::require_company_access(&headers, &state, company_id).await?;
    }
    let accounts = session_accounts(
        &state,
        payload.company_id.as_deref(),
        payload.runtime_host_id.as_deref(),
    )
    .await?;

    if let Some(host_id) = payload.runtime_host_id.as_deref() {
        let company_id = payload.company_id.as_deref().ok_or_else(|| {
            ApiError(AppError::Validation(
                "company_id is required when scanning another device".into(),
            ))
        })?;
        let workspace = validate_remote_workspace_path(&payload.workspace_path)?;
        let result: SessionScanResult = company_host(&state, company_id, host_id)
            .await?
            .call(HostRequest::ScanSessions {
                workspace_path: workspace.to_owned(),
                harnesses: payload.harnesses,
                accounts,
            })
            .await?;
        return Ok(Json(result));
    }

    let result: SessionScanResult = RuntimeHost::local(&state)
        .call(HostRequest::ScanSessions {
            workspace_path: payload.workspace_path,
            harnesses: payload.harnesses,
            accounts,
        })
        .await?;
    Ok(Json(result))
}

pub(crate) async fn import_workspace_sessions(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Json(payload): Json<ImportWorkspaceSessionsRequest>,
) -> Result<Json<ImportWorkspaceSessionsResponse>, ApiError> {
    let operator = require_human_operator(&headers, &state).await?;
    if payload.sessions.is_empty() || payload.sessions.len() > 100 {
        return Err(ApiError(AppError::Validation(
            "select between 1 and 100 sessions".into(),
        )));
    }
    if payload.sessions.iter().any(|selection| {
        selection.native_session_id.trim().is_empty()
            || selection
                .workspace_path
                .as_deref()
                .is_some_and(|path| path.trim().is_empty())
    }) {
        return Err(ApiError(AppError::Validation(
            "native session id and workspace path cannot be empty".into(),
        )));
    }

    let accessible = state.db.list_companies(&operator.id).await?;
    let company = accessible
        .into_iter()
        .find(|company| company.id == payload.company_id)
        .ok_or_else(|| ApiError(AppError::Forbidden("company is not accessible".into())))?;
    if company.archived_at.is_some() || company.deleted_at.is_some() {
        return Err(ApiError(AppError::Forbidden(
            "cannot import sessions into an inactive company".into(),
        )));
    }

    // Re-discover immediately before mutation. A remote client can select IDs,
    // but it cannot invent a session ID or bind a session from another cwd.
    let harnesses = payload
        .sessions
        .iter()
        .map(|selection| selection.harness)
        .collect::<BTreeSet<_>>();
    let host = match payload.runtime_host_id.as_deref() {
        Some(host_id) => {
            validate_remote_workspace_path(&payload.workspace_path)?;
            company_host(&state, &payload.company_id, host_id).await?
        }
        None => RuntimeHost::local(&state),
    };
    let scan: SessionScanResult = host
        .call(HostRequest::ScanSessions {
            workspace_path: payload.workspace_path.clone(),
            harnesses,
            accounts: session_accounts(
                &state,
                Some(&payload.company_id),
                payload.runtime_host_id.as_deref(),
            )
            .await?,
        })
        .await?;
    let canonical = PathBuf::from(validate_remote_workspace_path(&scan.workspace_path)?);
    let discovered = scan
        .sessions
        .into_iter()
        .map(|session| {
            (
                (
                    session.harness,
                    session.native_session_id.clone(),
                    session.workspace_path.clone(),
                    session.harness_account_id.clone(),
                ),
                session,
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();

    let mut imported = Vec::with_capacity(payload.sessions.len());
    for selection in payload.sessions {
        let selected_workspace = selection
            .workspace_path
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| canonical.clone());
        let selected_workspace = if payload.runtime_host_id.is_some() {
            PathBuf::from(validate_remote_workspace_path(
                selected_workspace.to_string_lossy().as_ref(),
            )?)
        } else {
            tokio::fs::canonicalize(selected_workspace)
                .await
                .map_err(|error| {
                    ApiError(AppError::NotFound(format!(
                        "session workspace path not found: {error}"
                    )))
                })?
        };
        if !selected_workspace.starts_with(&canonical) {
            return Err(ApiError(AppError::Forbidden(
                "session workspace is outside the selected scan root".into(),
            )));
        }
        let selected_workspace = selected_workspace.to_string_lossy().into_owned();
        let key = (
            selection.harness,
            selection.native_session_id.clone(),
            selected_workspace,
            selection.harness_account_id,
        );
        let session = discovered.get(&key).ok_or_else(|| {
            ApiError(AppError::NotFound(format!(
                "{} session is no longer present in this workspace",
                selection.harness.label()
            )))
        })?;
        let session_workspace = PathBuf::from(&session.workspace_path);
        imported.push(
            import_one_session(
                &state,
                &host,
                &operator.id,
                &payload.company_id,
                &session_workspace,
                session,
                payload.runtime_host_id.as_deref(),
            )
            .await?,
        );
    }

    Ok(Json(ImportWorkspaceSessionsResponse { imported }))
}

#[allow(clippy::too_many_arguments)]
async fn import_one_session(
    state: &ApiState,
    host: &RuntimeHost,
    operator_id: &str,
    company_id: &str,
    workspace: &std::path::Path,
    session: &NativeSessionSummary,
    runtime_host_id: Option<&str>,
) -> Result<ImportedWorkspaceSession, ApiError> {
    host.call::<()>(HostRequest::EnsureOutboxHelper {
        workspace_path: workspace.to_string_lossy().into_owned(),
    })
    .await?;
    let mut client = state.runtime.connect().await.map_err(ApiError)?;
    let transaction = client.transaction().await.map_err(|error| {
        ApiError(AppError::Internal(format!(
            "begin native session import: {error}"
        )))
    })?;
    let driver_type = import_driver(session.harness);
    let workspace_text = workspace.to_string_lossy().into_owned();
    let import_key = native_session_import_lock_key(
        runtime_host_id,
        &workspace_text,
        driver_type,
        &session.native_session_id,
        session.harness_account_id.as_deref(),
    );
    transaction
        .query_one(
            "SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0::bigint))",
            &[&import_key],
        )
        .await
        .map_err(|error| {
            ApiError(AppError::Internal(format!(
                "lock native session import: {error}"
            )))
        })?;

    if let Some(row) = transaction
        .query_opt(
            "SELECT n.company_id, n.agent_principal_id, n.conversation_id, n.binding_id, p.name
             FROM native_session_import n
             JOIN principal p ON p.id = n.agent_principal_id
             WHERE n.runtime_host_id IS NOT DISTINCT FROM $1
               AND n.workspace_path = $2 AND n.driver_type = $3 AND n.native_session_id = $4
               AND n.harness_account_id IS NOT DISTINCT FROM $5",
            &[
                &runtime_host_id,
                &workspace_text,
                &driver_type,
                &session.native_session_id,
                &session.harness_account_id,
            ],
        )
        .await
        .map_err(|error| {
            ApiError(AppError::Internal(format!(
                "find native session import: {error}"
            )))
        })?
    {
        let imported_company_id: String = row.get(0);
        if imported_company_id != company_id {
            let previous_binding: String = row.get(3);
            let previous_agent: String = row.get(1);
            transaction
                .batch_execute("SET LOCAL lock_timeout = '90s'")
                .await
                .map_err(|error| {
                    ApiError(AppError::Internal(format!(
                        "bound session recovery locks: {error}"
                    )))
                })?;
            transaction
                .execute(
                    "SELECT pg_advisory_xact_lock(hashtext($1)::bigint)",
                    &[&previous_agent],
                )
                .await
                .map_err(|error| {
                    ApiError(AppError::Internal(format!(
                        "lock previous agent commands: {error}"
                    )))
                })?;
            lock_terminal_launch(&transaction, &previous_binding).await?;
            let commands = transaction
                .query(
                    "SELECT command_id FROM agent_commands WHERE agent_id = $1
                 AND status IN ('pending', 'leased', 'started', 'heartbeating', 'retry_scheduled')
                 ORDER BY command_id FOR UPDATE",
                    &[&previous_agent],
                )
                .await
                .map_err(|error| {
                    ApiError(AppError::Internal(format!(
                        "check previous agent commands: {error}"
                    )))
                })?;
            let previous = transaction
                .query_one(
                    "SELECT c.owner_id, c.deleted_at, b.state, b.in_flight_turn_id
                 FROM company c JOIN agent_runtime_bindings b ON b.id = $2
                 WHERE c.id = $1 FOR UPDATE OF c, b",
                    &[&imported_company_id, &previous_binding],
                )
                .await
                .map_err(|error| {
                    ApiError(AppError::Internal(format!(
                        "lock previous session owner: {error}"
                    )))
                })?;
            if previous.get::<_, String>(0) != operator_id
                || previous
                    .get::<_, Option<chrono::DateTime<chrono::Utc>>>(1)
                    .is_none()
            {
                return Err(ApiError(AppError::Conflict(
                    "this native session is already imported into another company".into(),
                )));
            }
            if !commands.is_empty()
                || previous.get::<_, String>(2) == "running"
                || previous.get::<_, Option<String>>(3).is_some()
            {
                return Err(ApiError(AppError::Conflict(
                    "the previous agent has queued or running work; wait for it to finish before importing".into(),
                )));
            }
            host.close_terminal(&previous_binding).await?;
            transaction
                .execute(
                    "UPDATE agent_runtime_bindings SET state = 'disabled', updated_at = NOW()
                 WHERE id = $1",
                    &[&previous_binding],
                )
                .await
                .map_err(|error| {
                    ApiError(AppError::Internal(format!(
                        "retire previous session binding: {error}"
                    )))
                })?;
            transaction
                .execute(
                    "UPDATE principal SET disabled = true, updated_at = NOW() WHERE id = $1",
                    &[&row.get::<_, String>(1)],
                )
                .await
                .map_err(|error| {
                    ApiError(AppError::Internal(format!(
                        "retire previous session agent: {error}"
                    )))
                })?;
            transaction
                .execute(
                    "DELETE FROM native_session_import WHERE binding_id = $1",
                    &[&previous_binding],
                )
                .await
                .map_err(|error| {
                    ApiError(AppError::Internal(format!(
                        "release previous session claim: {error}"
                    )))
                })?;
        } else {
            transaction.commit().await.map_err(|error| {
                ApiError(AppError::Internal(format!(
                    "finish native session lookup: {error}"
                )))
            })?;
            state
                .db
                .restore_hidden_agent_session(operator_id, row.get::<_, &str>(2))
                .await?;
            return Ok(ImportedWorkspaceSession {
                harness: session.harness,
                native_session_id: session.native_session_id.clone(),
                agent_principal_id: row.get(1),
                conversation_id: row.get(2),
                binding_id: row.get(3),
                agent_name: row.get(4),
                already_imported: true,
            });
        }
    }

    let agent_name = unique_agent_name(&transaction, company_id, session).await?;
    let agent_id = choruz_common::new_id();
    let conversation_id = choruz_common::new_id();
    let binding_id = choruz_common::new_id();
    let import_id = choruz_common::new_id();
    let audit_id = choruz_common::new_id();
    let now = chrono::Utc::now();
    let mut config = json!({
        "is_primary": true,
        "agent_name": agent_name,
        "mention_aliases": [agent_name],
        "interaction_mode": "terminal",
        "native_session_import": {
            "harness": session.harness,
            "native_session_id": session.native_session_id,
            "native_title": session.title,
            "workspace_path": workspace_text,
            "verified_at": now,
        },
        "external_session_provenance": "workspace_scan_verified",
        "external_session_driver_type": driver_type,
        "external_session_binding_id": binding_id,
        "external_session_mode": "terminal",
        "external_session_captured_at": now,
    });
    if let Some(account_id) = &session.harness_account_id {
        config["harness_account_id"] = json!(account_id);
        config["harness_account_profile_kind"] = json!("isolated");
        crate::handlers_runtime::validate_harness_account(
            state,
            company_id,
            if session.harness == HarnessKind::Claude {
                choruz_agent_runtime::DriverType::ClaudeTerminal
            } else {
                choruz_agent_runtime::DriverType::CodexTerminal
            },
            runtime_host_id,
            Some(&config),
        )
        .await?;
    }
    if let Some(runtime_host_id) = runtime_host_id {
        config
            .as_object_mut()
            .expect("native session import configuration is an object")
            .insert("runtime_host_id".into(), json!(runtime_host_id));
    }
    if session.harness == choruz_agent_runtime::HarnessKind::Codex {
        let anchor = import_codex_terminal_session(
            host,
            &binding_id,
            &conversation_id,
            &agent_id,
            company_id,
            company_id,
            &workspace_text,
            &session.native_session_id,
            config.clone(),
        )
        .await?;
        let config = config
            .as_object_mut()
            .expect("native session import configuration is an object");
        config.insert("terminal_generation".into(), json!(0));
        config.insert("terminal_session".into(), anchor);
    }

    transaction
        .execute(
            "INSERT INTO principal
             (id, workspace_id, type, name, avatar_url, secret_hash, disabled, channel_visibility, created_at, updated_at)
             VALUES ($1, $2, 'agent', $3, NULL, NULL, FALSE, 'visible', $4, $4)",
            &[&agent_id, &company_id, &agent_name, &now],
        )
        .await
        .map_err(|error| ApiError(AppError::Internal(format!("create imported agent: {error}"))))?;
    transaction
        .execute(
            "INSERT INTO conversation
             (id, workspace_id, type, name, creator_id, created_at, updated_at)
             VALUES ($1, $2, 'direct', NULL, $3, $4, $4)",
            &[&conversation_id, &company_id, &operator_id, &now],
        )
        .await
        .map_err(|error| ApiError(AppError::Internal(format!("create imported DM: {error}"))))?;
    for member_id in [operator_id, agent_id.as_str()] {
        transaction
            .execute(
                "INSERT INTO conversation_member (conv_id, principal_id, joined_at)
                 VALUES ($1, $2, $3)",
                &[&conversation_id, &member_id, &now],
            )
            .await
            .map_err(|error| {
                ApiError(AppError::Internal(format!(
                    "add imported DM member: {error}"
                )))
            })?;
    }
    transaction
        .execute(
            "INSERT INTO agent_runtime_bindings
             (id, conversation_id, agent_principal_id, driver_type, workspace_path,
              git_worktree_path, external_session_id, external_thread_id,
              last_event_cursor, last_acked_event_cursor, last_seen_server_seq,
              state, config_json, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, NULL, $6, NULL, 0, 0, 0, 'idle', $7, $8, $8)",
            &[
                &binding_id,
                &conversation_id,
                &agent_id,
                &driver_type,
                &workspace_text,
                &session.native_session_id,
                &config,
                &now,
            ],
        )
        .await
        .map_err(|error| {
            ApiError(AppError::Internal(format!(
                "bind imported session: {error}"
            )))
        })?;
    transaction
        .execute(
            "INSERT INTO native_session_import
             (id, company_id, runtime_host_id, workspace_path, driver_type, native_session_id,
              agent_principal_id, conversation_id, binding_id, imported_by,
              native_title, created_at, updated_at, harness_account_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $12, $13)",
            &[
                &import_id,
                &company_id,
                &runtime_host_id,
                &workspace_text,
                &driver_type,
                &session.native_session_id,
                &agent_id,
                &conversation_id,
                &binding_id,
                &operator_id,
                &session.title,
                &now,
                &session.harness_account_id,
            ],
        )
        .await
        .map_err(|error| {
            ApiError(AppError::Internal(format!(
                "record native session import: {error}"
            )))
        })?;
    transaction
        .execute(
            "INSERT INTO audit_log
             (id, workspace_id, actor_id, action, target_type, target_id, metadata, created_at)
             VALUES ($1, $2, $3, 'native_session.imported', 'principal', $4, $5, $6)",
            &[
                &audit_id,
                &company_id,
                &operator_id,
                &agent_id,
                &json!({
                    "driver_type": driver_type,
                    "workspace_path": workspace_text,
                    "runtime_host_id": runtime_host_id,
                }),
                &now,
            ],
        )
        .await
        .map_err(|error| {
            ApiError(AppError::Internal(format!(
                "audit native session import: {error}"
            )))
        })?;
    transaction.commit().await.map_err(|error| {
        ApiError(AppError::Internal(format!(
            "commit native session import: {error}"
        )))
    })?;

    Ok(ImportedWorkspaceSession {
        harness: session.harness,
        native_session_id: session.native_session_id.clone(),
        agent_principal_id: agent_id,
        conversation_id,
        binding_id,
        agent_name,
        already_imported: false,
    })
}

async fn unique_agent_name(
    transaction: &tokio_postgres::Transaction<'_>,
    company_id: &str,
    session: &NativeSessionSummary,
) -> Result<String, ApiError> {
    let base = display_agent_name(&session.title, session.harness);
    let candidates = [
        base.clone(),
        display_agent_name(
            &format!("{} · {}", base, session.harness.label()),
            session.harness,
        ),
        display_agent_name(
            &format!(
                "{} · {}",
                base,
                session
                    .native_session_id
                    .chars()
                    .take(6)
                    .collect::<String>()
            ),
            session.harness,
        ),
    ];
    for candidate in candidates {
        let exists = transaction
            .query_opt(
                "SELECT 1 FROM principal
                 WHERE workspace_id = $1 AND lower(name) = lower($2) AND deleted_at IS NULL AND NOT online_guest",
                &[&company_id, &candidate],
            )
            .await
            .map_err(|error| {
                ApiError(AppError::Internal(format!(
                    "check imported agent name: {error}"
                )))
            })?
            .is_some();
        if !exists {
            return Ok(candidate);
        }
    }
    Err(ApiError(AppError::Conflict(
        "could not derive a unique Agent name for this session".into(),
    )))
}

fn display_agent_name(title: &str, harness: HarnessKind) -> String {
    let cleaned = title
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned = cleaned.trim();
    let value = if cleaned.is_empty() {
        format!("{} session", harness.label())
    } else {
        cleaned.to_owned()
    };
    value.chars().take(80).collect()
}

fn import_driver(harness: HarnessKind) -> &'static str {
    match harness {
        HarnessKind::Claude => "claude_terminal",
        HarnessKind::Codex => "codex_terminal",
        HarnessKind::Pi => "pi_terminal",
        HarnessKind::Grok => "grok_terminal",
        HarnessKind::OpenCode => "opencode_terminal",
    }
}

pub(crate) fn native_session_import_lock_key(
    runtime_host_id: Option<&str>,
    workspace_path: &str,
    driver_type: &str,
    native_session_id: &str,
    harness_account_id: Option<&str>,
) -> String {
    // PostgreSQL text values cannot contain NUL bytes. JSON gives the tuple a
    // stable, unambiguous representation while escaping every control byte.
    json!([
        runtime_host_id,
        workspace_path,
        driver_type,
        native_session_id,
        harness_account_id
    ])
    .to_string()
}

fn validate_remote_workspace_path(value: &str) -> Result<&str, ApiError> {
    let value = value.trim();
    let path = Path::new(value);
    if value.is_empty()
        || value.len() > 4_096
        || !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
        || value.chars().any(char::is_control)
    {
        return Err(ApiError(AppError::Validation(
            "remote workspace path must be an absolute normalized path".into(),
        )));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_agent_names_are_short_and_single_line() {
        let name = display_agent_name(
            "  Fix\nremote\tcontrol with a deliberately very long title that should never overflow the sidebar row because it is bounded  ",
            HarnessKind::Codex,
        );
        assert!(!name.contains('\n'));
        assert!(!name.contains('\t'));
        assert!(name.chars().count() <= 80);
    }

    #[test]
    fn imported_sessions_use_terminal_drivers() {
        assert_eq!(import_driver(HarnessKind::Claude), "claude_terminal");
        assert_eq!(import_driver(HarnessKind::Codex), "codex_terminal");
        assert_eq!(import_driver(HarnessKind::Pi), "pi_terminal");
        assert_eq!(import_driver(HarnessKind::Grok), "grok_terminal");
        assert_eq!(import_driver(HarnessKind::OpenCode), "opencode_terminal");
    }

    #[test]
    fn recursive_import_selections_identify_every_harness_and_workspace() {
        for harness in ["claude", "codex", "pi", "grok", "open_code"] {
            let selection: ImportSessionSelection = serde_json::from_value(json!({
                "harness": harness,
                "native_session_id": format!("{harness}-session"),
                "workspace_path": format!("/projects/{harness}")
            }))
            .expect("selection");
            assert_eq!(
                selection.workspace_path.as_deref(),
                Some(format!("/projects/{harness}").as_str())
            );
        }
    }

    #[test]
    fn exact_root_imports_remain_compatible_without_selection_workspace() {
        let selection: ImportSessionSelection = serde_json::from_value(json!({
            "harness": "codex",
            "native_session_id": "legacy-exact-root-session"
        }))
        .expect("legacy selection");
        assert!(selection.workspace_path.is_none());
    }

    #[test]
    fn native_session_import_lock_keys_are_postgres_safe_and_unambiguous() {
        let key = native_session_import_lock_key(
            None,
            "/projects/example",
            "codex_exec",
            "session\0with-control-byte",
            None,
        );
        assert!(!key.contains('\0'));
        assert_ne!(
            native_session_import_lock_key(None, "/projects/a", "bc", "d", None),
            native_session_import_lock_key(None, "/projects/ab", "c", "d", None)
        );
        assert_ne!(
            native_session_import_lock_key(Some("host-a"), "/projects/a", "bc", "d", None),
            native_session_import_lock_key(Some("host-b"), "/projects/a", "bc", "d", None)
        );
    }
}
