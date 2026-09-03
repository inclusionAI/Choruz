use std::{collections::BTreeSet, path::PathBuf};

use axum::{Json, extract::State, http::HeaderMap};
use choruz_agent_runtime::{
    HarnessKind, NativeSessionSummary, SessionCatalogScanner, SessionScanResult,
};
use choruz_common::AppError;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    ApiError, ApiState, handlers_filesystem::validate_path_whitelist,
    handlers_terminals::import_codex_terminal_session, require_human_operator,
};

const SEND_HELPER: &str = include_str!("../assets/choruz-send.sh");

#[derive(Debug, Deserialize)]
pub(crate) struct ScanWorkspaceSessionsRequest {
    workspace_path: String,
    harnesses: BTreeSet<HarnessKind>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ImportWorkspaceSessionsRequest {
    company_id: String,
    workspace_path: String,
    sessions: Vec<ImportSessionSelection>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ImportSessionSelection {
    harness: HarnessKind,
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
    let _ = require_human_operator(&headers, &state).await?;
    if payload.harnesses.is_empty() {
        return Err(ApiError(AppError::Validation(
            "select at least one harness".into(),
        )));
    }

    let canonical = tokio::fs::canonicalize(PathBuf::from(&payload.workspace_path))
        .await
        .map_err(|error| {
            ApiError(AppError::NotFound(format!(
                "workspace path not found: {error}"
            )))
        })?;
    validate_path_whitelist(&canonical)?;
    let metadata = tokio::fs::metadata(&canonical).await.map_err(|error| {
        ApiError(AppError::NotFound(format!(
            "cannot inspect workspace: {error}"
        )))
    })?;
    if !metadata.is_dir() {
        return Err(ApiError(AppError::Validation(
            "workspace path must be a directory".into(),
        )));
    }

    let scanner =
        SessionCatalogScanner::from_env().map_err(|error| ApiError(AppError::Internal(error)))?;
    let result = scanner
        .scan(&canonical, &payload.harnesses)
        .await
        .map_err(|error| ApiError(AppError::Internal(error)))?;
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

    let canonical = tokio::fs::canonicalize(PathBuf::from(&payload.workspace_path))
        .await
        .map_err(|error| {
            ApiError(AppError::NotFound(format!(
                "workspace path not found: {error}"
            )))
        })?;
    validate_path_whitelist(&canonical)?;
    // Re-discover immediately before mutation. A remote client can select IDs,
    // but it cannot invent a session ID or bind a session from another cwd.
    let harnesses = payload
        .sessions
        .iter()
        .map(|selection| selection.harness)
        .collect::<BTreeSet<_>>();
    let scanner =
        SessionCatalogScanner::from_env().map_err(|error| ApiError(AppError::Internal(error)))?;
    let scan = scanner
        .scan(&canonical, &harnesses)
        .await
        .map_err(|error| ApiError(AppError::Internal(error)))?;
    let discovered = scan
        .sessions
        .into_iter()
        .map(|session| {
            (
                (
                    session.harness,
                    session.native_session_id.clone(),
                    session.workspace_path.clone(),
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
        let selected_workspace =
            tokio::fs::canonicalize(selected_workspace)
                .await
                .map_err(|error| {
                    ApiError(AppError::NotFound(format!(
                        "session workspace path not found: {error}"
                    )))
                })?;
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
                &operator.id,
                &payload.company_id,
                &session_workspace,
                session,
            )
            .await?,
        );
    }

    Ok(Json(ImportWorkspaceSessionsResponse { imported }))
}

async fn import_one_session(
    state: &ApiState,
    operator_id: &str,
    company_id: &str,
    workspace: &std::path::Path,
    session: &NativeSessionSummary,
) -> Result<ImportedWorkspaceSession, ApiError> {
    ensure_outbox_helper(workspace).await?;
    let mut client = state.runtime.connect().await.map_err(ApiError)?;
    let transaction = client.transaction().await.map_err(|error| {
        ApiError(AppError::Internal(format!(
            "begin native session import: {error}"
        )))
    })?;
    let driver_type = import_driver(session.harness);
    // Imported sessions enter through the same terminal route as newly
    // provisioned Agents.  Terminal handlers cannot infer a binary from the
    // driver type: their historical fallback is `claude`, so persist the
    // harness-specific executable here rather than accidentally launching
    // Claude for an imported Codex/Pi/Grok/OpenCode session.
    let binary_path = import_binary(session.harness);
    let workspace_text = workspace.to_string_lossy().into_owned();
    let import_key =
        native_session_import_lock_key(&workspace_text, driver_type, &session.native_session_id);
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
             WHERE n.workspace_path = $1 AND n.driver_type = $2 AND n.native_session_id = $3",
            &[&workspace_text, &driver_type, &session.native_session_id],
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
            return Err(ApiError(AppError::Conflict(
                "this native session is already imported into another company".into(),
            )));
        }
        transaction.commit().await.map_err(|error| {
            ApiError(AppError::Internal(format!(
                "finish native session lookup: {error}"
            )))
        })?;
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
        "binary_path": binary_path,
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
        "model": session.model,
    });
    if session.harness == choruz_agent_runtime::HarnessKind::Codex {
        let anchor = import_codex_terminal_session(
            &binding_id,
            &conversation_id,
            &agent_id,
            company_id,
            company_id,
            &workspace_text,
            &session.native_session_id,
        )?;
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
             (id, company_id, workspace_path, driver_type, native_session_id,
              agent_principal_id, conversation_id, binding_id, imported_by,
              native_title, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11)",
            &[
                &import_id,
                &company_id,
                &workspace_text,
                &driver_type,
                &session.native_session_id,
                &agent_id,
                &conversation_id,
                &binding_id,
                &operator_id,
                &session.title,
                &now,
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
                &json!({"driver_type": driver_type, "workspace_path": workspace_text}),
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

async fn ensure_outbox_helper(workspace: &std::path::Path) -> Result<(), ApiError> {
    let helper_dir = workspace.join(".choruz");
    let outbox_dir = workspace.join(".choruz-outbox");
    for directory in [
        outbox_dir.join("tmp"),
        outbox_dir.join("new"),
        helper_dir.clone(),
    ] {
        tokio::fs::create_dir_all(directory)
            .await
            .map_err(|error| {
                ApiError(AppError::Internal(format!("prepare Agent outbox: {error}")))
            })?;
    }
    let helper_path = helper_dir.join("send");
    tokio::fs::write(&helper_path, SEND_HELPER)
        .await
        .map_err(|error| {
            ApiError(AppError::Internal(format!(
                "install Agent outbox helper: {error}"
            )))
        })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&helper_path, std::fs::Permissions::from_mode(0o755))
            .await
            .map_err(|error| {
                ApiError(AppError::Internal(format!(
                    "enable Agent outbox helper: {error}"
                )))
            })?;
    }
    Ok(())
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
                 WHERE workspace_id = $1 AND lower(name) = lower($2) AND deleted_at IS NULL",
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

fn import_binary(harness: HarnessKind) -> String {
    let environment_key = import_binary_environment_key(harness);
    std::env::var(environment_key)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| import_binary_fallback(harness).to_string())
}

fn import_binary_environment_key(harness: HarnessKind) -> &'static str {
    match harness {
        HarnessKind::Claude => "CHORUZ_CLAUDE_BINARY",
        HarnessKind::Codex => "CHORUZ_CODEX_BINARY",
        HarnessKind::Pi => "CHORUZ_PI_BINARY",
        HarnessKind::Grok => "CHORUZ_GROK_BINARY",
        HarnessKind::OpenCode => "CHORUZ_OPENCODE_BINARY",
    }
}

