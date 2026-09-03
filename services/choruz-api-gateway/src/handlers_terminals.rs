use std::{
    collections::HashSet,
    fs,
    path::{Path as StdPath, PathBuf},
    sync::Arc,
};

#[cfg(unix)]
use std::os::unix::fs::{PermissionsExt, symlink};

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
    headless::{HeadlessDriver, harness_account_env},
};
use choruz_common::AppError;
use choruz_domain::{ConversationType, Principal, PrincipalType};
use serde::Deserialize;
use serde_json::json;

use crate::{
    ApiError, ApiState, EnsureResult, PtyPool, PtySession, authenticated_principal,
    bearer_token_value, evict_stale_pty_sessions, handlers_runtime::accessible_workspace_ids,
};

// ── WebSocket terminal proxy ──────────────────────────────────────────

const MANAGED_CODEX_HOME_DIR: &str = "codex-homes";

#[derive(Debug, Clone)]
struct ManagedCodexHome {
    home_path: PathBuf,
    sessions_path: PathBuf,
}

fn choruz_runtime_dir() -> PathBuf {
    std::env::var("CHORUZ_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(".choruz-runtime"))
}

fn normal_codex_home() -> Option<PathBuf> {
    std::env::var("CODEX_HOME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(|home| PathBuf::from(home).join(".codex"))
        })
}

fn redact_local_path_error(context: &str, error: impl std::fmt::Display) -> AppError {
    AppError::Internal(format!("{context}: {error}"))
}

fn safe_binding_path_segment(binding_id: &str) -> Result<&str, AppError> {
    let valid = !binding_id.is_empty()
        && binding_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
    if !valid {
        return Err(AppError::Validation("invalid terminal binding id".into()));
    }
    Ok(binding_id)
}

#[cfg(unix)]
fn ensure_owner_only_dir(path: &StdPath) -> Result<(), AppError> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && metadata.file_type().is_symlink()
    {
        return Err(AppError::Internal(
            "managed Codex runtime path cannot be a symlink".into(),
        ));
    }
    fs::create_dir_all(path)
        .map_err(|error| redact_local_path_error("create runtime dir", error))?;
    let symlink_metadata = fs::symlink_metadata(path)
        .map_err(|error| redact_local_path_error("stat runtime dir", error))?;
    if symlink_metadata.file_type().is_symlink() || !symlink_metadata.is_dir() {
        return Err(AppError::Internal(
            "managed Codex runtime path must be a local directory".into(),
        ));
    }
    let mut permissions = fs::metadata(path)
        .map_err(|error| redact_local_path_error("stat runtime dir", error))?
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions)
        .map_err(|error| redact_local_path_error("chmod runtime dir", error))
}

#[cfg(unix)]
fn ensure_local_dir_not_symlink(path: &StdPath) -> Result<(), AppError> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && metadata.file_type().is_symlink()
    {
        return Err(AppError::Internal(
            "managed Codex sessions path cannot be a symlink".into(),
        ));
    }
    ensure_owner_only_dir(path)
}

#[cfg(unix)]
fn ensure_shared_codex_link(
    source_home: &StdPath,
    managed_home: &StdPath,
    name: &str,
) -> Result<(), AppError> {
    let source = source_home.join(name);
    if !source.exists() {
        return Ok(());
    }

    let target = managed_home.join(name);
    if let Ok(metadata) = fs::symlink_metadata(&target) {
        if !metadata.file_type().is_symlink() {
            return Err(AppError::Internal(
                "managed Codex home contains an unexpected local entry".into(),
            ));
        }
        let target_canonical = fs::canonicalize(&target)
            .map_err(|error| redact_local_path_error("validate Codex shared link", error))?;
        let source_canonical = fs::canonicalize(&source)
            .map_err(|error| redact_local_path_error("validate Codex shared source", error))?;
        if target_canonical != source_canonical {
            return Err(AppError::Internal(
                "managed Codex shared link points outside the intended source".into(),
            ));
        }
        return Ok(());
    }

    symlink(&source, &target)
        .map_err(|error| redact_local_path_error("link Codex shared state", error))
}

#[cfg(unix)]
fn provision_managed_codex_home(
    binding_id: &str,
    workspace_path: &str,
    source_home: Option<&StdPath>,
) -> Result<ManagedCodexHome, AppError> {
    let binding_segment = safe_binding_path_segment(binding_id)?;
    let runtime_dir = choruz_runtime_dir();
    ensure_owner_only_dir(&runtime_dir)?;
    let homes_root = runtime_dir.join(MANAGED_CODEX_HOME_DIR);
    ensure_owner_only_dir(&homes_root)?;

    let home_path = homes_root.join(binding_segment);
    ensure_owner_only_dir(&home_path)?;
    let sessions_path = home_path.join("sessions");
    ensure_local_dir_not_symlink(&sessions_path)?;

    if let Some(source_home) = source_home
        .map(StdPath::to_path_buf)
        .or_else(normal_codex_home)
        && source_home != home_path
    {
        for entry in ["config.toml", "auth.json", "plugins", "skills", "cache"] {
            ensure_shared_codex_link(&source_home, &home_path, entry)?;
        }
    }

    let canonical_home = fs::canonicalize(&home_path)
        .map_err(|error| redact_local_path_error("canonicalize Codex home", error))?;
    let canonical_sessions = fs::canonicalize(&sessions_path)
        .map_err(|error| redact_local_path_error("canonicalize Codex sessions", error))?;

    if !canonical_sessions.starts_with(&canonical_home) {
        return Err(AppError::Internal(
            "managed Codex sessions path escaped its home".into(),
        ));
    }

    if let Ok(workspace_canonical) = fs::canonicalize(workspace_path)
        && canonical_home.starts_with(workspace_canonical)
    {
        return Err(AppError::Internal(
            "managed Codex home must be outside the agent workspace".into(),
        ));
    }

    Ok(ManagedCodexHome {
        home_path: canonical_home,
        sessions_path: canonical_sessions,
    })
}

fn collect_codex_session_files(sessions_path: &StdPath) -> Result<HashSet<String>, AppError> {
    let canonical_sessions = fs::canonicalize(sessions_path)
        .map_err(|error| redact_local_path_error("canonicalize Codex sessions", error))?;
    let mut stack = vec![canonical_sessions.clone()];
    let mut files = HashSet::new();

    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(redact_local_path_error("read Codex sessions", error)),
        };
        for entry in entries {
            let entry = entry
                .map_err(|error| redact_local_path_error("read Codex session entry", error))?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| redact_local_path_error("stat Codex session entry", error))?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                stack.push(path);
                continue;
            }
            if !metadata.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("jsonl")
            {
                continue;
            }
            let canonical = fs::canonicalize(&path).map_err(|error| {
                redact_local_path_error("canonicalize Codex session file", error)
            })?;
            if canonical.starts_with(&canonical_sessions) {
                files.insert(canonical.to_string_lossy().to_string());
            }
        }
    }

    Ok(files)
}

#[derive(Debug, Clone)]
struct CodexSessionFileMeta {
    session_id: String,
    cwd: String,
    path: PathBuf,
}

