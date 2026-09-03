use std::{
    fmt,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use choruz_common::{AppError, AppResult, new_id};
use chrono::{DateTime, Utc};
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_postgres::{Error as PgError, Row, error::SqlState};

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

    fn from_str(value: &str) -> AppResult<Self> {
        match value {
            "idle" => Ok(Self::Idle),
            "running" => Ok(Self::Running),
            "paused" => Ok(Self::Paused),
            "disabled" => Ok(Self::Disabled),
            "error" => Ok(Self::Error),
            other => Err(AppError::Internal(format!(
                "unknown binding state from database: {other}"
            ))),
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

pub trait Clock: Send + Sync + fmt::Debug {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Debug)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

fn build_pool(database_url: &str) -> Pool {
    let mut pg_config: tokio_postgres::Config = database_url
        .parse()
        .expect("invalid database connection string");
    pg_config.connect_timeout(std::time::Duration::from_secs(5));
    let mgr_config = ManagerConfig {
        // Verified: run "SELECT 1" before handing out a connection.
        // Prevents "db error" from stale/dead connections in the pool
        // (e.g. after PG restart, network blip, idle timeout).
        recycling_method: RecyclingMethod::Verified,
    };
    let mgr = Manager::from_config(pg_config, tokio_postgres::NoTls, mgr_config);
    Pool::builder(mgr)
        .max_size(16)
        .build()
        .expect("failed to build connection pool")
}

#[derive(Clone, Debug)]
pub struct RuntimeStore {
    pub(crate) pool: Pool,
    pub(crate) clock: Arc<dyn Clock>,
}

impl RuntimeStore {
    pub fn new(database_url: impl Into<String>) -> Self {
        let url = database_url.into();
        Self {
            pool: build_pool(&url),
            clock: Arc::new(SystemClock),
        }
    }

    pub fn with_clock(database_url: impl Into<String>, clock: Arc<dyn Clock>) -> Self {
        let url = database_url.into();
        Self {
            pool: build_pool(&url),
            clock,
        }
    }

    pub async fn create_binding(&self, _input: CreateBindingInput) -> AppResult<RuntimeBinding> {
        let workspace_path = normalize_workspace_path(&_input.workspace_path)?;
        let git_worktree_path = _input
            .git_worktree_path
            .as_deref()
            .map(normalize_workspace_path)
            .transpose()?;
        let mut client = self.connect().await?;
        let tx = client.transaction().await.map_err(map_db_error)?;
        let now = self.clock.now();
        let id = new_id();
        let config_json = if _input.config_json.is_null() {
            Value::Object(Default::default())
        } else {
            _input.config_json.clone()
        };

        let principal = tx
            .query_opt(
                "SELECT type, disabled, deleted_at
                 FROM principal
                 WHERE id = $1
                 FOR SHARE",
                &[&_input.agent_principal_id],
            )
            .await
            .map_err(map_db_error)?
            .ok_or_else(|| AppError::NotFound("agent principal not found".into()))?;
        let principal_type: String = principal.get("type");
        if principal_type != "agent" {
            return Err(AppError::Validation(
                "runtime bindings require an Agent principal".into(),
            ));
        }
        if principal.get::<_, bool>("disabled")
            || principal
                .get::<_, Option<chrono::DateTime<chrono::Utc>>>("deleted_at")
                .is_some()
        {
            return Err(AppError::Forbidden("agent principal is disabled".into()));
        }

        let row = tx
            .query_one(
                "INSERT INTO agent_runtime_bindings (
                   id,
                   conversation_id,
                   agent_principal_id,
                   driver_type,
                   workspace_path,
                   git_worktree_path,
                   external_session_id,
                   external_thread_id,
                   last_event_cursor,
                   last_acked_event_cursor,
                   last_seen_server_seq,
                   state,
                   last_error,
                   in_flight_turn_id,
                   last_trigger_message_id,
                   config_json,
                   created_at,
                   updated_at
                 ) VALUES (
                   $1, $2, $3, $4, $5, $6, NULL, NULL, 0, 0, 0, 'idle', NULL, NULL, NULL, $7, $8, $8
                 )
                 RETURNING
                   id,
                   conversation_id,
                   agent_principal_id,
                   driver_type,
                   workspace_path,
                   git_worktree_path,
                   external_session_id,
                   external_thread_id,
                   last_event_cursor,
                   last_acked_event_cursor,
                   last_seen_server_seq,
                   state,
                   last_error,
                   in_flight_turn_id,
                   last_trigger_message_id,
                   config_json,
                   created_at,
                   updated_at",
                &[
                    &id,
                    &_input.conversation_id,
                    &_input.agent_principal_id,
                    &_input.driver_type.as_str(),
                    &workspace_path,
                    &git_worktree_path,
                    &config_json,
                    &now,
                ],
            )
            .await
            .map_err(map_db_error)?;
        let binding = binding_from_row(&row)?;

        // The principal's SHARE lock is held through this commit. Principal
        // disable takes an UPDATE lock and disables all bindings in that same
        // transaction, so creation can neither slip into nor outlive disable.
        tx.commit().await.map_err(map_db_error)?;

        if let Some(actor) = &_input.audit_actor {
            self.record_audit(
                &client,
                actor,
                "runtime.binding_created",
                "agent_runtime_binding",
                &binding.id,
                serde_json::json!({
                    "conversation_id": binding.conversation_id,
                    "agent_principal_id": binding.agent_principal_id,
                    "driver_type": binding.driver_type.as_str(),
                    "workspace_path": binding.workspace_path,
                }),
            )
            .await?;
        }

        Ok(binding)
    }

    pub async fn get_binding(&self, binding_id: &str) -> AppResult<RuntimeBinding> {
        let client = self.connect().await?;
        self.fetch_binding(&client, binding_id).await
    }

    pub async fn list_bindings(&self) -> AppResult<Vec<RuntimeBinding>> {
        let client = self.connect().await?;
        let rows = client
            .query(
                "SELECT
                   id,
                   conversation_id,
                   agent_principal_id,
                   driver_type,
                   workspace_path,
                   git_worktree_path,
                   external_session_id,
                   external_thread_id,
                   last_event_cursor,
                   last_acked_event_cursor,
                   last_seen_server_seq,
                   state,
                   last_error,
                   in_flight_turn_id,
                   last_trigger_message_id,
                   config_json,
                   created_at,
                   updated_at
                 FROM agent_runtime_bindings
                 ORDER BY created_at ASC",
                &[],
            )
            .await
            .map_err(map_db_error)?;
        rows.iter().map(binding_from_row).collect()
    }

    pub async fn list_active_bindings(&self) -> AppResult<Vec<RuntimeBinding>> {
        let client = self.connect().await?;
        let rows = client
            .query(
                "SELECT
                   id,
                   conversation_id,
                   agent_principal_id,
                   driver_type,
                   workspace_path,
                   git_worktree_path,
                   external_session_id,
                   external_thread_id,
                   last_event_cursor,
                   last_acked_event_cursor,
                   last_seen_server_seq,
                   state,
                   last_error,
                   in_flight_turn_id,
                   last_trigger_message_id,
                   config_json,
                   created_at,
                   updated_at
                 FROM agent_runtime_bindings
                 WHERE state NOT IN ('disabled', 'paused')
                 ORDER BY created_at ASC",
                &[],
            )
            .await
            .map_err(map_db_error)?;
        rows.iter().map(binding_from_row).collect()
    }

    pub async fn update_binding_state(
        &self,
        binding_id: &str,
        next_state: BindingState,
        actor: Option<&AuditActor>,
    ) -> AppResult<RuntimeBinding> {
        let mut client = self.connect().await?;
        let tx = client.transaction().await.map_err(map_db_error)?;

        let existing = {
            let row = tx
                .query_opt(
                    "SELECT
                       id,
                       conversation_id,
                       agent_principal_id,
                       driver_type,
                       workspace_path,
                       git_worktree_path,
                       external_session_id,
                       external_thread_id,
                       last_event_cursor,
                       last_acked_event_cursor,
                       last_seen_server_seq,
                       state,
                       last_error,
                       in_flight_turn_id,
                       last_trigger_message_id,
                       config_json,
                       created_at,
                       updated_at
                     FROM agent_runtime_bindings
                     WHERE id = $1
                     FOR UPDATE",
                    &[&binding_id],
                )
                .await
                .map_err(map_db_error)?;
            let row = row.ok_or_else(|| AppError::NotFound("runtime binding not found".into()))?;
            binding_from_row(&row)?
        };

        if !existing.state.can_transition_to(&next_state) {
            tx.rollback().await.map_err(map_db_error)?;
            return Err(AppError::Validation(format!(
                "invalid binding state transition: {} -> {}",
                existing.state.as_str(),
                next_state.as_str()
            )));
        }

        let now = self.clock.now();
        let row = tx
            .query_one(
                "UPDATE agent_runtime_bindings
                 SET state = $2, updated_at = $3
                 WHERE id = $1
                 RETURNING
                   id,
                   conversation_id,
                   agent_principal_id,
                   driver_type,
                   workspace_path,
                   git_worktree_path,
                   external_session_id,
                   external_thread_id,
                   last_event_cursor,
                   last_acked_event_cursor,
                   last_seen_server_seq,
                   state,
                   last_error,
                   in_flight_turn_id,
                   last_trigger_message_id,
                   config_json,
                   created_at,
                   updated_at",
                &[&binding_id, &next_state.as_str(), &now],
            )
            .await
            .map_err(map_db_error)?;
        let binding = binding_from_row(&row)?;

        if let Some(actor) = actor {
            let action = binding_transition_audit_action(&existing.state, &next_state);
            tx.execute(
                "INSERT INTO principal (
                   id, workspace_id, type, name, created_at, updated_at
                 ) VALUES ($1, $2, 'human', $3, $4, $4)
                 ON CONFLICT (id) DO UPDATE
                 SET workspace_id = EXCLUDED.workspace_id,
                     updated_at = EXCLUDED.updated_at",
                &[
                    &actor.actor_id,
                    &actor.workspace_id,
                    &actor.actor_id,
                    &self.clock.now(),
                ],
            )
            .await
            .map_err(map_db_error)?;
            tx.execute(
                "INSERT INTO audit_log (
                   id, workspace_id, actor_id, action, target_type, target_id, metadata, created_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[
                    &new_id(),
                    &actor.workspace_id,
                    &actor.actor_id,
                    &action,
                    &"agent_runtime_binding",
                    &binding_id,
                    &serde_json::json!({
                        "previous_state": existing.state.as_str(),
                        "state": binding.state.as_str(),
                    }),
                    &self.clock.now(),
                ],
            )
            .await
            .map_err(map_db_error)?;
        }

        tx.commit().await.map_err(map_db_error)?;
        Ok(binding)
    }

    /// Atomically make every runtime binding for an Agent non-executable.
    ///
    /// Disabled rows remain as audit tombstones. Repeating the operation is
    /// safe and reports zero newly-disabled bindings.
    pub async fn disable_bindings_by_agent(&self, agent_principal_id: &str) -> AppResult<u64> {
        let client = self.connect().await?;
        let now = self.clock.now();
        client
            .execute(
                "UPDATE agent_runtime_bindings
                 SET state = 'disabled',
                     in_flight_turn_id = NULL,
                     updated_at = $2
                 WHERE agent_principal_id = $1
                   AND state <> 'disabled'",
                &[&agent_principal_id, &now],
            )
            .await
            .map_err(map_db_error)
    }

    pub async fn rebind_workspace(
        &self,
        binding_id: &str,
        workspace_path: &str,
        actor: &AuditActor,
    ) -> AppResult<RuntimeBinding> {
        let normalized = normalize_workspace_path(workspace_path)?;
        let client = self.connect().await?;
        let now = self.clock.now();
        let row = client
            .query_opt(
                "UPDATE agent_runtime_bindings
                 SET workspace_path = $2,
                     config_json = jsonb_set(
                       config_json - 'terminal_session' - 'terminal_capture',
                       '{terminal_generation}',
                       to_jsonb(COALESCE((config_json->>'terminal_generation')::bigint, 0) + 1),
                       true
                     ),
                     updated_at = $3
                 WHERE id = $1
                 RETURNING
                   id,
                   conversation_id,
                   agent_principal_id,
                   driver_type,
                   workspace_path,
                   git_worktree_path,
                   external_session_id,
                   external_thread_id,
                   last_event_cursor,
                   last_acked_event_cursor,
                   last_seen_server_seq,
                   state,
                   last_error,
                   in_flight_turn_id,
                   last_trigger_message_id,
                   config_json,
                   created_at,
                   updated_at",
                &[&binding_id, &normalized, &now],
            )
            .await
            .map_err(map_db_error)?;
        let row = row.ok_or_else(|| AppError::NotFound("runtime binding not found".into()))?;
        let binding = binding_from_row(&row)?;
        self.record_audit(
            &client,
            actor,
            "runtime.binding_rebound",
            "agent_runtime_binding",
            binding_id,
            serde_json::json!({
                "workspace_path": binding.workspace_path,
            }),
        )
        .await?;
        Ok(binding)
    }

    pub async fn update_binding_cursors(
        &self,
        binding_id: &str,
        last_event_cursor: i64,
        last_acked_event_cursor: i64,
    ) -> AppResult<RuntimeBinding> {
        let client = self.connect().await?;
        let now = self.clock.now();
        let row = client
            .query_opt(
                "UPDATE agent_runtime_bindings
                 SET last_event_cursor = $2,
                     last_acked_event_cursor = $3,
                     updated_at = $4
                 WHERE id = $1
                 RETURNING
                   id,
                   conversation_id,
                   agent_principal_id,
                   driver_type,
                   workspace_path,
                   git_worktree_path,
                   external_session_id,
                   external_thread_id,
                   last_event_cursor,
                   last_acked_event_cursor,
                   last_seen_server_seq,
                   state,
                   last_error,
                   in_flight_turn_id,
                   last_trigger_message_id,
                   config_json,
                   created_at,
                   updated_at",
                &[
                    &binding_id,
                    &last_event_cursor,
                    &last_acked_event_cursor,
                    &now,
                ],
            )
            .await
            .map_err(map_db_error)?;
        let row = row.ok_or_else(|| AppError::NotFound("runtime binding not found".into()))?;
        binding_from_row(&row)
    }

    pub async fn update_binding_config_json(
        &self,
        binding_id: &str,
        config_json: Value,
    ) -> AppResult<RuntimeBinding> {
        let client = self.connect().await?;
        let now = self.clock.now();
        let row = client
            .query_opt(
                "UPDATE agent_runtime_bindings
                 SET config_json = $2,
                     updated_at = $3
                 WHERE id = $1
                 RETURNING
                   id,
                   conversation_id,
                   agent_principal_id,
                   driver_type,
                   workspace_path,
                   git_worktree_path,
                   external_session_id,
                   external_thread_id,
                   last_event_cursor,
                   last_acked_event_cursor,
                   last_seen_server_seq,
                   state,
                   last_error,
                   in_flight_turn_id,
                   last_trigger_message_id,
                   config_json,
                   created_at,
                   updated_at",
                &[&binding_id, &config_json, &now],
            )
            .await
            .map_err(map_db_error)?;
        let row = row.ok_or_else(|| AppError::NotFound("runtime binding not found".into()))?;
        binding_from_row(&row)
    }

    pub async fn begin_codex_terminal_capture(
        &self,
        binding_id: &str,
        input: CodexTerminalCaptureInput,
    ) -> AppResult<RuntimeBinding> {
        if binding_id != input.binding_id {
            return Err(AppError::Validation(
                "terminal capture binding mismatch".into(),
            ));
        }

        let mut client = self.connect().await?;
        let tx = client.transaction().await.map_err(map_db_error)?;
        let row = tx
            .query_opt(
                "SELECT
                   id,
                   conversation_id,
                   agent_principal_id,
                   driver_type,
                   workspace_path,
                   git_worktree_path,
                   external_session_id,
                   external_thread_id,
                   last_event_cursor,
                   last_acked_event_cursor,
                   last_seen_server_seq,
                   state,
                   last_error,
                   in_flight_turn_id,
                   last_trigger_message_id,
                   config_json,
                   created_at,
                   updated_at
                 FROM agent_runtime_bindings
                 WHERE id = $1
                 FOR UPDATE",
                &[&binding_id],
            )
            .await
            .map_err(map_db_error)?;
        let current = binding_from_row(
            &row.ok_or_else(|| AppError::NotFound("runtime binding not found".into()))?,
        )?;

        if current.conversation_id != input.conversation_id
            || current.agent_principal_id != input.agent_principal_id
            || current.driver_type.as_str() != input.driver_type
            || current.workspace_path != input.workspace_path
            || current.updated_at != input.binding_updated_at
        {
            tx.rollback().await.map_err(map_db_error)?;
            return Err(AppError::NotFound("runtime binding context changed".into()));
        }

        let generation = current.terminal_generation().max(1);
        let mut config = current.config_json.as_object().cloned().unwrap_or_default();
        config.insert(
            "agent_workspace_id".into(),
            serde_json::json!(input.workspace_id.clone()),
        );
        config.insert(
            "conversation_workspace_id".into(),
            serde_json::json!(input.company_id.clone()),
        );
        config.insert("terminal_generation".into(), serde_json::json!(generation));
        config.insert(
            "terminal_capture".into(),
            serde_json::json!({
                "binding_id": input.binding_id,
                "conversation_id": input.conversation_id,
                "agent_principal_id": input.agent_principal_id,
                "company_id": input.company_id,
                "driver_type": input.driver_type,
                "workspace_id": input.workspace_id,
                "workspace_path": input.workspace_path,
                "native_home_path": input.native_home_path,
                "sessions_path": input.sessions_path,
                "binding_generation": generation,
                "spawn_started_at": input.spawn_started_at.to_rfc3339(),
                "baseline_session_files": input.baseline_session_files,
            }),
        );
        let config_json = Value::Object(config);
        let now = self.clock.now();
        let row = tx
            .query_one(
                "UPDATE agent_runtime_bindings
                 SET config_json = $2,
                     updated_at = $3
                 WHERE id = $1
                 RETURNING
                   id,
                   conversation_id,
                   agent_principal_id,
                   driver_type,
                   workspace_path,
                   git_worktree_path,
                   external_session_id,
                   external_thread_id,
                   last_event_cursor,
                   last_acked_event_cursor,
                   last_seen_server_seq,
                   state,
                   last_error,
                   in_flight_turn_id,
                   last_trigger_message_id,
                   config_json,
                   created_at,
                   updated_at",
                &[&binding_id, &config_json, &now],
            )
            .await
            .map_err(map_db_error)?;
        tx.commit().await.map_err(map_db_error)?;
        binding_from_row(&row)
    }

    pub async fn write_terminal_session_anchor(
        &self,
        binding_id: &str,
        input: TerminalSessionAnchorInput,
    ) -> AppResult<RuntimeBinding> {
        if binding_id != input.binding_id {
            return Err(AppError::Validation(
                "terminal session anchor binding mismatch".into(),
            ));
        }
        let client = self.connect().await?;
        let now = self.clock.now();
        let anchor = serde_json::json!({
            "driver_type": input.driver_type,
            "session_id": input.session_id,
            "source": input.source,
            "provenance": input.provenance,
            "binding_id": input.binding_id,
            "conversation_id": input.conversation_id,
            "agent_principal_id": input.agent_principal_id,
            "company_id": input.company_id,
            "workspace_id": input.workspace_id,
            "workspace_path": input.workspace_path,
            "native_home_path": input.native_home_path,
            "native_session_path": input.native_session_path,
            "binding_generation": input.binding_generation,
            "captured_at": now.to_rfc3339(),
            "last_verified_at": now.to_rfc3339(),
        });
        let row = client
            .query_opt(
                "UPDATE agent_runtime_bindings
                 SET config_json = (config_json - 'terminal_capture')
                       || jsonb_build_object('terminal_session', $2::jsonb),
                     updated_at = $3
                 WHERE id = $1
                   AND conversation_id = $4
                   AND agent_principal_id = $5
                   AND driver_type = $6
                   AND workspace_path = $7
                   AND updated_at = $8
                   AND COALESCE((config_json->>'terminal_generation')::bigint, 0) = $9
                   AND NOT EXISTS (
                     SELECT 1
                     FROM agent_runtime_bindings other
                     WHERE other.id != $1
                       AND other.config_json->'terminal_session'->>'session_id' = $10
                   )
                 RETURNING
                   id,
                   conversation_id,
                   agent_principal_id,
                   driver_type,
                   workspace_path,
                   git_worktree_path,
                   external_session_id,
                   external_thread_id,
                   last_event_cursor,
                   last_acked_event_cursor,
                   last_seen_server_seq,
                   state,
                   last_error,
                   in_flight_turn_id,
                   last_trigger_message_id,
                   config_json,
                   created_at,
                   updated_at",
                &[
                    &binding_id,
                    &anchor,
                    &now,
                    &input.conversation_id,
                    &input.agent_principal_id,
                    &input.driver_type,
                    &input.workspace_path,
                    &input.binding_updated_at,
                    &input.binding_generation,
                    &input.session_id,
                ],
            )
            .await
            .map_err(map_db_error)?;
        let row =
            row.ok_or_else(|| AppError::NotFound("runtime binding context changed".into()))?;
        binding_from_row(&row)
    }

    pub async fn connect(&self) -> AppResult<deadpool_postgres::Client> {
        self.pool
            .get()
            .await
            .map_err(|error| AppError::Internal(format!("agent runtime pool error: {error}")))
    }

    /// Run a lightweight `SELECT 1` query to verify database connectivity.
    pub async fn health_check(&self) -> AppResult<()> {
        let client = self.connect().await?;
        client
            .execute("SELECT 1", &[])
            .await
            .map_err(|error| AppError::Internal(format!("health check failed: {error}")))?;
        Ok(())
    }

    pub(crate) async fn fetch_binding(
        &self,
        client: &deadpool_postgres::Client,
        binding_id: &str,
    ) -> AppResult<RuntimeBinding> {
        let row = client
            .query_opt(
                "SELECT
                   id,
                   conversation_id,
                   agent_principal_id,
                   driver_type,
                   workspace_path,
                   git_worktree_path,
                   external_session_id,
                   external_thread_id,
                   last_event_cursor,
                   last_acked_event_cursor,
                   last_seen_server_seq,
                   state,
                   last_error,
                   in_flight_turn_id,
                   last_trigger_message_id,
                   config_json,
                   created_at,
                   updated_at
                 FROM agent_runtime_bindings
                 WHERE id = $1",
                &[&binding_id],
            )
            .await
            .map_err(map_db_error)?;
        let row = row.ok_or_else(|| AppError::NotFound("runtime binding not found".into()))?;
        binding_from_row(&row)
    }

    /// List all bindings for a given agent principal ID (any state).
    pub async fn list_bindings_by_agent(
        &self,
        agent_principal_id: &str,
    ) -> AppResult<Vec<RuntimeBinding>> {
        let client = self.connect().await?;
        let rows = client
            .query(
                "SELECT
                   id,
                   conversation_id,
                   agent_principal_id,
                   driver_type,
                   workspace_path,
                   git_worktree_path,
                   external_session_id,
                   external_thread_id,
                   last_event_cursor,
                   last_acked_event_cursor,
                   last_seen_server_seq,
                   state,
                   last_error,
                   in_flight_turn_id,
                   last_trigger_message_id,
                   config_json,
                   created_at,
                   updated_at
                 FROM agent_runtime_bindings
                 WHERE agent_principal_id = $1
                 ORDER BY created_at ASC",
                &[&agent_principal_id],
            )
            .await
            .map_err(map_db_error)?;
        rows.iter().map(binding_from_row).collect()
    }

    /// Find a binding for a specific agent in a specific conversation.
    ///
    /// Returns `Ok(None)` if no binding exists for this combination.
    pub async fn find_binding_by_agent_and_conversation(
        &self,
        agent_principal_id: &str,
        conversation_id: &str,
    ) -> AppResult<Option<RuntimeBinding>> {
        let client = self.connect().await?;
        let row = client
            .query_opt(
                "SELECT
                   id,
                   conversation_id,
                   agent_principal_id,
                   driver_type,
                   workspace_path,
                   git_worktree_path,
                   external_session_id,
                   external_thread_id,
                   last_event_cursor,
                   last_acked_event_cursor,
                   last_seen_server_seq,
                   state,
                   last_error,
                   in_flight_turn_id,
                   last_trigger_message_id,
                   config_json,
                   created_at,
                   updated_at
                 FROM agent_runtime_bindings
                 WHERE agent_principal_id = $1
                   AND conversation_id = $2
                 LIMIT 1",
                &[&agent_principal_id, &conversation_id],
            )
            .await
            .map_err(map_db_error)?;
        match row {
            Some(r) => Ok(Some(binding_from_row(&r)?)),
            None => Ok(None),
        }
    }

    /// Find the primary binding for an agent (the first non-proxy binding).
    ///
    /// Checks for `(config_json->>'is_primary')::boolean = true` first,
    /// then falls back to the earliest created binding.
    pub async fn find_primary_binding_for_agent(
        &self,
        agent_principal_id: &str,
    ) -> AppResult<Option<RuntimeBinding>> {
        let client = self.connect().await?;
        // Try explicit is_primary = true first
        let row = client
            .query_opt(
                "SELECT
                   id,
                   conversation_id,
                   agent_principal_id,
                   driver_type,
                   workspace_path,
                   git_worktree_path,
                   external_session_id,
                   external_thread_id,
                   last_event_cursor,
                   last_acked_event_cursor,
                   last_seen_server_seq,
                   state,
                   last_error,
                   in_flight_turn_id,
                   last_trigger_message_id,
                   config_json,
                   created_at,
                   updated_at
                 FROM agent_runtime_bindings
                 WHERE agent_principal_id = $1
                   AND (config_json->>'is_primary')::boolean = true
                 LIMIT 1",
                &[&agent_principal_id],
            )
            .await
            .map_err(map_db_error)?;
        if let Some(r) = row {
            return Ok(Some(binding_from_row(&r)?));
        }
        // Fallback: earliest created binding for this agent
        let row = client
            .query_opt(
                "SELECT
                   id,
                   conversation_id,
                   agent_principal_id,
                   driver_type,
                   workspace_path,
                   git_worktree_path,
                   external_session_id,
                   external_thread_id,
                   last_event_cursor,
                   last_acked_event_cursor,
                   last_seen_server_seq,
                   state,
                   last_error,
                   in_flight_turn_id,
                   last_trigger_message_id,
                   config_json,
                   created_at,
                   updated_at
                 FROM agent_runtime_bindings
                 WHERE agent_principal_id = $1
                 ORDER BY created_at ASC
                 LIMIT 1",
                &[&agent_principal_id],
            )
            .await
            .map_err(map_db_error)?;
        match row {
            Some(r) => Ok(Some(binding_from_row(&r)?)),
            None => Ok(None),
        }
    }

    /// Remove a git worktree associated with a binding.
    ///
    /// Calls `git worktree remove --force <path>`. If the binding has no
    /// `git_worktree_path`, this is a no-op.
    pub async fn cleanup_worktree(&self, binding: &RuntimeBinding) -> AppResult<()> {
        if let Some(worktree_path) = &binding.git_worktree_path {
            let output = tokio::process::Command::new("git")
                .args(["worktree", "remove", "--force", worktree_path])
                .output()
                .await;
            match output {
                Ok(o) if o.status.success() => {
                    tracing::info!(path = %worktree_path, "removed git worktree");
                }
                Ok(o) => {
                    tracing::warn!(
                        path = %worktree_path,
                        stderr = %String::from_utf8_lossy(&o.stderr),
                        "failed to remove git worktree"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        path = %worktree_path,
                        error = %e,
                        "failed to execute git worktree remove"
                    );
                }
            }
        }
        Ok(())
    }

    pub(crate) async fn record_audit(
        &self,
        client: &deadpool_postgres::Client,
        actor: &AuditActor,
        action: &str,
        target_type: &str,
        target_id: &str,
        metadata: Value,
    ) -> AppResult<()> {
        client
            .execute(
                "INSERT INTO principal (
                   id,
                   workspace_id,
                   type,
                   name,
                   created_at,
                   updated_at
                 ) VALUES ($1, $2, 'human', $3, $4, $4)
                 ON CONFLICT (id) DO UPDATE
                 SET workspace_id = EXCLUDED.workspace_id,
                     updated_at = EXCLUDED.updated_at",
                &[
                    &actor.actor_id,
                    &actor.workspace_id,
                    &actor.actor_id,
                    &self.clock.now(),
                ],
            )
            .await
            .map_err(map_db_error)?;
        client
            .execute(
                "INSERT INTO audit_log (
                   id,
                   workspace_id,
                   actor_id,
                   action,
                   target_type,
                   target_id,
                   metadata,
                   created_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[
                    &new_id(),
                    &actor.workspace_id,
                    &actor.actor_id,
                    &action,
                    &target_type,
                    &target_id,
                    &metadata,
                    &self.clock.now(),
                ],
            )
            .await
            .map_err(map_db_error)?;
        Ok(())
    }

    /// Return the distinct agent principal IDs for all non-disabled bindings.
    ///
    /// Used by the per-agent consumer on startup to know which agents it should
    /// spin up inbox consumers for.
    pub async fn list_active_agent_ids(&self) -> Result<Vec<String>, AppError> {
        let client = self.connect().await?;
        let rows = client
            .query(
                "SELECT DISTINCT agent_principal_id FROM agent_runtime_bindings WHERE state != 'disabled'",
                &[],
            )
            .await
            .map_err(|e| AppError::Internal(format!("list_active_agent_ids: {e}")))?;
        Ok(rows
            .into_iter()
            .map(|r| r.get("agent_principal_id"))
            .collect())
    }

    /// Scan a CLI's trusted on-disk session store for the latest session
    /// matching this binding's workspace, and write it to
    /// `external_session_id`. Returns the session_id found.
    ///
    /// Always overwrites the existing value when a fresher session is found
    /// on disk. Caller decides whether to call (e.g. only when current value
    /// is empty, or unconditionally on PTY disconnect).
    ///
    /// Codex is intentionally excluded: its global session files do not prove
    /// which Choruz binding created a session, so Codex IDs are only accepted
    /// when captured from a process Choruz launched for that exact binding.
    ///
    /// Why this exists: runner used to write `external_session_id` per turn
    /// (`runner/processor.rs::update_binding_runtime_session`). Runner was
    /// disabled 2026-04-02 (commit 32a230b), and pipeline's session-writeback
    /// in `executor.rs:1023` only fires for headless mode. PTY sessions have
    /// no pipeline writeback path, so binding rows stay NULL forever and
    /// resume flags can never fire. This method bridges that gap using each
    /// CLI's workspace-scoped session registry.
    pub async fn sync_session_id_from_disk(&self, binding_id: &str) -> AppResult<Option<String>> {
        let client = self.connect().await?;

        let row = client
            .query_opt(
                "SELECT workspace_path, driver_type, updated_at
                 FROM agent_runtime_bindings WHERE id = $1",
                &[&binding_id],
            )
            .await
            .map_err(map_db_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let workspace_path: String = row.get("workspace_path");
        let driver_type: String = row.get("driver_type");
        let binding_updated_at: chrono::DateTime<chrono::Utc> = row.get("updated_at");
        drop(client);

        let session_id = find_latest_session_for_driver(&workspace_path, &driver_type).await;
        let Some(session_id) = session_id else {
            return Ok(None);
        };

        let client = self.connect().await?;
        let updated = client
            .execute(
                "UPDATE agent_runtime_bindings
                 SET external_session_id = $1,
                     config_json = config_json
                       - 'external_session_provenance'
                       - 'external_session_driver_type'
                       - 'external_session_binding_id'
                       - 'external_session_mode'
                       - 'external_session_captured_at',
                     updated_at = NOW()
                 WHERE id = $2
                   AND workspace_path = $3
                   AND driver_type = $4
                   AND updated_at = $5",
                &[
                    &session_id,
                    &binding_id,
                    &workspace_path,
                    &driver_type,
                    &binding_updated_at,
                ],
            )
            .await
            .map_err(map_db_error)?;
        if updated == 0 {
            tracing::info!(
                binding_id,
                "discarded discovered session because binding identity changed"
            );
            return Ok(None);
        }
        Ok(Some(session_id))
    }

    pub async fn backfill_session_ids(&self) -> AppResult<u64> {
        let client = self.connect().await?;
        let rows = client
            .query(
                "SELECT id, workspace_path, driver_type, updated_at FROM agent_runtime_bindings
                 WHERE state NOT IN ('disabled')
                   AND (external_session_id IS NULL OR external_session_id = '')",
                &[],
            )
            .await
            .map_err(map_db_error)?;
        drop(client);

        let mut updated = 0u64;
        for row in &rows {
            let binding_id: String = row.get("id");
            let workspace_path: String = row.get("workspace_path");
            let driver_type: String = row.get("driver_type");
            let binding_updated_at: chrono::DateTime<chrono::Utc> = row.get("updated_at");

            let session_id =
                match find_latest_session_for_driver(&workspace_path, &driver_type).await {
                    Some(sid) => sid,
                    None => continue,
                };

            let client = self.connect().await?;
            let n = client
                .execute(
                    "UPDATE agent_runtime_bindings
                     SET external_session_id = $1,
                         config_json = config_json
                           - 'external_session_provenance'
                           - 'external_session_driver_type'
                           - 'external_session_binding_id'
                           - 'external_session_mode'
                           - 'external_session_captured_at',
                         updated_at = NOW()
                     WHERE id = $2
                       AND (external_session_id IS NULL OR external_session_id = '')
                       AND workspace_path = $3
                       AND driver_type = $4
                       AND updated_at = $5",
                    &[
                        &session_id,
                        &binding_id,
                        &workspace_path,
                        &driver_type,
                        &binding_updated_at,
                    ],
                )
                .await
                .map_err(map_db_error)?;
            if n > 0 {
                tracing::info!(binding_id, session_id, workspace = %workspace_path, "backfilled session ID from disk");
                updated += 1;
            }
        }
        Ok(updated)
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

fn binding_transition_audit_action(current: &BindingState, next: &BindingState) -> &'static str {
    match (current, next) {
        (_, BindingState::Paused) => "runtime.binding_paused",
        (BindingState::Paused, BindingState::Idle) => "runtime.binding_resumed",
        _ => "runtime.binding_state_changed",
    }
}

fn binding_from_row(row: &Row) -> AppResult<RuntimeBinding> {
    Ok(RuntimeBinding {
        id: row.get("id"),
        conversation_id: row.get("conversation_id"),
        agent_principal_id: row.get("agent_principal_id"),
        driver_type: DriverType::from_str(row.get::<_, &str>("driver_type"))?,
        workspace_path: row.get("workspace_path"),
        git_worktree_path: row.get("git_worktree_path"),
        external_session_id: row.get("external_session_id"),
        external_thread_id: row.get("external_thread_id"),
        last_event_cursor: row.get("last_event_cursor"),
        last_acked_event_cursor: row.get("last_acked_event_cursor"),
        last_seen_server_seq: row.get("last_seen_server_seq"),
        state: BindingState::from_str(row.get::<_, &str>("state"))?,
        last_error: row.get("last_error"),
        in_flight_turn_id: row.get("in_flight_turn_id"),
        last_trigger_message_id: row.get("last_trigger_message_id"),
        config_json: row.get("config_json"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

pub(crate) fn map_db_error(error: PgError) -> AppError {
    match error.code() {
        Some(&SqlState::UNIQUE_VIOLATION) => {
            AppError::Conflict("runtime binding already exists for conversation and agent".into())
        }
        _ => {
            // Include Debug form too — Display on tokio_postgres::Error often just says "db error"
            // while Debug includes the underlying io::Error / SQL state / source chain.
            tracing::error!(
                error = %error,
                error_debug = ?error,
                "agent runtime postgres error",
            );
            AppError::Internal(format!("agent runtime postgres error: {error:?}"))
        }
    }
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

async fn find_latest_session_for_driver(workspace_path: &str, driver_type: &str) -> Option<String> {
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

    // ── binding_transition_audit_action ───────────────────────────────

    #[test]
    fn audit_action_paused() {
        assert_eq!(
            binding_transition_audit_action(&BindingState::Idle, &BindingState::Paused),
            "runtime.binding_paused"
        );
    }

    #[test]
    fn audit_action_resumed() {
        assert_eq!(
            binding_transition_audit_action(&BindingState::Paused, &BindingState::Idle),
            "runtime.binding_resumed"
        );
    }

    #[test]
    fn audit_action_generic() {
        assert_eq!(
            binding_transition_audit_action(&BindingState::Idle, &BindingState::Running),
            "runtime.binding_state_changed"
        );
    }

    #[test]
    fn audit_action_to_paused_from_running() {
        assert_eq!(
            binding_transition_audit_action(&BindingState::Running, &BindingState::Paused),
            "runtime.binding_paused"
        );
    }

    #[test]
    fn audit_action_error_to_disabled() {
        assert_eq!(
            binding_transition_audit_action(&BindingState::Error, &BindingState::Disabled),
            "runtime.binding_state_changed"
        );
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