fn import_binary_fallback(harness: HarnessKind) -> &'static str {
    match harness {
        HarnessKind::Claude => "claude",
        HarnessKind::Codex => "codex",
        HarnessKind::Pi => "pi",
        HarnessKind::Grok => "grok",
        HarnessKind::OpenCode => "opencode",
    }
}

pub(crate) fn native_session_import_lock_key(
    workspace_path: &str,
    driver_type: &str,
    native_session_id: &str,
) -> String {
    // PostgreSQL text values cannot contain NUL bytes. JSON gives the tuple a
    // stable, unambiguous representation while escaping every control byte.
    json!([workspace_path, driver_type, native_session_id]).to_string()
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
    fn imported_sessions_use_their_own_terminal_binary() {
        assert_eq!(import_binary_fallback(HarnessKind::Claude), "claude");
        assert_eq!(import_binary_fallback(HarnessKind::Codex), "codex");
        assert_eq!(import_binary_fallback(HarnessKind::Pi), "pi");
        assert_eq!(import_binary_fallback(HarnessKind::Grok), "grok");
        assert_eq!(import_binary_fallback(HarnessKind::OpenCode), "opencode");
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
            "/projects/example",
            "codex_exec",
            "session\0with-control-byte",
        );
        assert!(!key.contains('\0'));
        assert_ne!(
            native_session_import_lock_key("/projects/a", "bc", "d"),
            native_session_import_lock_key("/projects/ab", "c", "d")
        );
    }

    #[tokio::test]
    async fn imported_workspace_gets_the_shared_outbox_helper() {
        let directory = tempfile::tempdir().expect("temp workspace");
        ensure_outbox_helper(directory.path())
            .await
            .expect("helper");
        let helper = directory.path().join(".choruz/send");
        assert_eq!(std::fs::read_to_string(&helper).unwrap(), SEND_HELPER);
        assert!(directory.path().join(".choruz-outbox/new").is_dir());
        assert!(directory.path().join(".choruz-outbox/tmp").is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_ne!(
                std::fs::metadata(helper).unwrap().permissions().mode() & 0o111,
                0
            );
        }
    }
}