fn read_codex_session_meta(path: &StdPath) -> Result<Option<CodexSessionFileMeta>, AppError> {
    let file = fs::File::open(path)
        .map_err(|error| redact_local_path_error("read Codex session", error))?;
    let reader = std::io::BufReader::new(file);
    for line in std::io::BufRead::lines(reader).take(20) {
        let line = line.map_err(|error| redact_local_path_error("read Codex session", error))?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if value.get("type").and_then(|v| v.as_str()) != Some("session_meta") {
            continue;
        }
        let payload = value.get("payload").unwrap_or(&value);
        let Some(session_id) = payload
            .get("id")
            .or_else(|| payload.get("session_id"))
            .and_then(|v| v.as_str())
            .filter(|v| !v.trim().is_empty())
        else {
            return Ok(None);
        };
        let Some(cwd) = payload
            .get("cwd")
            .or_else(|| payload.get("workspace_path"))
            .and_then(|v| v.as_str())
            .filter(|v| !v.trim().is_empty())
        else {
            return Ok(None);
        };
        return Ok(Some(CodexSessionFileMeta {
            session_id: session_id.to_string(),
            cwd: cwd.to_string(),
            path: path.to_path_buf(),
        }));
    }
    Ok(None)
}

/// Copy a user-selected native Codex session into the binding-owned session
/// store and return the terminal anchor that proves the copied history belongs
/// to this binding. Imports already identify one exact native session, unlike
/// the unsafe "newest session for this cwd" fallback used nowhere for Codex.
pub(crate) fn import_codex_terminal_session(
    binding_id: &str,
    conversation_id: &str,
    agent_principal_id: &str,
    company_id: &str,
    workspace_id: &str,
    workspace_path: &str,
    native_session_id: &str,
) -> Result<serde_json::Value, AppError> {
    let source_home = normal_codex_home().ok_or_else(|| {
        AppError::NotFound("Codex home is unavailable for the selected session".into())
    })?;
    let source_sessions = fs::canonicalize(source_home.join("sessions"))
        .map_err(|error| redact_local_path_error("canonicalize imported Codex sessions", error))?;
    let candidates = collect_codex_session_files(&source_sessions)?
        .into_iter()
        .filter_map(|path| {
            let path = PathBuf::from(path);
            match read_codex_session_meta(&path) {
                Ok(Some(meta))
                    if meta.session_id == native_session_id && meta.cwd == workspace_path =>
                {
                    Some(Ok(meta))
                }
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let [source] = candidates.as_slice() else {
        return Err(AppError::NotFound(
            "the selected Codex session has no unique matching native history file".into(),
        ));
    };

    let relative_path = source.path.strip_prefix(&source_sessions).map_err(|_| {
        AppError::Internal("selected Codex session escaped its native sessions directory".into())
    })?;
    let managed = provision_managed_codex_home(binding_id, workspace_path, Some(&source_home))?;
    let destination = managed.sessions_path.join(relative_path);
    let parent = destination.parent().ok_or_else(|| {
        AppError::Internal("selected Codex session has no parent directory".into())
    })?;
    ensure_local_dir_not_symlink(parent)?;
    fs::copy(&source.path, &destination)
        .map_err(|error| redact_local_path_error("copy imported Codex session", error))?;
    let destination = fs::canonicalize(&destination)
        .map_err(|error| redact_local_path_error("canonicalize imported Codex copy", error))?;
    if !destination.starts_with(&managed.sessions_path) {
        return Err(AppError::Internal(
            "imported Codex session escaped its binding-owned store".into(),
        ));
    }
    let copied = read_codex_session_meta(&destination)?
        .ok_or_else(|| AppError::Internal("copied Codex session lost its metadata".into()))?;
    if copied.session_id != native_session_id || copied.cwd != workspace_path {
        return Err(AppError::Internal(
            "copied Codex session no longer matches the selected session".into(),
        ));
    }

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
        "native_home_path": managed.home_path,
        "native_session_path": destination,
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

fn unique_codex_session_candidate(
    binding: &RuntimeBinding,
    capture: &CodexTerminalCaptureMetadata,
) -> Result<Option<CodexSessionFileMeta>, AppError> {
    if !codex_capture_metadata_matches_binding(binding, capture) {
        return Ok(None);
    }

    let home = fs::canonicalize(&capture.native_home_path)
        .map_err(|error| redact_local_path_error("canonicalize Codex home", error))?;
    let sessions = fs::canonicalize(&capture.sessions_path)
        .map_err(|error| redact_local_path_error("canonicalize Codex sessions", error))?;
    if !sessions.starts_with(&home) {
        return Ok(None);
    }

    let baseline: HashSet<&str> = capture
        .baseline_session_files
        .iter()
        .map(String::as_str)
        .collect();
    let mut candidates = Vec::new();
    for path in collect_codex_session_files(&sessions)? {
        if baseline.contains(path.as_str()) {
            continue;
        }
        let path_buf = PathBuf::from(path);
        let canonical = fs::canonicalize(&path_buf)
            .map_err(|error| redact_local_path_error("canonicalize Codex session", error))?;
        if !canonical.starts_with(&sessions) {
            continue;
        }
        let Some(meta) = read_codex_session_meta(&canonical)? else {
            continue;
        };
        if meta.cwd == capture.workspace_path {
            candidates.push(meta);
        }
    }

    match candidates.len() {
        0 => Ok(None),
        1 => Ok(candidates.pop()),
        _ => {
            tracing::warn!(
                binding_id = %binding.id,
                candidate_count = candidates.len(),
                "Codex terminal session attribution failed closed with multiple candidates"
            );
            Ok(None)
        }
    }
}

async fn reconcile_codex_terminal_session_from_capture(
    runtime: &RuntimeStore,
    binding: &RuntimeBinding,
) -> Result<Option<RuntimeBinding>, AppError> {
    let Some(capture) = binding.codex_terminal_capture_metadata() else {
        return Ok(None);
    };
    let Some(candidate) = unique_codex_session_candidate(binding, &capture)? else {
        return Ok(None);
    };
    let native_session_path = fs::canonicalize(&candidate.path)
        .map_err(|error| redact_local_path_error("canonicalize Codex session", error))?
        .to_string_lossy()
        .to_string();

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
                native_session_path,
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
    runtime: &RuntimeStore,
    binding: &mut RuntimeBinding,
    captured_session_id: &mut Option<String>,
    capture_disabled: &mut bool,
) {
    if captured_session_id.is_some() || *capture_disabled {
        return;
    }
    match reconcile_codex_terminal_session_from_capture(runtime, binding).await {
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

fn codex_terminal_args(resume_session_id: Option<&str>, model: Option<&str>) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(sid) = resume_session_id
        && !sid.is_empty()
    {
        args.push("resume".into());
        args.push(sid.into());
        args.push("--all".into());
    }
    args.extend([
        "--sandbox".into(),
        "workspace-write".into(),
        "--ask-for-approval".into(),
        "on-request".into(),
        // Suppress the blocking "Update now / Skip" startup prompt — a PTY
        // agent can never answer it and the terminal hangs until a human
        // presses enter. (There is no env var for this; verified against
        // codex 0.138.)
        "--config".into(),
        "check_for_update_on_startup=false".into(),
    ]);
    if let Some(model) = model {
        args.extend(["--model".into(), model.into()]);
    }
    args
}

fn terminal_cli_args(
    driver_type: &DriverType,
    resume_session_id: Option<&str>,
    model: Option<&str>,
) -> Vec<String> {
    let resume_session_id = resume_session_id.filter(|session_id| !session_id.is_empty());
    match driver_type {
        DriverType::ClaudeTerminal => {
            let mut args = vec!["--dangerously-skip-permissions".into()];
            if let Some(session_id) = resume_session_id {
                args.extend(["--resume".into(), session_id.into()]);
            }
            if let Some(model) = model {
                args.extend(["--model".into(), model.into()]);
            }
            args
        }
        DriverType::CodexTerminal => codex_terminal_args(resume_session_id, model),
        DriverType::PiTerminal => {
            let mut args = vec!["--approve".into()];
            if let Some(session_id) = resume_session_id {
                args.extend(["--session".into(), session_id.into()]);
            }
            if let Some(model) = model {
                args.extend(["--model".into(), model.into()]);
            }
            args
        }
        DriverType::GrokTerminal => {
            let mut args = vec!["--no-auto-update".into(), "--always-approve".into()];
            if let Some(session_id) = resume_session_id {
                args.extend(["--resume".into(), session_id.into()]);
            }
            if let Some(model) = model {
                args.extend(["--model".into(), model.into()]);
            }
            args
        }
        DriverType::OpenCodeTerminal => {
            let mut args = vec!["--auto".into()];
            if let Some(session_id) = resume_session_id {
                args.extend(["--session".into(), session_id.into()]);
            }
            if let Some(model) = model {
                args.extend(["--model".into(), model.into()]);
            }
            args
        }
        DriverType::MathCodeTerminal => Vec::new(),
        _ => Vec::new(),
    }
}

async fn terminal_resume_session_id(
    runtime: &RuntimeStore,
    binding: &RuntimeBinding,
    binding_id: &str,
    context: &str,
) -> Option<String> {
    let is_codex = binding.driver_type == DriverType::CodexTerminal;

    if is_codex {
        let _ = binding_id;
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

    match binding.external_session_id.clone() {
        Some(s) if !s.is_empty() => Some(s),
        _ => match runtime.sync_session_id_from_disk(binding_id).await {
            Ok(Some(sid)) => {
                tracing::info!(binding_id = %binding_id, session_id = %sid, context, "PTY: backfilled session_id from disk");
                Some(sid)
            }
            Ok(None) => None,
            Err(e) => {
                tracing::warn!(binding_id = %binding_id, error = %e, context, "PTY: session_id disk backfill failed");
                None
            }
        },
    }
}

fn codex_resume_session_id_from_anchor(
    binding: &RuntimeBinding,
    managed_home: &ManagedCodexHome,
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
    let session_path = fs::canonicalize(&anchor.native_session_path).ok()?;
    if !session_path.starts_with(&managed_home.sessions_path) {
        return None;
    }
    let meta = read_codex_session_meta(&session_path).ok()??;
    if meta.session_id != anchor.session_id || meta.cwd != binding.workspace_path {
        return None;
    }
    Some(anchor.session_id)
}

fn live_pty_session_exists(pool: &PtyPool, binding_id: &str) -> bool {
    let sessions = pool.lock().expect("pty pool lock");
    sessions
        .get(binding_id)
        .is_some_and(|session| session.is_child_alive())
}

fn take_startup_replay(
    startup_replay: &Arc<std::sync::Mutex<Option<Vec<Vec<u8>>>>>,
) -> Vec<Vec<u8>> {
    startup_replay
        .lock()
        .expect("startup replay lock")
        .take()
        .unwrap_or_default()
}

async fn prepare_codex_spawn_if_needed(
    runtime: &RuntimeStore,
    pool: &PtyPool,
    binding: RuntimeBinding,
    binding_id: &str,
) -> Result<(RuntimeBinding, Option<String>, Option<String>), AppError> {
    if binding.driver_type != DriverType::CodexTerminal || live_pty_session_exists(pool, binding_id)
    {
        return Ok((binding, None, None));
    }

    let account_home = harness_account_env(HeadlessDriver::Codex, &binding.config_json)
        .map_err(AppError::Validation)?
        .map(|(_, path)| path);
    let managed =
        provision_managed_codex_home(binding_id, &binding.workspace_path, account_home.as_deref())?;
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
        None => match reconcile_codex_terminal_session_from_capture(runtime, &binding).await {
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
        },
    };

    let resume_session_id =
        codex_resume_session_id_from_anchor(&binding, &managed, &workspace_id, &company_id);
    let baseline = collect_codex_session_files(&managed.sessions_path)?
        .into_iter()
        .collect();
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
                baseline_session_files: baseline,
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
    runtime: &RuntimeStore,
    binding_id: &str,
    fallback_binding: RuntimeBinding,
) -> Result<Option<RuntimeBinding>, AppError> {
    let latest_binding = runtime
        .get_binding(binding_id)
        .await
        .unwrap_or(fallback_binding);
    reconcile_codex_terminal_session_from_capture(runtime, &latest_binding).await
}

pub(crate) fn is_terminal_driver(driver_type: &DriverType) -> bool {
    matches!(
        driver_type,
        DriverType::ClaudeTerminal
            | DriverType::CodexTerminal
            | DriverType::PiTerminal
            | DriverType::GrokTerminal
            | DriverType::OpenCodeTerminal
            | DriverType::MathCodeTerminal
    )
}

fn default_terminal_binary(driver_type: &DriverType) -> &'static str {
    match driver_type {
        DriverType::ClaudeTerminal => "claude",
        DriverType::CodexTerminal => "codex",
        DriverType::PiTerminal => "pi",
        DriverType::GrokTerminal => "grok",
        DriverType::OpenCodeTerminal => "opencode",
        DriverType::MathCodeTerminal => "mathcode",
        _ => "claude",
    }
}

fn terminal_binary(binding: &RuntimeBinding) -> String {
    let configured = binding
        .config_json
        .get("binary_path")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default_terminal_binary(&binding.driver_type));

    // `binary_path: "codex"` is the portable default persisted by older
    // bindings. Prefer an explicitly configured harness executable over that
    // bare default: PATH can otherwise select a stale CLI that cannot parse
    // the user's current session/configuration. Absolute or custom per-agent
    // paths remain authoritative.
    let environment_key = match binding.driver_type {
        DriverType::ClaudeTerminal => Some("CHORUZ_CLAUDE_BINARY"),
        DriverType::CodexTerminal => Some("CHORUZ_CODEX_BINARY"),
        DriverType::PiTerminal => Some("CHORUZ_PI_BINARY"),
        DriverType::GrokTerminal => Some("CHORUZ_GROK_BINARY"),
        DriverType::OpenCodeTerminal => Some("CHORUZ_OPENCODE_BINARY"),
        DriverType::MathCodeTerminal => Some("CHORUZ_MATHCODE_BINARY"),
        _ => None,
    };
    if configured == default_terminal_binary(&binding.driver_type)
        && let Some(path) = environment_key
            .and_then(|key| std::env::var(key).ok())
            .filter(|value| !value.trim().is_empty())
    {
        return path;
    }
    configured.to_string()
}

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

#[allow(clippy::too_many_arguments)]
pub(crate) fn ensure_pty_session(
    pool: &PtyPool,
    binding_id: &str,
    driver_type: &DriverType,
    binary_path: &str,
    workspace_path: &str,
    cols: u16,
    rows: u16,
    resume_session_id: Option<&str>,
    codex_home_env: Option<&str>,
    model: Option<&str>,
    binding_config: &serde_json::Value,
) -> Result<EnsureResult, AppError> {
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};

    // Evict stale sessions before checking/creating
    evict_stale_pty_sessions(pool);

    let mut sessions = pool.lock().expect("pty pool lock");
    if let Some(session) = sessions.get(binding_id) {
        if session.is_child_alive() {
            session.touch();
            return Ok(EnsureResult {
                session: Arc::clone(session),
                newly_created: false,
            });
        }
        // Child process is dead — remove stale session and recreate
        tracing::warn!(binding_id, "PTY child process exited, recreating session");
        sessions.remove(binding_id);
    }

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| AppError::Internal(format!("pty open: {e}")))?;

    let is_codex = *driver_type == DriverType::CodexTerminal;
    let mut cmd = CommandBuilder::new(binary_path);
    cmd.cwd(workspace_path);
    // Interactive PTYs use the same absolute workspace-bound command surface
    // as headless executions. A capable terminal type also prevents modern
    // harnesses from pausing on a TERM=dumb confirmation prompt.
    cmd.env("TERM", "xterm-256color");
    cmd.env(
        "CHORUZ_SEND",
        PathBuf::from(workspace_path).join(".choruz").join("send"),
    );
    // Suppress auto-update behavior in spawned CLI processes. Each CLI has
    // its OWN mechanism (verified against the installed binaries — the
    // previous CODEX_/CLAUDE_/GEMINI_DISABLE_UPDATE_CHECK env vars were
    // recognized by none of them):
    // - Claude Code: DISABLE_AUTOUPDATER=1 (official env var).
    // - Codex: a CLI flag, `--config check_for_update_on_startup=false`,
    //   appended in codex_terminal_args. Without it, a pending codex
    //   release shows a blocking "Update now / Skip" prompt on startup
    //   that the PTY agent can never answer — the terminal hangs there.
    cmd.env("DISABLE_AUTOUPDATER", "1");
    cmd.env("PI_SKIP_VERSION_CHECK", "1");
    if let Some(codex_home) = codex_home_env
        && is_codex
    {
        cmd.env("CODEX_HOME", codex_home);
    }
    if !is_codex
        && let Some(driver) = HeadlessDriver::from_driver_type(driver_type.as_str())
        && let Some((key, value)) =
            harness_account_env(driver, binding_config).map_err(AppError::Validation)?
    {
        cmd.env(key, value);
    }

    if let Some(session_id) = resume_session_id.filter(|session_id| !session_id.is_empty()) {
        tracing::info!(
            binding_id,
            session_id,
            driver_type = driver_type.as_str(),
            "resuming CLI session in PTY"
        );
    }
    for arg in terminal_cli_args(driver_type, resume_session_id, model) {
        cmd.arg(arg);
    }

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| AppError::Internal(format!("pty spawn: {e}")))?;
    drop(pair.slave);

    // Create a process container that wraps the child's entire process tree.
    // portable-pty calls setsid() before exec, so child_pid == session leader
    // == initial PGID.  The container uses this to kill all descendants on
    // cleanup, not just the direct child.
    let child_pid = child.process_id().ok_or_else(|| {
        AppError::Internal("spawned child has no PID — cannot create process container".into())
    })?;
    let container =
        crate::pty_manager::ProcessContainer::new(format!("pty-{binding_id}"), child_pid);
    tracing::info!(
        binding_id,
        child_pid,
        "PTY child spawned and placed in process container"
    );

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| AppError::Internal(format!("pty reader: {e}")))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| AppError::Internal(format!("pty writer: {e}")))?;

    let (output_tx, _) = tokio::sync::broadcast::channel(4096);
    let startup_replay: Arc<std::sync::Mutex<Option<Vec<Vec<u8>>>>> =
        Arc::new(std::sync::Mutex::new(Some(Vec::new())));
    let tx_clone = output_tx.clone();
    let replay_clone = Arc::clone(&startup_replay);

    // Background thread: read PTY output and broadcast
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut reader = reader;
        loop {
            match std::io::Read::read(&mut reader, &mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx_clone.receiver_count() == 0 {
                        let mut replay = replay_clone.lock().expect("startup replay lock");
                        if let Some(frames) = replay.as_mut() {
                            let current: usize = frames.iter().map(Vec::len).sum();
                            if current < 64 * 1024 {
                                let remaining = (64 * 1024) - current;
                                frames.push(buf[..n.min(remaining)].to_vec());
                            }
                        }
                    }
                    // Err = no active subscribers; discard data but keep reading
                    // so the child process doesn't block on a full PTY buffer.
                    let _ = tx_clone.send(buf[..n].to_vec());
                }
                Err(_) => break,
            }
        }
    });

    let session = Arc::new(PtySession {
        writer: Arc::new(std::sync::Mutex::new(writer)),
        output_tx,
        startup_replay,
        child: std::sync::Mutex::new(child),
        master: Arc::new(std::sync::Mutex::new(pair.master)),
        last_accessed: std::sync::Mutex::new(std::time::Instant::now()),
        _container: container,
    });

    sessions.insert(binding_id.to_string(), Arc::clone(&session));
    Ok(EnsureResult {
        session,
        newly_created: true,
    })
}

