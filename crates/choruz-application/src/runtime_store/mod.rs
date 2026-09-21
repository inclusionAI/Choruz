//! PostgreSQL binding and policy persistence for the Choruz platform.
mod policy;
pub use policy::{AutoMode, ConversationRuntimePolicy, UntaggedHumanMode, UpsertPolicyInput};

use choruz_agent_runtime::{
    AuditActor, BindingState, CodexTerminalCaptureInput, CreateBindingInput, RuntimeBinding,
    TerminalSessionAnchorInput, latest_native_session, normalize_workspace_path,
};
use choruz_common::{AppError, AppResult, new_id};
use chrono::{DateTime, Utc};
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use serde_json::Value;
use std::{fmt, sync::Arc};
use tokio_postgres::{Error as PgError, Row, error::SqlState};

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

#[derive(Debug, Clone)]
pub struct SessionSyncTarget {
    pub workspace_path: String,
    pub driver_type: String,
    pub binding_updated_at: chrono::DateTime<chrono::Utc>,
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
        self.write_session_anchor(binding_id, input, None).await
    }

    /// Direct sessions may race a headless completion, but not a change to
    /// their account, device, generation, or previously captured identity.
    pub async fn write_direct_session_anchor(
        &self,
        binding: &RuntimeBinding,
        input: TerminalSessionAnchorInput,
    ) -> AppResult<RuntimeBinding> {
        self.write_session_anchor(&binding.id, input, Some(&binding.config_json))
            .await
    }

    async fn write_session_anchor(
        &self,
        binding_id: &str,
        input: TerminalSessionAnchorInput,
        expected_config: Option<&Value>,
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
                       || jsonb_build_object('terminal_session', $2::jsonb
                            || jsonb_build_object('runtime_host_id', config_json->'runtime_host_id')),
                     updated_at = $3
                 WHERE id = $1
                   AND conversation_id = $4
                   AND agent_principal_id = $5
                   AND driver_type = $6
                   AND workspace_path = $7
                   AND CASE WHEN $11::jsonb IS NULL THEN updated_at = $8 ELSE (
                     config_json - ARRAY['external_session_provenance',
                       'external_session_driver_type', 'external_session_binding_id',
                       'external_session_mode', 'external_session_captured_at',
                       'agent_workspace_id', 'conversation_workspace_id']::text[]
                     = $11::jsonb - ARRAY['external_session_provenance',
                       'external_session_driver_type', 'external_session_binding_id',
                       'external_session_mode', 'external_session_captured_at',
                       'agent_workspace_id', 'conversation_workspace_id']::text[]
                     OR (
                       config_json - ARRAY['terminal_session',
                         'external_session_provenance', 'external_session_driver_type',
                         'external_session_binding_id', 'external_session_mode',
                         'external_session_captured_at', 'agent_workspace_id',
                         'conversation_workspace_id']::text[]
                       = $11::jsonb - ARRAY['terminal_session',
                         'external_session_provenance', 'external_session_driver_type',
                         'external_session_binding_id', 'external_session_mode',
                         'external_session_captured_at', 'agent_workspace_id',
                         'conversation_workspace_id']::text[]
                       AND (config_json->'terminal_session')
                         - ARRAY['captured_at', 'last_verified_at', 'runtime_host_id']::text[]
                         = $2::jsonb - ARRAY['captured_at', 'last_verified_at']::text[]
                       AND config_json->'terminal_session'->'runtime_host_id'
                         IS NOT DISTINCT FROM COALESCE(config_json->'runtime_host_id', 'null'::jsonb)
                     ))
                   END
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
                    &expected_config,
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
    /// The binding identity a discovered native session is recorded against;
    /// `None` when the binding no longer exists.
    pub async fn session_sync_target(
        &self,
        binding_id: &str,
    ) -> AppResult<Option<SessionSyncTarget>> {
        let client = self.connect().await?;
        let row = client
            .query_opt(
                "SELECT workspace_path, driver_type, updated_at
                 FROM agent_runtime_bindings WHERE id = $1",
                &[&binding_id],
            )
            .await
            .map_err(map_db_error)?;
        Ok(row.map(|row| SessionSyncTarget {
            workspace_path: row.get("workspace_path"),
            driver_type: row.get("driver_type"),
            binding_updated_at: row.get("updated_at"),
        }))
    }

    /// Store a native session id the device found for `target`; returns false
    /// when the binding identity changed since the target was read.
    pub async fn record_discovered_session_id(
        &self,
        binding_id: &str,
        target: &SessionSyncTarget,
        session_id: &str,
    ) -> AppResult<bool> {
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
                    &target.workspace_path,
                    &target.driver_type,
                    &target.binding_updated_at,
                ],
            )
            .await
            .map_err(map_db_error)?;
        if updated == 0 {
            tracing::info!(
                binding_id,
                "discarded discovered session because binding identity changed"
            );
        }
        Ok(updated > 0)
    }

    /// Discover the newest native session for a binding on this device and
    /// record it.
    pub async fn sync_session_id_from_disk(&self, binding_id: &str) -> AppResult<Option<String>> {
        let Some(target) = self.session_sync_target(binding_id).await? else {
            return Ok(None);
        };
        let Some(session_id) =
            latest_native_session(&target.workspace_path, &target.driver_type).await
        else {
            return Ok(None);
        };
        if self
            .record_discovered_session_id(binding_id, &target, &session_id)
            .await?
        {
            Ok(Some(session_id))
        } else {
            Ok(None)
        }
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

            let session_id = match latest_native_session(&workspace_path, &driver_type).await {
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

fn binding_state_from_str(value: &str) -> AppResult<BindingState> {
    match value {
        "idle" => Ok(BindingState::Idle),
        "running" => Ok(BindingState::Running),
        "paused" => Ok(BindingState::Paused),
        "disabled" => Ok(BindingState::Disabled),
        "error" => Ok(BindingState::Error),
        other => Err(AppError::Internal(format!(
            "unknown binding state from database: {other}"
        ))),
    }
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
        driver_type: row.get::<_, &str>("driver_type").parse()?,
        workspace_path: row.get("workspace_path"),
        git_worktree_path: row.get("git_worktree_path"),
        external_session_id: row.get("external_session_id"),
        external_thread_id: row.get("external_thread_id"),
        last_event_cursor: row.get("last_event_cursor"),
        last_acked_event_cursor: row.get("last_acked_event_cursor"),
        last_seen_server_seq: row.get("last_seen_server_seq"),
        state: binding_state_from_str(row.get::<_, &str>("state"))?,
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

#[cfg(test)]
mod tests {
    use super::*;
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
}
