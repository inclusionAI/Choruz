use std::path::{Component, Path, PathBuf};

use choruz_common::{AppError, AppResult};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriverType {
    ClaudePrint,
    ClaudeTerminal,
    CodexExec,
    CodexAppServer,
    CodexTerminal,
    PiTerminal,
    GrokTerminal,
    #[serde(rename = "opencode_terminal")]
    OpenCodeTerminal,
    #[serde(rename = "mathcode_terminal")]
    MathCodeTerminal,
    Acp,
    /// External HTTP-webhook-driven agent (Hermes, OpenClaw, or any
    /// custom app). The pipeline never spawns a CLI for this driver;
    /// events go out via `event_webhook` and the app posts replies
    /// back via `/v1/messages`. See migration 0021.
    WebhookAgent,
}

impl DriverType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ClaudePrint => "claude_print",
            Self::ClaudeTerminal => "claude_terminal",
            Self::CodexExec => "codex_exec",
            Self::CodexAppServer => "codex_app_server",
            Self::CodexTerminal => "codex_terminal",
            Self::PiTerminal => "pi_terminal",
            Self::GrokTerminal => "grok_terminal",
            Self::OpenCodeTerminal => "opencode_terminal",
            Self::MathCodeTerminal => "mathcode_terminal",
            Self::Acp => "acp",
            Self::WebhookAgent => "webhook_agent",
        }
    }
}

impl std::str::FromStr for DriverType {
    type Err = AppError;