pub(crate) async fn websocket_terminal(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(binding_id): Path<String>,
    Query(query): Query<TerminalQuery>,
) -> Result<impl IntoResponse, ApiError> {
    // Auth: try Bearer header first, then query token
    let auth_headers = if query.token.is_some() && bearer_token_value(&headers).is_none() {
        // Synthesize an Authorization header from the query param so we can
        // reuse the existing authenticate() machinery.
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

    let request_started = std::time::Instant::now();
    let principal = authenticated_principal(&auth_headers, &state).await?;
    let binding = authorize_terminal_binding(&state, &principal, &binding_id).await?;
    let authorized_ms = request_started.elapsed().as_millis() as u64;

    let binary_path = terminal_binary(&binding);
    let workspace_path = binding.workspace_path.clone();

    let cols = query.cols.unwrap_or(120);
    let rows = query.rows.unwrap_or(40);

    let pool = state.pty_pool.clone();
    let runtime = state.runtime.clone();
    let resume_lookup_started = std::time::Instant::now();
    let resume_session_id = if binding.driver_type == DriverType::CodexTerminal {
        None
    } else {
        terminal_resume_session_id(&runtime, &binding, &binding_id, "ws_connect").await
    };
    // Timing of everything that delays the upgrade itself; the browser sees
    // "Connecting to terminal..." until this returns.
    tracing::info!(
        binding_id = %binding_id,
        driver_type = binding.driver_type.as_str(),
        authorized_ms,
        resume_lookup_ms = resume_lookup_started.elapsed().as_millis() as u64,
        resume_session = resume_session_id.is_some(),
        "PTY: terminal websocket accepted"
    );

    Ok(ws.on_upgrade(move |socket| {
        terminal_bridge_pty(
            socket,
            pool,
            runtime,
            binding,
            binding_id,
            binary_path,
            workspace_path,
            cols,
            rows,
            resume_session_id,
        )
    }))
}

/// Bridge a WebSocket to a persistent PTY session from the pool.
#[allow(clippy::too_many_arguments)]
async fn terminal_bridge_pty(
    socket: WebSocket,
    pool: PtyPool,
    runtime: choruz_agent_runtime::RuntimeStore,
    binding: RuntimeBinding,
    binding_id: String,
    binary_path: String,
    workspace_path: String,
    cols: u16,
    rows: u16,
    resume_session_id: Option<String>,
) {
    use futures_util::{SinkExt, StreamExt};
    use portable_pty::PtySize;

    let bridge_started = std::time::Instant::now();
    let (binding, codex_resume_session_id, codex_home_env) = match prepare_codex_spawn_if_needed(
        &runtime,
        &pool,
        binding,
        &binding_id,
    )
    .await
    {
        Ok(prepared) => prepared,
        Err(error) => {
            tracing::error!(binding_id = %binding_id, error = %error, "failed to prepare Codex terminal spawn");
            return;
        }
    };
    let effective_resume_session_id = if codex_home_env.is_some() {
        codex_resume_session_id
    } else {
        resume_session_id
    };

    // Ensure PTY exists
    let result = match ensure_pty_session(
        &pool,
        &binding_id,
        &binding.driver_type,
        &binary_path,
        &workspace_path,
        cols,
        rows,
        effective_resume_session_id.as_deref(),
        codex_home_env.as_deref(),
        binding
            .config_json
            .get("model")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        &binding.config_json,
    ) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "failed to ensure pty session");
            return;
        }
    };
    let session = result.session;

    let (mut ws_sender, mut ws_receiver) = socket.split();
    let mut output_rx = session.output_tx.subscribe();
    let startup_frames = take_startup_replay(&session.startup_replay);
    tracing::info!(
        binding_id = %binding_id,
        driver_type = binding.driver_type.as_str(),
        newly_created = result.newly_created,
        spawn_ms = bridge_started.elapsed().as_millis() as u64,
        replay_frames = startup_frames.len(),
        "PTY: session ready for websocket"
    );
    let binding_id_for_first_output = binding_id.clone();

    // Task A: PTY broadcast output -> WebSocket
    //
    // We can't rely on `output_rx.recv()` returning Err to detect PTY death,
    // because the session struct still holds a live `output_tx` Sender even
    // after the PTY reader thread exits. Without the explicit liveness probe
    // below, a crashed CLI (e.g. codex exits instantly on a bad session
    // resume) leaves the WebSocket half-open — the client just sees a black
    // terminal forever because no Close frame is ever emitted. Poll
    // `is_child_alive` on the side and break out so the client can reconnect.
    let session_for_liveness = Arc::clone(&session);
    let runtime_for_capture = runtime.clone();
    let binding_for_capture = binding.clone();
    let is_codex_terminal = binding.driver_type == choruz_agent_runtime::DriverType::CodexTerminal;
    let is_codex_terminal_for_cleanup = is_codex_terminal;
    let binding_for_cleanup_capture = binding.clone();
    let runtime_for_cleanup_capture = runtime.clone();
    let send_task = tokio::spawn(async move {
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
        let mut binding_for_capture = binding_for_capture;
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
        for data in startup_frames {
            if is_codex_terminal {
                try_capture_codex_terminal_session(
                    &runtime_for_capture,
                    &mut binding_for_capture,
                    &mut captured_terminal_session_id,
                    &mut terminal_capture_disabled,
                )
                .await;
            }
            log_first_output(data.len());
            if ws_sender
                .send(WsMessage::Binary(data.into()))
                .await
                .is_err()
            {
                return;
            }
        }
        loop {
            tokio::select! {
                msg = output_rx.recv() => match msg {
                    Ok(data) => {
                        if is_codex_terminal {
                            try_capture_codex_terminal_session(
                                &runtime_for_capture,
                                &mut binding_for_capture,
                                &mut captured_terminal_session_id,
                                &mut terminal_capture_disabled,
                            )
                            .await;
                        }
                        log_first_output(data.len());
                        if ws_sender
                            .send(WsMessage::Binary(data.into()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(skipped = n, "PTY output consumer lagged, skipping frames");
                        continue;
                    }
                    Err(_) => break, // channel closed
                },
                _ = tokio::time::sleep(std::time::Duration::from_millis(750)) => {
                    if !session_for_liveness.is_child_alive() {
                        tracing::info!(
                            "PTY child exited; closing WebSocket so client can reconnect"
                        );
                        let _ = ws_sender.send(WsMessage::Close(None)).await;
                        break;
                    }
                }
            }
        }
    });

    // Task B: WebSocket -> PTY stdin + resize
    let writer = Arc::clone(&session.writer);
    let master = Arc::clone(&session.master);
    let binding_id_for_cleanup = binding_id.clone();
    let recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_receiver.next().await {
            match msg {
                WsMessage::Text(text) => {
                    // Check for JSON resize command: {"type":"resize","cols":N,"rows":N}
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text)
                        && parsed.get("type").and_then(|v| v.as_str()) == Some("resize")
                    {
                        let new_cols =
                            parsed.get("cols").and_then(|v| v.as_u64()).unwrap_or(120) as u16;
                        let new_rows =
                            parsed.get("rows").and_then(|v| v.as_u64()).unwrap_or(40) as u16;
                        if let Err(e) = master.lock().expect("master lock").resize(PtySize {
                            rows: new_rows,
                            cols: new_cols,
                            pixel_width: 0,
                            pixel_height: 0,
                        }) {
                            tracing::warn!(binding_id = %binding_id, error = %e, "PTY resize failed");
                        }
                        continue;
                    }
                    // Regular text -> write to PTY
                    if let Err(e) = writer
                        .lock()
                        .expect("writer lock")
                        .write_all(text.as_bytes())
                    {
                        tracing::warn!(binding_id = %binding_id, error = %e, "PTY stdin write failed");
                        break;
                    }
                }
                WsMessage::Binary(data) => {
                    if let Err(e) = writer.lock().expect("writer lock").write_all(&data) {
                        tracing::warn!(binding_id = %binding_id, error = %e, "PTY binary write failed");
                        break;
                    }
                }
                WsMessage::Close(_) => break,
                _ => {}
            }
        }
    });

    // Wait for any leg to finish, then let the others drop.
    tokio::select! {
        _ = send_task => {}
        _ = recv_task => {}
    }

    if is_codex_terminal_for_cleanup {
        match capture_codex_terminal_before_cleanup(
            &runtime_for_cleanup_capture,
            &binding_id_for_cleanup,
            binding_for_cleanup_capture,
        )
        .await
        {
            Ok(Some(_)) => tracing::info!(
                binding_id = %binding_id_for_cleanup,
                "captured Codex terminal session before PTY cleanup"
            ),
            Ok(None) => {}
            Err(error) => tracing::warn!(
                binding_id = %binding_id_for_cleanup,
                error = %error,
                "Codex terminal session capture before PTY cleanup failed"
            ),
        }
    } else {
        // Persist the active CLI's exact-workspace session before killing the
        // PTY. This prevents a reconnect from falling back to another CLI's
        // store and gives the CLI a chance to expose its current session while
        // its process context is still intact.
        match runtime_for_cleanup_capture
            .sync_session_id_from_disk(&binding_id_for_cleanup)
            .await
        {
            Ok(Some(sid)) => tracing::info!(
                binding_id = %binding_id_for_cleanup,
                session_id = %sid,
                "PTY: captured session_id before disconnect cleanup"
            ),
            Ok(None) => {}
            Err(error) => tracing::warn!(
                binding_id = %binding_id_for_cleanup,
                error = %error,
                "PTY: pre-cleanup session_id capture failed"
            ),
        }
    }

    // WS disconnected → kill PTY so next connection does a fresh --resume.
    // This ensures the user always sees the full CLI on reconnect.
    // Session continuity is maintained via --resume SESSION_ID.
    {
        let mut sessions = pool.lock().expect("pty pool lock");
        if let Some(removed) = sessions.remove(&binding_id_for_cleanup) {
            tracing::info!(binding_id = %binding_id_for_cleanup, "PTY session removed on WS disconnect (will --resume on next connect)");
            drop(removed); // ProcessContainer kills the process tree
        }
    }
}

// ── REST terminal endpoints ───────────────────────────────────────────

pub(crate) async fn ensure_terminal(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(binding_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    let binding = authorize_terminal_binding(&state, &principal, &binding_id).await?;
    let binary = terminal_binary(&binding);

    let session_id_owned = if binding.driver_type == DriverType::CodexTerminal {
        None
    } else {
        terminal_resume_session_id(&state.runtime, &binding, &binding_id, "ensure_terminal").await
    };
    let (binding, codex_resume_session_id, codex_home_env) =
        prepare_codex_spawn_if_needed(&state.runtime, &state.pty_pool, binding, &binding_id)
            .await
            .map_err(ApiError::from)?;
    let effective_session_id = if codex_home_env.is_some() {
        codex_resume_session_id
    } else {
        session_id_owned
    };
    let session_id = effective_session_id.as_deref();
    let result = ensure_pty_session(
        &state.pty_pool,
        &binding_id,
        &binding.driver_type,
        &binary,
        &binding.workspace_path,
        120,
        40,
        session_id,
        codex_home_env.as_deref(),
        binding
            .config_json
            .get("model")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        &binding.config_json,
    )
    .map_err(ApiError::from)?;

    Ok(Json(json!({
        "status": "ok",
        "binding_id": binding_id,
        "newly_created": result.newly_created,
        "resumed_session": session_id.is_some() && result.newly_created,
    })))
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
    let _binding = authorize_terminal_binding(&state, &principal, &binding_id).await?;

    let pool = state.pty_pool.lock().expect("pool lock");
    let session = pool
        .get(&binding_id)
        .ok_or_else(|| ApiError::from(AppError::Validation("terminal not running".into())))?;
    session.touch();

    let mut writer = session.writer.lock().expect("writer lock");

    // Use bracketed paste mode for multi-line input so that CLI applications
    // (Claude Code, Codex) treat the entire block as a single paste event
    // instead of processing each newline as a separate Enter keypress.
    let needs_bracketed_paste = payload.data.contains('\n');
    if needs_bracketed_paste {
        writer
            .write_all(b"\x1b[200~")
            .map_err(|e| ApiError::from(AppError::Internal(format!("pty write: {e}"))))?;
    }

    writer
        .write_all(payload.data.as_bytes())
        .map_err(|e| ApiError::from(AppError::Internal(format!("pty write: {e}"))))?;

    if needs_bracketed_paste {
        writer
            .write_all(b"\x1b[201~")
            .map_err(|e| ApiError::from(AppError::Internal(format!("pty write: {e}"))))?;
    }

    // Flush the text content so the TUI app processes it before receiving
    // the Enter keypress. Without this flush, Codex TUI may treat the text
    // and Enter as a single paste event and not submit the input.
    writer
        .flush()
        .map_err(|e| ApiError::from(AppError::Internal(format!("pty flush: {e}"))))?;

    // Brief pause to let the TUI process the text before the Enter keypress.
    // Codex TUI (ink-based React renderer) needs a tick to process pasted
    // input before it can recognize Enter as a submission event.
    drop(writer);
    std::thread::sleep(std::time::Duration::from_millis(50));
    let mut writer = session.writer.lock().expect("writer lock");

    // Send \r (carriage return) to submit — terminals use \r for Enter,
    // NOT \n (line-feed) which doesn't trigger submission in TUI apps.
    writer
        .write_all(b"\r")
        .map_err(|e| ApiError::from(AppError::Internal(format!("pty write: {e}"))))?;

    Ok(Json(json!({"status": "ok"})))
}

use std::io::Write;

#[cfg(test)]
mod tests {
    use super::{
        ManagedCodexHome, codex_resume_session_id_from_anchor, codex_session_provenance_matches,
        codex_terminal_args, collect_codex_session_files, default_terminal_binary,
        ensure_pty_session, import_codex_terminal_session, provision_managed_codex_home,
        take_startup_replay, terminal_capture_error_is_permanent, terminal_cli_args,
        terminal_resume_session_id, unique_codex_session_candidate,
    };
    use choruz_agent_runtime::{
        BindingState, CodexTerminalCaptureMetadata, DriverType, RuntimeBinding, RuntimeStore,
    };
    use chrono::Utc;
    use serde_json::json;
    use std::fs;

    #[test]
    fn terminal_capture_stops_retrying_when_binding_context_is_gone() {
        assert!(terminal_capture_error_is_permanent(
            &choruz_common::AppError::NotFound("runtime binding context changed".into())
        ));
        assert!(!terminal_capture_error_is_permanent(
            &choruz_common::AppError::Internal("temporary database failure".into())
        ));
    }

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn isolated_test_dir(name: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("choruz-{name}-{unique}"));
        fs::create_dir_all(&dir).expect("create isolated test dir");
        dir
    }

    fn with_env_lock<T>(f: impl FnOnce() -> T) -> T {
        let _guard = crate::test_support::api_test_env_lock().blocking_lock();
        f()
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

        fn remove(key: &'static str) -> Self {
            let guard = Self {
                key,
                previous: std::env::var_os(key),
            };
            unsafe {
                std::env::remove_var(key);
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

    fn capture_metadata_for(
        binding: &RuntimeBinding,
        managed: &ManagedCodexHome,
        baseline: Vec<String>,
    ) -> CodexTerminalCaptureMetadata {
        CodexTerminalCaptureMetadata {
            binding_id: binding.id.clone(),
            conversation_id: binding.conversation_id.clone(),
            agent_principal_id: binding.agent_principal_id.clone(),
            company_id: "company-1".into(),
            driver_type: binding.driver_type.as_str().into(),
            workspace_id: "workspace-1".into(),
            workspace_path: binding.workspace_path.clone(),
            native_home_path: managed.home_path.to_string_lossy().to_string(),
            sessions_path: managed.sessions_path.to_string_lossy().to_string(),
            binding_generation: binding.terminal_generation(),
            spawn_started_at: "2026-05-29T00:00:00Z".into(),
            baseline_session_files: baseline,
        }
    }

    #[test]
    fn startup_replay_buffer_drains_once() {
        let replay = std::sync::Arc::new(std::sync::Mutex::new(Some(vec![
            b"first frame".to_vec(),
            b"second frame".to_vec(),
        ])));

        let first = take_startup_replay(&replay);
        let second = take_startup_replay(&replay);

        assert_eq!(first.len(), 2);
        assert_eq!(first[0], b"first frame");
        assert!(
            second.is_empty(),
            "startup replay must not duplicate frames"
        );
        assert!(replay.lock().expect("replay lock").is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fake_cli_pty_round_trips_input_output_and_exit_code() {
        let temp = tempfile::tempdir().expect("create fake PTY dir");
        let dir = temp.path();
        let codex_home = dir.join("managed-codex-home");
        fs::create_dir_all(&codex_home).expect("create managed Codex home");
        let cli = dir.join("fake-cli.sh");
        fs::write(
            &cli,
            "#!/bin/sh\nprintf 'READY\\r\\n'\nprintf 'ARGS:%s\\r\\n' \"$*\"\nprintf 'CODEX_HOME:%s\\r\\n' \"${CODEX_HOME:-}\"\nprintf 'TERM:%s\\r\\n' \"${TERM:-}\"\nprintf 'CHORUZ_SEND:%s\\r\\n' \"${CHORUZ_SEND:-}\"\nIFS= read -r line\nprintf 'ECHO:%s\\r\\n' \"$line\"\nexit 23\n",
        )
        .expect("write fake PTY cli");
        let mut permissions = fs::metadata(&cli).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&cli, permissions).unwrap();

        let original_codex_home = std::env::var_os("CODEX_HOME");
        let cases = [
            (
                "claude",
                DriverType::ClaudeTerminal,
                "claude-session",
                None,
                "ARGS:--dangerously-skip-permissions --resume claude-session",
            ),
            (
                "codex",
                DriverType::CodexTerminal,
                "codex-session",
                Some(codex_home.to_str().unwrap()),
                "ARGS:resume codex-session --all --sandbox workspace-write --ask-for-approval on-request --config check_for_update_on_startup=false",
            ),
            (
                "pi",
                DriverType::PiTerminal,
                "pi-session",
                None,
                "ARGS:--approve --session pi-session",
            ),
            (
                "grok",
                DriverType::GrokTerminal,
                "grok-session",
                None,
                "ARGS:--no-auto-update --always-approve --resume grok-session",
            ),
            (
                "opencode",
                DriverType::OpenCodeTerminal,
                "opencode-session",
                None,
                "ARGS:--auto --session opencode-session",
            ),
            (
                "mathcode",
                DriverType::MathCodeTerminal,
                "mathcode-session",
                None,
                "ARGS:",
            ),
        ];

        for (label, driver, resume_session_id, codex_home_env, expected_args) in cases {
            let pool: crate::PtyPool =
                std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
            let ensured = ensure_pty_session(
                &pool,
                &format!("binding-fake-pty-{label}"),
                &driver,
                cli.to_str().unwrap(),
                dir.to_str().unwrap(),
                100,
                30,
                Some(resume_session_id),
                codex_home_env,
                None,
                &json!({}),
            )
            .expect("spawn fake CLI in PTY");
            assert!(ensured.newly_created);

            let mut output = ensured.session.output_tx.subscribe();
            {
                let mut writer = ensured.session.writer.lock().expect("PTY writer lock");
                std::io::Write::write_all(&mut *writer, b"hello from test\n").unwrap();
                std::io::Write::flush(&mut *writer).unwrap();
            }

            let mut transcript = take_startup_replay(&ensured.session.startup_replay)
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            tokio::time::timeout(std::time::Duration::from_secs(3), async {
                while !String::from_utf8_lossy(&transcript).contains("ECHO:hello from test") {
                    transcript.extend(output.recv().await.expect("PTY output frame"));
                }
            })
            .await
            .expect("fake CLI should echo PTY input");
            let transcript = String::from_utf8_lossy(&transcript);
            assert!(
                transcript.contains("READY"),
                "{label} startup output must reach the PTY client"
            );
            assert!(
                transcript.contains(expected_args),
                "{label} must receive its driver-specific session arguments: {transcript}"
            );
            assert!(
                transcript.contains("TERM:xterm-256color"),
                "{label} must receive a capable terminal type: {transcript}"
            );
            assert!(
                transcript.contains(&format!("CHORUZ_SEND:{}/.choruz/send", dir.display())),
                "{label} must receive its absolute Choruz helper binding: {transcript}"
            );
            if driver == DriverType::CodexTerminal {
                assert!(
                    transcript.contains(&format!("CODEX_HOME:{}", codex_home.display())),
                    "Codex child must receive its managed home: {transcript}"
                );
            }

            let exit_code = tokio::time::timeout(std::time::Duration::from_secs(3), async {
                loop {
                    let status = ensured
                        .session
                        .child
                        .lock()
                        .expect("PTY child lock")
                        .try_wait()
                        .expect("query fake CLI status");
                    if let Some(status) = status {
                        break status.exit_code();
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("fake CLI should exit");
            assert_eq!(exit_code, 23);
            assert_eq!(std::env::var_os("CODEX_HOME"), original_codex_home);

            pool.lock().unwrap().clear();
        }
    }

    #[test]
    fn codex_terminal_args_use_current_cli_flags() {
        let args = codex_terminal_args(Some("session-123"), None);

        assert_eq!(
            args,
            vec![
                "resume",
                "session-123",
                "--all",
                "--sandbox",
                "workspace-write",
                "--ask-for-approval",
                "on-request",
                "--config",
                "check_for_update_on_startup=false",
            ]
        );
        assert!(!args.iter().any(|arg| arg == "--full-auto"));
    }

    #[test]
    fn codex_terminal_args_start_new_session_without_resume() {
        let args = codex_terminal_args(None, None);

        assert_eq!(
            args,
            vec![
                "--sandbox",
                "workspace-write",
                "--ask-for-approval",
                "on-request",
                "--config",
                "check_for_update_on_startup=false",
            ]
        );
        assert!(!args.iter().any(|arg| arg == "--full-auto"));
    }

    #[test]
    fn additional_terminal_drivers_use_documented_flags_and_exact_resume_ids() {
        assert_eq!(
            terminal_cli_args(&DriverType::PiTerminal, Some("pi-session"), None),
            vec!["--approve", "--session", "pi-session"]
        );
        assert_eq!(
            terminal_cli_args(&DriverType::GrokTerminal, Some("grok-session"), None),
            vec![
                "--no-auto-update",
                "--always-approve",
                "--resume",
                "grok-session"
            ]
        );
        assert_eq!(
            terminal_cli_args(&DriverType::OpenCodeTerminal, Some("oc-session"), None),
            vec!["--auto", "--session", "oc-session"]
        );
        assert!(
            terminal_cli_args(&DriverType::MathCodeTerminal, Some("math-session"), None).is_empty()
        );
    }

    #[test]
    fn selected_model_is_forwarded_to_every_interactive_harness() {
        let cases = [
            (DriverType::ClaudeTerminal, "claude-opus-5"),
            (DriverType::CodexTerminal, "gpt-5.6-codex"),
            (DriverType::PiTerminal, "anthropic/claude-sonnet-5"),
            (DriverType::GrokTerminal, "grok-4.6"),
            (
                DriverType::OpenCodeTerminal,
                "openrouter/anthropic/claude-sonnet-5",
            ),
        ];
        for (driver, model) in cases {
            let args = terminal_cli_args(&driver, Some("session-id"), Some(model));
            let model_flag = args
                .iter()
                .position(|arg| arg == "--model")
                .expect("model flag");
            assert_eq!(args.get(model_flag + 1).map(String::as_str), Some(model));
        }
    }

    #[test]
    fn terminal_driver_defaults_do_not_fall_back_to_claude() {
        assert_eq!(
            default_terminal_binary(&DriverType::ClaudeTerminal),
            "claude"
        );
        assert_eq!(default_terminal_binary(&DriverType::CodexTerminal), "codex");
        assert_eq!(default_terminal_binary(&DriverType::PiTerminal), "pi");
        assert_eq!(default_terminal_binary(&DriverType::GrokTerminal), "grok");
        assert_eq!(
            default_terminal_binary(&DriverType::OpenCodeTerminal),
            "opencode"
        );
        assert_eq!(
            default_terminal_binary(&DriverType::MathCodeTerminal),
            "mathcode"
        );
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
            terminal_resume_session_id(&runtime, &binding, "binding-1", "test")
                .await
                .as_deref(),
            None
        );
        assert_eq!(
            codex_terminal_args(None, None),
            vec![
                "--sandbox",
                "workspace-write",
                "--ask-for-approval",
                "on-request",
                "--config",
                "check_for_update_on_startup=false",
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn managed_codex_home_uses_local_sessions_and_shared_safe_links() {
        with_env_lock(|| {
            let root = isolated_test_dir("codex-managed-home");
            let runtime_dir = root.join("runtime");
            let normal_home = root.join("normal-codex");
            fs::create_dir_all(normal_home.join("plugins")).expect("create plugins");
            fs::create_dir_all(normal_home.join("skills")).expect("create skills");
            fs::create_dir_all(normal_home.join("cache")).expect("create cache");
            fs::write(normal_home.join("config.toml"), "model = \"gpt-5\"\n")
                .expect("write config");
            fs::write(normal_home.join("auth.json"), "{}\n").expect("write auth");
            let workspace = root.join("workspace");
            fs::create_dir_all(&workspace).expect("create workspace");

            let _runtime_env = EnvVarGuard::set_path("CHORUZ_RUNTIME_DIR", &runtime_dir);
            let _codex_home_env = EnvVarGuard::set_path("CODEX_HOME", &normal_home);

            let managed =
                provision_managed_codex_home("binding-1", workspace.to_str().unwrap(), None)
                    .expect("provision managed home");

            assert!(
                managed
                    .home_path
                    .starts_with(runtime_dir.canonicalize().unwrap())
            );
            assert!(managed.sessions_path.starts_with(&managed.home_path));
            assert!(managed.sessions_path.is_dir());
            assert!(
                !managed
                    .home_path
                    .starts_with(workspace.canonicalize().unwrap())
            );

            for entry in ["config.toml", "auth.json", "plugins", "skills", "cache"] {
                let target = managed.home_path.join(entry);
                let metadata = fs::symlink_metadata(&target).expect("shared entry exists");
                assert!(
                    metadata.file_type().is_symlink(),
                    "{entry} should be a symlink"
                );
                assert_eq!(
                    fs::canonicalize(&target).unwrap(),
                    fs::canonicalize(normal_home.join(entry)).unwrap()
                );
            }

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    fs::metadata(&managed.home_path)
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o700
                );
                assert_eq!(
                    fs::metadata(&managed.sessions_path)
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o700
                );
            }
        });
    }

    #[cfg(unix)]
    #[test]
    fn imported_codex_session_becomes_a_binding_scoped_terminal_anchor() {
        with_env_lock(|| {
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
            let anchor = import_codex_terminal_session(
                "binding-1",
                "conversation-1",
                "agent-1",
                "company-1",
                "workspace-1",
                workspace.to_str().unwrap(),
                "imported-session",
            )
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
            let managed = ManagedCodexHome {
                home_path: fs::canonicalize(managed_home).expect("canonical managed home"),
                sessions_path: fs::canonicalize(
                    std::path::PathBuf::from(managed_home).join("sessions"),
                )
                .expect("canonical managed sessions"),
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
                codex_resume_session_id_from_anchor(&binding, &managed, "workspace-1", "company-1")
                    .as_deref(),
                Some("imported-session")
            );
        });
    }

    #[cfg(unix)]
    #[test]
    fn codex_session_baseline_ignores_symlinks_and_non_jsonl_files() {
        let root = isolated_test_dir("codex-session-baseline");
        let sessions = root.join("sessions");
        let day = sessions.join("2026").join("05").join("29");
        fs::create_dir_all(&day).expect("create sessions day");
        let real = day.join("rollout-1.jsonl");
        fs::write(&real, "{}\n").expect("write jsonl");
        fs::write(day.join("notes.txt"), "ignore").expect("write ignored");
        std::os::unix::fs::symlink(&real, day.join("linked.jsonl")).expect("link jsonl");

        let files = collect_codex_session_files(&sessions).expect("collect files");

        assert_eq!(files.len(), 1);
        assert!(
            files.contains(
                &fs::canonicalize(real)
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            )
        );
    }

    #[cfg(unix)]
    #[test]
    fn managed_codex_home_rejects_symlinked_binding_home() {
        with_env_lock(|| {
            let root = isolated_test_dir("codex-managed-home-symlink");
            let runtime_dir = root.join("runtime");
            let homes = runtime_dir.join("codex-homes");
            let target = root.join("redirected");
            fs::create_dir_all(&homes).expect("create homes");
            fs::create_dir_all(&target).expect("create target");
            std::os::unix::fs::symlink(&target, homes.join("binding-1"))
                .expect("create binding symlink");
            let workspace = root.join("workspace");
            fs::create_dir_all(&workspace).expect("create workspace");

            let _runtime_env = EnvVarGuard::set_path("CHORUZ_RUNTIME_DIR", &runtime_dir);
            let _codex_home_env = EnvVarGuard::remove("CODEX_HOME");

            let result =
                provision_managed_codex_home("binding-1", workspace.to_str().unwrap(), None);
            assert!(
                result.is_err(),
                "managed home must reject pre-existing binding symlinks"
            );
        });
    }

    #[cfg(unix)]
    #[test]
    fn codex_session_candidate_requires_unique_matching_workspace() {
        let root = isolated_test_dir("codex-session-candidate");
        let managed = ManagedCodexHome {
            home_path: root.join("home"),
            sessions_path: root.join("home").join("sessions"),
        };
        fs::create_dir_all(&managed.sessions_path).expect("create sessions");
        let old_file = managed.sessions_path.join("2026/05/28/old.jsonl");
        write_codex_session_meta(&old_file, "old-session", "/workspace");
        let baseline = vec![
            fs::canonicalize(&old_file)
                .unwrap()
                .to_string_lossy()
                .to_string(),
        ];
        let binding = runtime_binding_with_config(json!({
            "terminal_generation": 7,
            "agent_workspace_id": "workspace-1",
            "conversation_workspace_id": "company-1"
        }));
        let capture = capture_metadata_for(&binding, &managed, baseline);

        let new_file = managed.sessions_path.join("2026/05/29/new.jsonl");
        write_codex_session_meta(&new_file, "new-session", "/workspace");

        let candidate = unique_codex_session_candidate(&binding, &capture)
            .expect("candidate scan")
            .expect("unique candidate");
        assert_eq!(candidate.session_id, "new-session");

        let wrong_workspace = managed.sessions_path.join("2026/05/29/wrong.jsonl");
        write_codex_session_meta(&wrong_workspace, "wrong-session", "/other");
        let candidate = unique_codex_session_candidate(&binding, &capture)
            .expect("candidate scan")
            .expect("wrong workspace is ignored");
        assert_eq!(candidate.session_id, "new-session");

        let second = managed.sessions_path.join("2026/05/29/second.jsonl");
        write_codex_session_meta(&second, "second-session", "/workspace");
        assert!(
            unique_codex_session_candidate(&binding, &capture)
                .expect("candidate scan")
                .is_none(),
            "multiple matching Codex session files must fail closed"
        );
    }

    #[cfg(unix)]
    #[test]
    fn codex_resume_anchor_requires_managed_home_session_file() {
        let root = isolated_test_dir("codex-resume-anchor");
        let managed = ManagedCodexHome {
            home_path: root.join("home"),
            sessions_path: root.join("home").join("sessions"),
        };
        fs::create_dir_all(&managed.sessions_path).expect("create sessions");
        let session_path = managed.sessions_path.join("2026/05/29/session.jsonl");
        write_codex_session_meta(&session_path, "session-1", "/workspace");
        let session_path = fs::canonicalize(session_path).unwrap();
        let managed = ManagedCodexHome {
            home_path: fs::canonicalize(&managed.home_path).unwrap(),
            sessions_path: fs::canonicalize(&managed.sessions_path).unwrap(),
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

        assert_eq!(
            codex_resume_session_id_from_anchor(&binding, &managed, "workspace-1", "company-1")
                .as_deref(),
            Some("session-1")
        );

        let other_managed = ManagedCodexHome {
            home_path: root.join("other-home"),
            sessions_path: root.join("other-home").join("sessions"),
        };
        fs::create_dir_all(&other_managed.sessions_path).expect("create other sessions");
        let other_managed = ManagedCodexHome {
            home_path: fs::canonicalize(&other_managed.home_path).unwrap(),
            sessions_path: fs::canonicalize(&other_managed.sessions_path).unwrap(),
        };
        assert_eq!(
            codex_resume_session_id_from_anchor(
                &binding,
                &other_managed,
                "workspace-1",
                "company-1"
            ),
            None
        );
    }
}