    fn from_str(value: &str) -> AppResult<Self> {
        match value {
            "claude_print" => Ok(Self::ClaudePrint),
            "claude_terminal" => Ok(Self::ClaudeTerminal),
            "codex_exec" => Ok(Self::CodexExec),
            "codex_app_server" => Ok(Self::CodexAppServer),
            "codex_terminal" => Ok(Self::CodexTerminal),
            "pi_terminal" => Ok(Self::PiTerminal),
            "grok_terminal" => Ok(Self::GrokTerminal),
            "opencode_terminal" => Ok(Self::OpenCodeTerminal),
            "mathcode_terminal" => Ok(Self::MathCodeTerminal),
            "acp" => Ok(Self::Acp),
            "webhook_agent" => Ok(Self::WebhookAgent),
            other => Err(AppError::Internal(format!(
                "unknown driver type from database: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingState {
    Idle,
    Running,
    Paused,
    Disabled,
    Error,
}

impl BindingState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Disabled => "disabled",
            Self::Error => "error",
        }
    }

    pub fn can_transition_to(&self, next: &Self) -> bool {
        match (self, next) {
            (Self::Disabled, Self::Running) => false,
            (current, target) if current == target => true,
            (Self::Idle, Self::Running | Self::Paused | Self::Disabled | Self::Error) => true,
            (Self::Running, Self::Idle | Self::Paused | Self::Disabled | Self::Error) => true,
            (Self::Paused, Self::Idle | Self::Disabled | Self::Error) => true,
            (Self::Error, Self::Idle | Self::Paused | Self::Disabled) => true,
            (Self::Disabled, Self::Idle) => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerType {
    Mention,
    Metadata,
    Reply,
    System,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeBinding {
    pub id: String,
    pub conversation_id: String,
    pub agent_principal_id: String,
    pub driver_type: DriverType,
    pub workspace_path: String,
    pub git_worktree_path: Option<String>,
    pub external_session_id: Option<String>,
    pub external_thread_id: Option<String>,
    pub last_event_cursor: i64,
    pub last_acked_event_cursor: i64,
    pub last_seen_server_seq: i64,
    pub state: BindingState,
    pub last_error: Option<String>,
    pub in_flight_turn_id: Option<String>,
    pub last_trigger_message_id: Option<String>,
    pub config_json: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditActor {
    pub actor_id: String,
    pub workspace_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateBindingInput {
    pub conversation_id: String,
    pub agent_principal_id: String,
    pub driver_type: DriverType,
    pub workspace_path: String,
    pub git_worktree_path: Option<String>,
    pub config_json: Value,
    pub audit_actor: Option<AuditActor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSessionAnchorInput {
    pub session_id: String,
    pub source: String,
    pub provenance: String,
    pub binding_id: String,
    pub conversation_id: String,
    pub agent_principal_id: String,
    pub company_id: String,
    pub driver_type: String,
    pub workspace_id: String,
    pub workspace_path: String,
    pub native_home_path: String,
    pub native_session_path: String,
    pub binding_generation: i64,
    pub binding_updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSessionAnchor {
    #[serde(default)]
    pub runtime_host_id: Option<String>,
    pub driver_type: String,
    pub session_id: String,
    pub source: String,
    pub provenance: String,
    pub binding_id: String,
    pub conversation_id: String,
    pub agent_principal_id: String,
    #[serde(default)]
    pub company_id: String,
    pub workspace_id: String,
    pub workspace_path: String,
    #[serde(default)]
    pub native_home_path: String,
    #[serde(default)]
    pub native_session_path: String,
    #[serde(default)]
    pub binding_generation: Option<i64>,
    pub captured_at: String,
    #[serde(default)]
    pub last_verified_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexTerminalCaptureInput {
    pub binding_id: String,
    pub conversation_id: String,
    pub agent_principal_id: String,
    pub company_id: String,
    pub driver_type: String,
    pub workspace_id: String,
    pub workspace_path: String,
    pub native_home_path: String,
    pub sessions_path: String,
    pub spawn_started_at: DateTime<Utc>,
    pub baseline_session_files: Vec<String>,
    pub binding_updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexTerminalCaptureMetadata {
    pub binding_id: String,
    pub conversation_id: String,
    pub agent_principal_id: String,
    pub company_id: String,
    pub driver_type: String,
    pub workspace_id: String,
    pub workspace_path: String,
    pub native_home_path: String,
    pub sessions_path: String,
    pub binding_generation: i64,
    pub spawn_started_at: String,
    pub baseline_session_files: Vec<String>,
}

impl RuntimeBinding {
    fn terminal_session_provenance_matches(&self, anchor: &TerminalSessionAnchor) -> bool {
        if anchor.runtime_host_id.as_deref()
            != self
                .config_json
                .get("runtime_host_id")
                .and_then(Value::as_str)
        {
            return false;
        }
        if anchor.provenance == "direct_session_reserved"
            && self.driver_type == DriverType::ClaudeTerminal
        {
            return true;
        }
        if anchor.provenance == "terminal_process_captured" {
            return true;
        }
        anchor.provenance == "workspace_scan_imported"
            && self.driver_type == DriverType::CodexTerminal
            && self
                .config_json
                .get("native_session_import")
                .is_some_and(|import| {
                    import.get("native_session_id").and_then(Value::as_str)
                        == Some(anchor.session_id.as_str())
                        && import.get("workspace_path").and_then(Value::as_str)
                            == Some(anchor.workspace_path.as_str())
                })
    }

    pub fn terminal_session_anchor(&self) -> Option<TerminalSessionAnchor> {
        serde_json::from_value(self.config_json.get("terminal_session")?.clone()).ok()
    }

    pub fn codex_terminal_capture_metadata(&self) -> Option<CodexTerminalCaptureMetadata> {
        serde_json::from_value(self.config_json.get("terminal_capture")?.clone()).ok()
    }

    pub fn terminal_generation(&self) -> i64 {
        self.config_json
            .get("terminal_generation")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
    }

    pub fn valid_terminal_session_id(&self) -> Option<String> {
        self.valid_terminal_session_id_for_workspace(None)
    }

    pub fn valid_terminal_session_id_for_workspace(
        &self,
        expected_workspace_id: Option<&str>,
    ) -> Option<String> {
        let anchor = self.terminal_session_anchor()?;
        let session_id = anchor.session_id.trim();
        if session_id.is_empty()
            || anchor.driver_type != self.driver_type.as_str()
            || anchor.binding_id != self.id
            || anchor.conversation_id != self.conversation_id
            || anchor.agent_principal_id != self.agent_principal_id
            || anchor.workspace_path != self.workspace_path
            || anchor.source != "native_cli"
            || !self.terminal_session_provenance_matches(&anchor)
            || expected_workspace_id.is_some_and(|workspace_id| anchor.workspace_id != workspace_id)
        {
            return None;
        }
        Some(session_id.to_string())
    }

    pub fn valid_terminal_session_anchor_for_context(
        &self,
        expected_workspace_id: Option<&str>,
        expected_company_id: Option<&str>,
        expected_generation: Option<i64>,
        expected_native_home_path: Option<&str>,
    ) -> Option<TerminalSessionAnchor> {
        let anchor = self.terminal_session_anchor()?;
        let session_id = anchor.session_id.trim();
        if session_id.is_empty()
            || anchor.driver_type != self.driver_type.as_str()
            || anchor.binding_id != self.id
            || anchor.conversation_id != self.conversation_id
            || anchor.agent_principal_id != self.agent_principal_id
            || anchor.workspace_path != self.workspace_path
            || anchor.source != "native_cli"
            || !self.terminal_session_provenance_matches(&anchor)
            || expected_workspace_id.is_some_and(|workspace_id| anchor.workspace_id != workspace_id)
            || expected_company_id.is_some_and(|company_id| anchor.company_id != company_id)
            || expected_generation.is_some_and(|generation| {
                anchor.binding_generation != Some(generation)
                    || self.terminal_generation() != generation
            })
            || expected_native_home_path
                .is_some_and(|native_home_path| anchor.native_home_path != native_home_path)
        {
            return None;
        }
        Some(anchor)
    }
}

#[cfg(test)]
mod terminal_session_tests {
    use super::{BindingState, DriverType, RuntimeBinding};
    use chrono::Utc;
    use serde_json::json;

    fn binding(config_json: serde_json::Value) -> RuntimeBinding {
        RuntimeBinding {
            id: "binding-1".into(),
            conversation_id: "conversation-1".into(),
            agent_principal_id: "agent-1".into(),
            driver_type: DriverType::CodexTerminal,
            workspace_path: "/workspace".into(),
            git_worktree_path: None,
            external_session_id: None,
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
    fn imported_terminal_anchor_requires_the_matching_selected_session() {
        let anchor = json!({
            "driver_type": "codex_terminal",
            "session_id": "selected-session",
            "source": "native_cli",
            "provenance": "workspace_scan_imported",
            "binding_id": "binding-1",
            "conversation_id": "conversation-1",
            "agent_principal_id": "agent-1",
            "company_id": "company-1",
            "workspace_id": "workspace-1",
            "workspace_path": "/workspace",
            "native_home_path": "/runtime/codex-homes/binding-1",
            "native_session_path": "/runtime/codex-homes/binding-1/sessions/imported.jsonl",
            "binding_generation": 0,
            "captured_at": "2026-09-03T00:00:00Z",
        });
        let valid = binding(json!({
            "native_session_import": {
                "native_session_id": "selected-session",
                "workspace_path": "/workspace",
            },
            "terminal_session": anchor,
        }));
        assert_eq!(
            valid
                .valid_terminal_session_id_for_workspace(Some("workspace-1"))
                .as_deref(),
            Some("selected-session")
        );

        let invalid = binding(json!({
            "native_session_import": {
                "native_session_id": "other-session",
                "workspace_path": "/workspace",
            },
            "terminal_session": valid.config_json["terminal_session"],
        }));
        assert_eq!(invalid.valid_terminal_session_id(), None);
    }
}

pub fn normalize_workspace_path(path: &str) -> AppResult<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(AppError::Validation("workspace path is required".into()));
    }

    let source = Path::new(trimmed);
    let mut normalized = PathBuf::new();

    for component in source.components() {
        match component {
            Component::ParentDir => {
                return Err(AppError::Validation(
                    "workspace path cannot contain parent segments".into(),
                ));
            }
            Component::CurDir => {}
            Component::RootDir => normalized.push("/"),
            Component::Normal(segment) => normalized.push(segment),
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
        }
    }

    let normalized = normalized.to_string_lossy().to_string();
    if normalized.is_empty() {
        return Err(AppError::Validation("workspace path is required".into()));
    }
    Ok(normalized)
}

fn find_latest_session_on_disk(workspace_path: &str, driver_type: &str) -> Option<String> {
    match driver_type {
        "claude_print" | "claude_terminal" => find_latest_claude_session(workspace_path),
        "pi_terminal" => find_latest_pi_session(workspace_path),
        "grok_terminal" => find_latest_grok_session(workspace_path),
        // Codex stores sessions globally without a binding identity, and
        // OpenCode is queried through its own session registry in the async
        // caller. Never let either fall through to another CLI's store.
        _ => None,
    }
}

/// The newest native session the CLI for `driver_type` stored for a
/// workspace on this device. Codex sessions carry no binding identity and are
/// never guessed; OpenCode is asked through its own session registry.
pub async fn latest_native_session(workspace_path: &str, driver_type: &str) -> Option<String> {
    if driver_type == "opencode_terminal" {
        find_latest_opencode_session(workspace_path).await
    } else {
        let workspace_path = workspace_path.to_owned();
        let driver_type = driver_type.to_owned();
        tokio::task::spawn_blocking(move || {
            find_latest_session_on_disk(&workspace_path, &driver_type)
        })
        .await
        .ok()
        .flatten()
    }
}

fn find_latest_pi_session(workspace_path: &str) -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    find_latest_pi_session_in_root(
        &std::path::PathBuf::from(home).join(".pi/agent/sessions"),
        workspace_path,
    )
}

fn find_latest_pi_session_in_root(
    sessions_root: &std::path::Path,
    workspace_path: &str,
) -> Option<String> {
    use std::io::BufRead;

    let mut best: Option<(std::time::SystemTime, String)> = None;
    for project_entry in std::fs::read_dir(sessions_root).ok()?.flatten() {
        let project_path = project_entry.path();
        if !project_path.is_dir() {
            continue;
        }
        let Ok(session_entries) = std::fs::read_dir(project_path) else {
            continue;
        };
        for session_entry in session_entries.flatten() {
            let session_path = session_entry.path();
            if session_path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(file) = std::fs::File::open(&session_path) else {
                continue;
            };
            let Some(Ok(header)) = std::io::BufReader::new(file).lines().next() else {
                continue;
            };
            let Ok(header) = serde_json::from_str::<serde_json::Value>(&header) else {
                continue;
            };
            if header.get("type").and_then(|value| value.as_str()) != Some("session")
                || header.get("cwd").and_then(|value| value.as_str()) != Some(workspace_path)
            {
                continue;
            }
            let Some(session_id) = header.get("id").and_then(|value| value.as_str()) else {
                continue;
            };
            let modified = session_entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            if best
                .as_ref()
                .is_none_or(|(best_modified, _)| modified > *best_modified)
            {
                best = Some((modified, session_id.to_owned()));
            }
        }
    }
    best.map(|(_, session_id)| session_id)
}

fn find_latest_grok_session(workspace_path: &str) -> Option<String> {
    let root = std::env::var_os("GROK_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".grok"))
        })?;
    find_latest_grok_session_in_root(&root.join("sessions"), workspace_path)
}

fn find_latest_grok_session_in_root(
    sessions_root: &std::path::Path,
    workspace_path: &str,
) -> Option<String> {
    let mut best: Option<(std::time::SystemTime, String)> = None;
    for cwd_entry in std::fs::read_dir(sessions_root).ok()?.flatten() {
        let cwd_path = cwd_entry.path();
        if !cwd_path.is_dir() {
            continue;
        }
        let Ok(session_entries) = std::fs::read_dir(cwd_path) else {
            continue;
        };
        for session_entry in session_entries.flatten() {
            let session_path = session_entry.path();
            if !session_path.is_dir() {
                continue;
            }
            let summary_path = session_path.join("summary.json");
            let Ok(summary) = std::fs::read_to_string(&summary_path) else {
                continue;
            };
            let Ok(summary) = serde_json::from_str::<serde_json::Value>(&summary) else {
                continue;
            };
            let info = &summary["info"];
            if info.get("cwd").and_then(|value| value.as_str()) != Some(workspace_path) {
                continue;
            }
            let Some(session_id) = info.get("id").and_then(|value| value.as_str()) else {
                continue;
            };
            let modified = std::fs::metadata(&summary_path)
                .and_then(|metadata| metadata.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            if best
                .as_ref()
                .is_none_or(|(best_modified, _)| modified > *best_modified)
            {
                best = Some((modified, session_id.to_owned()));
            }
        }
    }
    best.map(|(_, session_id)| session_id)
}

async fn find_latest_opencode_session(workspace_path: &str) -> Option<String> {
    let binary = std::env::var("CHORUZ_OPENCODE_BINARY").unwrap_or_else(|_| "opencode".into());
    let mut command = tokio::process::Command::new(binary);
    command
        .args(["session", "list", "--format", "json"])
        .current_dir(workspace_path)
        .kill_on_drop(true);
    let output = tokio::time::timeout(std::time::Duration::from_secs(5), command.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    latest_opencode_session_from_json(&output.stdout, workspace_path)
}

fn latest_opencode_session_from_json(output: &[u8], workspace_path: &str) -> Option<String> {
    let sessions = serde_json::from_slice::<Vec<serde_json::Value>>(output).ok()?;
    sessions
        .into_iter()
        .filter(|session| {
            session.get("directory").and_then(|value| value.as_str()) == Some(workspace_path)
        })
        .filter_map(|session| {
            Some((
                session.get("updated").and_then(|value| value.as_i64())?,
                session
                    .get("id")
                    .and_then(|value| value.as_str())?
                    .to_owned(),
            ))
        })
        .max_by_key(|(updated, _)| *updated)
        .map(|(_, session_id)| session_id)
}

fn find_latest_claude_session(workspace_path: &str) -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    find_latest_claude_session_in_home(&std::path::PathBuf::from(home), workspace_path)
}

fn find_latest_claude_session_in_home(
    home: &std::path::Path,
    workspace_path: &str,
) -> Option<String> {
    // Claude Code mangles CWD: / . _ all become -
    let replaced: String = workspace_path
        .chars()
        .map(|c| {
            if c == '/' || c == '.' || c == '_' {
                '-'
            } else {
                c
            }
        })
        .collect();
    // workspace_path starts with /, which becomes -, so no extra prefix needed
    let mangled = replaced;
    let session_dir = home.join(".claude").join("projects").join(&mangled);

    if !session_dir.is_dir() {
        return None;
    }

    let mut best: Option<(std::time::SystemTime, String)> = None;
    if let Ok(entries) = std::fs::read_dir(&session_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl")
                && let Ok(meta) = path.metadata()
                && let Ok(modified) = meta.modified()
                && best.as_ref().is_none_or(|(t, _)| modified > *t)
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                best = Some((modified, stem.to_string()));
            }
        }
    }
    best.map(|(_, id)| id)
}

// ── Unit tests (QA-004) ─────────────────────────────────────────────────

#[cfg(test)]
mod state_tests {
    use super::*;
    use std::fs;
    use uuid::Uuid;

    // ── can_transition_to ────────────────────────────────────────────

    #[test]
    fn idle_to_running() {
        assert!(BindingState::Idle.can_transition_to(&BindingState::Running));
    }

    #[test]
    fn idle_to_disabled() {
        assert!(BindingState::Idle.can_transition_to(&BindingState::Disabled));
    }

    #[test]
    fn idle_to_error() {
        assert!(BindingState::Idle.can_transition_to(&BindingState::Error));
    }

    #[test]
    fn idle_to_paused() {
        assert!(BindingState::Idle.can_transition_to(&BindingState::Paused));
    }

    #[test]
    fn running_to_idle() {
        assert!(BindingState::Running.can_transition_to(&BindingState::Idle));
    }

    #[test]
    fn running_to_error() {
        assert!(BindingState::Running.can_transition_to(&BindingState::Error));
    }

    #[test]
    fn running_to_disabled() {
        assert!(BindingState::Running.can_transition_to(&BindingState::Disabled));
    }

    #[test]
    fn disabled_cannot_go_to_running() {
        assert!(!BindingState::Disabled.can_transition_to(&BindingState::Running));
    }

    #[test]
    fn disabled_to_idle() {
        assert!(BindingState::Disabled.can_transition_to(&BindingState::Idle));
    }

    #[test]
    fn error_to_idle() {
        assert!(BindingState::Error.can_transition_to(&BindingState::Idle));
    }

    #[test]
    fn error_to_disabled() {
        assert!(BindingState::Error.can_transition_to(&BindingState::Disabled));
    }

    #[test]
    fn error_to_paused() {
        assert!(BindingState::Error.can_transition_to(&BindingState::Paused));
    }

    #[test]
    fn paused_to_idle() {
        assert!(BindingState::Paused.can_transition_to(&BindingState::Idle));
    }

    #[test]
    fn paused_to_disabled() {
        assert!(BindingState::Paused.can_transition_to(&BindingState::Disabled));
    }

    #[test]
    fn paused_cannot_go_to_running() {
        assert!(!BindingState::Paused.can_transition_to(&BindingState::Running));
    }

    #[test]
    fn disabled_cannot_go_to_paused() {
        assert!(!BindingState::Disabled.can_transition_to(&BindingState::Paused));
    }

    #[test]
    fn disabled_cannot_go_to_error() {
        assert!(!BindingState::Disabled.can_transition_to(&BindingState::Error));
    }

    #[test]
    fn same_state_identity() {
        for state in [
            BindingState::Idle,
            BindingState::Running,
            BindingState::Paused,
            BindingState::Disabled,
            BindingState::Error,
        ] {
            assert!(
                state.can_transition_to(&state),
                "{state:?} -> {state:?} should be allowed"
            );
        }
    }

    #[test]
    fn codex_session_lookup_never_guesses_from_disk() {
        assert_eq!(
            find_latest_session_on_disk("/same/repo/workspace", "codex_terminal"),
            None
        );
        assert_eq!(
            find_latest_session_on_disk("/same/repo/workspace", "codex_exec"),
            None
        );
    }

    #[test]
    fn claude_session_lookup_is_scoped_to_exact_workspace_project_dir() {
        let root = std::env::temp_dir().join(format!(
            "choruz-claude-session-scope-{}",
            Uuid::now_v7().simple()
        ));
        let workspace_a = root.join("workspace_a");
        let workspace_b = root.join("workspace_b");
        fs::create_dir_all(&workspace_a).expect("create workspace a");
        fs::create_dir_all(&workspace_b).expect("create workspace b");

        let session_a = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        let session_b = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
        write_claude_session_file(&root, workspace_a.to_str().unwrap(), session_a);
        write_claude_session_file(&root, workspace_b.to_str().unwrap(), session_b);

        assert_eq!(
            find_latest_claude_session_in_home(&root, workspace_a.to_str().unwrap()).as_deref(),
            Some(session_a),
            "agent A must not pick up agent B's Claude session file"
        );
        assert_eq!(
            find_latest_claude_session_in_home(&root, workspace_b.to_str().unwrap()).as_deref(),
            Some(session_b),
            "agent B must not pick up agent A's Claude session file"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn pi_session_lookup_reads_header_and_scopes_by_workspace() {
        let root = std::env::temp_dir().join(format!(
            "choruz-pi-session-scope-{}",
            Uuid::now_v7().simple()
        ));
        let project_dir = root.join("--workspace--");
        fs::create_dir_all(&project_dir).expect("create pi project session dir");
        fs::write(
            project_dir.join("one.jsonl"),
            "{\"type\":\"session\",\"id\":\"pi-session-a\",\"cwd\":\"/workspace/a\"}\n",
        )
        .expect("write pi session");
        fs::write(
            project_dir.join("other.jsonl"),
            "{\"type\":\"session\",\"id\":\"pi-session-b\",\"cwd\":\"/workspace/b\"}\n",
        )
        .expect("write other pi session");

        assert_eq!(
            find_latest_pi_session_in_root(&root, "/workspace/a").as_deref(),
            Some("pi-session-a")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn grok_session_lookup_reads_summary_and_scopes_by_workspace() {
        let root = std::env::temp_dir().join(format!(
            "choruz-grok-session-scope-{}",
            Uuid::now_v7().simple()
        ));
        let session_dir = root.join("encoded-cwd").join("grok-session-a");
        fs::create_dir_all(&session_dir).expect("create grok session dir");
        fs::write(
            session_dir.join("summary.json"),
            "{\"info\":{\"id\":\"grok-session-a\",\"cwd\":\"/workspace/a\"}}",
        )
        .expect("write grok summary");

        assert_eq!(
            find_latest_grok_session_in_root(&root, "/workspace/a").as_deref(),
            Some("grok-session-a")
        );
        assert_eq!(
            find_latest_grok_session_in_root(&root, "/workspace/b"),
            None
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn opencode_session_lookup_uses_exact_directory_and_latest_update() {
        let output = br#"[
          {"id":"wrong","directory":"/workspace/b","updated":999},
          {"id":"older","directory":"/workspace/a","updated":10},
          {"id":"latest","directory":"/workspace/a","updated":20}
        ]"#;

        assert_eq!(
            latest_opencode_session_from_json(output, "/workspace/a").as_deref(),
            Some("latest")
        );
        assert_eq!(
            latest_opencode_session_from_json(output, "/workspace/c"),
            None
        );
    }

    fn write_claude_session_file(home: &std::path::Path, workspace_path: &str, session_id: &str) {
        let mangled: String = workspace_path
            .chars()
            .map(|c| {
                if c == '/' || c == '.' || c == '_' {
                    '-'
                } else {
                    c
                }
            })
            .collect();
        let session_dir = home.join(".claude").join("projects").join(mangled);
        fs::create_dir_all(&session_dir).expect("create claude project session dir");
        fs::write(session_dir.join(format!("{session_id}.jsonl")), "{}\n")
            .expect("write claude session file");
    }
}
