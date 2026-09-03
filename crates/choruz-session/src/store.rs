//! PostgreSQL-backed session store.
//!
//! Implements all CRUD operations for `session_registry`, `agent_commands`,
//! `dead_letters`, and the in-memory executor registry.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use tokio::sync::RwLock;
use tokio_postgres::Row;
use uuid::Uuid;

use crate::error::{SessionError, SessionResult};
use crate::models::*;
use crate::retry;

// ---------------------------------------------------------------------------
// PgSessionStore
// ---------------------------------------------------------------------------

/// Database-backed session store for all Session Manager operations.
#[derive(Clone)]
pub struct PgSessionStore {
    pool: Pool,
    executors: Arc<RwLock<HashMap<String, Executor>>>,
}

impl PgSessionStore {
    /// Create a new store with a connection pool built from a libpq-style
    /// connection string (e.g. `host=127.0.0.1 port=5432 user=choruz dbname=choruz`).
    pub fn new(database_url: &str) -> Self {
        Self {
            pool: build_pool(database_url),
            executors: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a store from an existing deadpool `Pool` (useful for tests).
    pub fn from_pool(pool: Pool) -> Self {
        Self {
            pool,
            executors: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn connect(&self) -> SessionResult<deadpool_postgres::Client> {
        self.pool
            .get()
            .await
            .map_err(|e| SessionError::Database(format!("pool error: {e}")))
    }

    // =======================================================================
    // D1: session_registry CRUD
    // =======================================================================

    /// Insert or update a session. If the session_key already exists, this is
    /// a no-op (the existing row is returned).
    pub async fn upsert_session(
        &self,
        session_key: &str,
        agent_id: &str,
        conversation_id: &str,
    ) -> SessionResult<Session> {
        let client = self.connect().await?;
        let now = Utc::now();
        let row = client
            .query_one(
                "INSERT INTO session_registry (
                    session_key, agent_id, conversation_id,
                    epoch, status, created_at, updated_at
                 ) VALUES ($1, $2, $3, 0, 'idle', $4, $4)
                 ON CONFLICT (session_key) DO UPDATE
                    SET updated_at = $4
                 RETURNING
                    session_key, agent_id, conversation_id,
                    executor_node_id, epoch, status,
                    workspace_snapshot_id, memory_context_id,
                    last_heartbeat_at, created_at, updated_at",
                &[&session_key, &agent_id, &conversation_id, &now],
            )
            .await?;
        Ok(session_from_row(&row))
    }

    /// Get a session by its key.
    pub async fn get_session(&self, session_key: &str) -> SessionResult<Option<Session>> {
        let client = self.connect().await?;
        let row = client
            .query_opt(
                "SELECT
                    session_key, agent_id, conversation_id,
                    executor_node_id, epoch, status,
                    workspace_snapshot_id, memory_context_id,
                    last_heartbeat_at, created_at, updated_at
                 FROM session_registry
                 WHERE session_key = $1",
                &[&session_key],
            )
            .await?;
        Ok(row.map(|r| session_from_row(&r)))
    }

    /// Apply a partial update to a session.
    pub async fn update_session(&self, update: &SessionUpdate) -> SessionResult<()> {
        let client = self.connect().await?;
        let now = Utc::now();

        // Build dynamic SET clause
        let mut sets: Vec<String> = vec!["updated_at = $2".to_string()];
        let mut param_idx = 3u32;

        // We use a simple approach: always set all optional fields if provided.
        // For Option<Option<T>>, Some(None) means "SET to NULL", Some(Some(v))
        // means "SET to v", None means "don't change".

        // Collect params as trait objects to pass to query.
        // Since tokio-postgres needs &(dyn ToSql + Sync), we build the query
        // string and params vector together.

        // We'll use a straightforward SQL UPDATE with COALESCE-style approach
        // but for clarity, just build the full UPDATE.

        let status_str = update.status.map(|s| s.as_str().to_string());

        if update.executor_node_id.is_some() {
            sets.push(format!("executor_node_id = ${param_idx}"));
            param_idx += 1;
        }
        if update.status.is_some() {
            sets.push(format!("status = ${param_idx}"));
            param_idx += 1;
        }
        if update.last_heartbeat_at.is_some() {
            sets.push(format!("last_heartbeat_at = ${param_idx}"));
            let _ = param_idx; // suppress unused warning on last
        }

        let sql = format!(
            "UPDATE session_registry SET {} WHERE session_key = $1",
            sets.join(", ")
        );

        // Build params dynamically. This is a bit verbose but type-safe.
        let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> = Vec::new();
        params.push(Box::new(update.session_key.clone()));
        params.push(Box::new(now));

        if let Some(ref exec_node) = update.executor_node_id {
            params.push(Box::new(exec_node.clone()));
        }
        if let Some(ref status) = status_str {
            params.push(Box::new(status.clone()));
        }
        if let Some(ref hb) = update.last_heartbeat_at {
            params.push(Box::new(*hb));
        }

        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let affected = client.execute(&sql, &param_refs).await?;
        if affected == 0 {
            return Err(SessionError::SessionNotFound(update.session_key.clone()));
        }
        Ok(())
    }

    /// Refresh a heartbeat only while the caller's lease epoch is current.
    pub async fn update_session_heartbeat_for_epoch(
        &self,
        session_key: &str,
        expected_epoch: i32,
    ) -> SessionResult<()> {
        let mut client = self.connect().await?;
        let tx = client.transaction().await?;
        let now = Utc::now();
        let row = tx
            .query_opt(
                "SELECT epoch, status FROM session_registry
                 WHERE session_key = $1 FOR UPDATE",
                &[&session_key],
            )
            .await?;
        let Some(row) = row else {
            tx.rollback().await?;
            return Err(SessionError::SessionNotFound(session_key.to_string()));
        };
        let actual: i32 = row.get("epoch");
        let status: String = row.get("status");
        if actual != expected_epoch {
            tx.rollback().await?;
            return Err(SessionError::EpochMismatch {
                expected: expected_epoch,
                actual,
            });
        }
        if status != "active" {
            tx.rollback().await?;
            return Err(SessionError::SessionInactive {
                session_key: session_key.to_string(),
                status,
            });
        }
        tx.execute(
            "UPDATE session_registry
             SET last_heartbeat_at = $2, updated_at = $2
             WHERE session_key = $1",
            &[&session_key, &now],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    // =======================================================================
    // D2: agent_commands state machine operations
    // =======================================================================

    /// Insert a new agent command in `pending` status.
    pub async fn insert_command(&self, input: &InsertCommand) -> SessionResult<AgentCommand> {
        let mut client = self.connect().await?;
        let tx = client.transaction().await?;
        let now = Utc::now();

        let idempotency_key = format!("{}:{}", input.message_id, input.agent_id);
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtext($1)::bigint)",
            &[&input.agent_id],
        )
        .await?;
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtext($1)::bigint)",
            &[&idempotency_key],
        )
        .await?;

        if let Some(row) = tx
            .query_opt(
                "SELECT
                    command_id, route_id, session_key, agent_id,
                    conversation_id, message_id, turn_id,
                    status, current_attempt_id, current_epoch,
                    attempt_count, max_attempts,
                    prompt, metadata, next_retry_at, last_error,
                    created_at, updated_at
                 FROM agent_commands
                 WHERE message_id = $1 AND agent_id = $2",
                &[&input.message_id, &input.agent_id],
            )
            .await?
        {
            tx.commit().await?;
            return Ok(command_from_row(&row));
        }

        let metadata = runtime_host_metadata(&tx, &input.agent_id, &input.metadata).await?;
        let row = tx
            .query_one(
                "INSERT INTO agent_commands (
                    command_id, route_id, session_key, agent_id,
                    conversation_id, message_id, turn_id,
                    status, attempt_count, max_attempts,
                    prompt, metadata, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7,
                           'pending', 0, $8, $9, $10, $11, $11)
                 RETURNING
                    command_id, route_id, session_key, agent_id,
                    conversation_id, message_id, turn_id,
                    status, current_attempt_id, current_epoch,
                    attempt_count, max_attempts,
                    prompt, metadata, next_retry_at, last_error,
                    created_at, updated_at",
                &[
                    &input.command_id,
                    &input.route_id,
                    &input.session_key,
                    &input.agent_id,
                    &input.conversation_id,
                    &input.message_id,
                    &input.turn_id,
                    &input.max_attempts,
                    &input.prompt,
                    &metadata,
                    &now,
                ],
            )
            .await?;
        tx.commit().await?;
        Ok(command_from_row(&row))
    }

    /// Find the currently active command for a session (status in leased,
    /// started, heartbeating).
    pub async fn find_active_command_for_session(
        &self,
        session_key: &str,
    ) -> SessionResult<Option<AgentCommand>> {
        let client = self.connect().await?;
        let row = client
            .query_opt(
                "SELECT
                    command_id, route_id, session_key, agent_id,
                    conversation_id, message_id, turn_id,
                    status, current_attempt_id, current_epoch,
                    attempt_count, max_attempts,
                    prompt, metadata, next_retry_at, last_error,
                    created_at, updated_at
                 FROM agent_commands
                 WHERE session_key = $1
                   AND status IN ('leased', 'started', 'heartbeating')
                 ORDER BY created_at DESC
                 LIMIT 1",
                &[&session_key],
            )
            .await?;
        Ok(row.map(|r| command_from_row(&r)))
    }

    /// Find the oldest active command for a conversation/agent pair.
    pub async fn find_active_command_for_agent(
        &self,
        conversation_id: &str,
        agent_id: &str,
    ) -> SessionResult<Option<AgentCommand>> {
        let client = self.connect().await?;
        let row = client
            .query_opt(
                "SELECT
                    command_id, route_id, session_key, agent_id,
                    conversation_id, message_id, turn_id,
                    status, current_attempt_id, current_epoch,
                    attempt_count, max_attempts,
                    prompt, metadata, next_retry_at, last_error,
                    created_at, updated_at
                 FROM agent_commands
                 WHERE conversation_id = $1
                   AND agent_id = $2
                   AND status IN ('leased', 'started', 'heartbeating')
                 ORDER BY created_at ASC, command_id ASC
                 LIMIT 1",
                &[&conversation_id, &agent_id],
            )
            .await?;
        Ok(row.map(|r| command_from_row(&r)))
    }

    /// Count queued commands for a conversation/agent pair.
    pub async fn count_queued_commands_for_agent(
        &self,
        conversation_id: &str,
        agent_id: &str,
    ) -> SessionResult<i64> {
        let client = self.connect().await?;
        let row = client
            .query_one(
                "SELECT COUNT(*)
                 FROM agent_commands
                 WHERE conversation_id = $1
                   AND agent_id = $2
                   AND status IN ('pending', 'retry_scheduled')",
                &[&conversation_id, &agent_id],
            )
            .await?;
        Ok(row.get::<_, i64>(0))
    }

    /// List derived runtime status for multiple agents in one conversation.
    pub async fn list_runtime_status_for_agents(
        &self,
        conversation_id: &str,
        agent_ids: &[String],
        now: DateTime<Utc>,
    ) -> SessionResult<Vec<AgentRuntimeStatus>> {
        if agent_ids.is_empty() {
            return Ok(Vec::new());
        }

        let client = self.connect().await?;
        let agent_ids = agent_ids.to_vec();

        let active_rows = client
            .query(
                "SELECT DISTINCT ON (agent_id)
                    command_id, route_id, session_key, agent_id,
                    conversation_id, message_id, turn_id,
                    status, current_attempt_id, current_epoch,
                    attempt_count, max_attempts,
                    prompt, metadata, next_retry_at, last_error,
                    created_at, updated_at
                 FROM agent_commands
                 WHERE conversation_id = $1
                   AND agent_id = ANY($2)
                   AND status IN ('leased', 'started', 'heartbeating')
                 ORDER BY agent_id, created_at ASC, command_id ASC",
                &[&conversation_id, &agent_ids],
            )
            .await?;
        let active_by_agent: HashMap<String, AgentCommand> = active_rows
            .iter()
            .map(|row| {
                let command = command_from_row(row);
                (command.agent_id.clone(), command)
            })
            .collect();

        let queued_rows = client
            .query(
                "SELECT agent_id, COUNT(*) AS queued_count
                 FROM agent_commands
                 WHERE conversation_id = $1
                   AND agent_id = ANY($2)
                   AND status IN ('pending', 'retry_scheduled')
                 GROUP BY agent_id",
                &[&conversation_id, &agent_ids],
            )
            .await?;
        let queued_count_by_agent: HashMap<String, i64> = queued_rows
            .iter()
            .map(|row| (row.get("agent_id"), row.get("queued_count")))
            .collect();

        let error_rows = client
            .query(
                "SELECT DISTINCT ON (agent_id)
                    agent_id, last_error
                 FROM agent_commands
                 WHERE conversation_id = $1
                   AND agent_id = ANY($2)
                   AND last_error IS NOT NULL
                 ORDER BY agent_id, updated_at DESC, created_at DESC, command_id DESC",
                &[&conversation_id, &agent_ids],
            )
            .await?;
        let last_error_by_agent: HashMap<String, String> = error_rows
            .iter()
            .map(|row| (row.get("agent_id"), row.get("last_error")))
            .collect();

        Ok(agent_ids
            .iter()
            .map(|agent_id| {
                let active_command = active_by_agent
                    .get(agent_id)
                    .map(|command| runtime_status_command_from_command(command, now));
                let queued_count = *queued_count_by_agent.get(agent_id).unwrap_or(&0);
                let status = if active_command.is_some() {
                    "busy"
                } else if queued_count > 0 {
                    "queued"
                } else {
                    "idle"
                }
                .to_string();
                let last_error = active_command
                    .as_ref()
                    .and_then(|command| command.last_error.clone())
                    .or_else(|| last_error_by_agent.get(agent_id).cloned());

                AgentRuntimeStatus {
                    conversation_id: conversation_id.to_string(),
                    agent_id: agent_id.clone(),
                    status,
                    active_command,
                    queued_count,
                    last_error,
                }
            })
            .collect())
    }

    /// Update a command's status and related fields.
    pub async fn update_command_status(&self, update: &CommandStatusUpdate) -> SessionResult<()> {
        self.update_command_status_inner(update, None).await
    }

    /// Update a command only when the caller still owns its current attempt.
    pub async fn update_command_status_for_attempt(
        &self,
        update: &CommandStatusUpdate,
        expected_attempt_id: &str,
    ) -> SessionResult<()> {
        self.update_command_status_inner(update, Some(expected_attempt_id))
            .await
    }

    async fn update_command_status_inner(
        &self,
        update: &CommandStatusUpdate,
        expected_attempt_id: Option<&str>,
    ) -> SessionResult<()> {
        let client = self.connect().await?;
        let now = Utc::now();
        let status_str = update.status.as_str().to_string();
        let expected_attempt_id = expected_attempt_id.map(str::to_owned);

        let affected = client
            .execute(
                "UPDATE agent_commands
                 SET status = $2,
                     current_attempt_id = COALESCE($3, current_attempt_id),
                     current_epoch = COALESCE($4, current_epoch),
                     attempt_count = COALESCE($5, attempt_count),
                     next_retry_at = CASE WHEN $6 THEN $7 ELSE next_retry_at END,
                     last_error = COALESCE($8, last_error),
                     updated_at = $9
                 WHERE command_id = $1
                   AND (
                       $10::text IS NULL OR (
                           current_attempt_id = $10
                           AND current_epoch = (
                               SELECT epoch FROM session_registry
                               WHERE session_key = agent_commands.session_key
                           )
                       )
                   )",
                &[
                    &update.command_id,
                    &status_str,
                    &update.current_attempt_id,
                    &update.current_epoch,
                    &update.attempt_count,
                    &update.next_retry_at.is_some(),
                    &update.next_retry_at.as_ref().and_then(|o| o.as_ref()),
                    &update.last_error,
                    &now,
                    &expected_attempt_id,
                ],
            )
            .await?;
        if affected == 0 {
            return match expected_attempt_id {
                Some(attempt_id) => Err(SessionError::StaleAttempt {
                    command_id: update.command_id.clone(),
                    attempt_id,
                }),
                None => Err(SessionError::CommandNotFound(update.command_id.clone())),
            };
        }
        Ok(())
    }

    /// Find commands in `pending` status (not yet dispatched), fairly ordered
    /// across agents while preserving FIFO order inside each agent's queue.
    ///
    /// Coalescer (per-agent): skip a pending command when there is already a
    /// command in a non-terminal active or queued state for the SAME AGENT —
    /// regardless of conversation. This caps concurrent `claude --print`
    /// spawns at 1 per agent so the OS process count stays bounded under
    /// bursty traffic. Without it, 13 agents × hundreds of group messages had
    /// produced 400+ simultaneously-live child processes that pinned the
    /// host's RAM (~100-200 GB).
    ///
    /// Per-agent scope (rather than per-(agent, conv)) intentionally matches
    /// the runtime-binding model: `agent_runtime_bindings` stores one
    /// `external_session_id` per agent (migration 0018 enforces "at most one
    /// active binding per agent_principal_id"), so resume / history is
    /// already workspace-scoped — letting two spawns race the same session
    /// would corrupt that history with last-write-wins on session-id update
    /// and interleave messages from different groups into one claude thread.
    /// One agent → one in-flight spawn → one session writer at a time.
    ///
    /// Cross-conv replies for the same agent now serialise: A's @-mention in
    /// group2 waits for A's reply in group1 to finish. Acceptable because
    /// the bottleneck is the LLM round-trip, not the dispatcher.
    ///
    /// The blocking statuses cover both "actively executing" (leased,
    /// started, heartbeating) and "queued for retry" (retry_scheduled).
    /// retry_scheduled is included so a backed-off failure doesn't get
    /// lapped by a fresher pending command.
    ///
    /// Eligible rows receive a 1-based position inside their agent's FIFO.
    /// Ordering by that position before creation time gives every idle agent's
    /// oldest command a dispatch slot before a hot agent's second command can
    /// consume one. A single agent can still fill the batch, so burst
    /// coalescing remains intact when there is no competing work.
    ///
    /// The active-command partial index from migration V016 keeps the
    /// `NOT EXISTS` lookup cheap; migration V022 adds the pending FIFO index
    /// used by the window ordering.
    pub async fn find_pending_commands(&self, limit: i64) -> SessionResult<Vec<AgentCommand>> {
        let client = self.connect().await?;
        let rows = client
            .query(
                "WITH eligible AS (
                     SELECT
                         c.command_id, c.route_id, c.session_key, c.agent_id,
                         c.conversation_id, c.message_id, c.turn_id,
                         c.status, c.current_attempt_id, c.current_epoch,
                         c.attempt_count, c.max_attempts,
                         c.prompt, c.metadata, c.next_retry_at, c.last_error,
                         c.created_at, c.updated_at,
                         ROW_NUMBER() OVER (
                             PARTITION BY c.agent_id
                             ORDER BY c.created_at ASC, c.command_id ASC
                         ) AS agent_queue_position
                     FROM agent_commands c
                     WHERE c.status = 'pending'
                       AND NOT (c.metadata ? 'runtime_host_id')
                       AND NOT EXISTS (
                           SELECT 1 FROM agent_commands c2
                           WHERE c2.agent_id = c.agent_id
                             AND c2.status IN ('leased', 'started', 'heartbeating', 'retry_scheduled')
                       )
                 )
                 SELECT
                     command_id, route_id, session_key, agent_id,
                     conversation_id, message_id, turn_id,
                     status, current_attempt_id, current_epoch,
                     attempt_count, max_attempts,
                     prompt, metadata, next_retry_at, last_error,
                     created_at, updated_at
                 FROM eligible
                 ORDER BY agent_queue_position ASC, created_at ASC, command_id ASC
                 LIMIT $1",
                &[&limit],
            )
            .await?;
        Ok(rows.iter().map(command_from_row).collect())
    }

    /// Find commands in `retry_scheduled` status whose `next_retry_at` is due.
    pub async fn find_retriable_commands(
        &self,
        now: DateTime<Utc>,
        limit: i64,
    ) -> SessionResult<Vec<AgentCommand>> {
        let client = self.connect().await?;
        let rows = client
            .query(
                "SELECT
                    command_id, route_id, session_key, agent_id,
                    conversation_id, message_id, turn_id,
                    status, current_attempt_id, current_epoch,
                    attempt_count, max_attempts,
                    prompt, metadata, next_retry_at, last_error,
                    created_at, updated_at
                 FROM agent_commands
                 WHERE status = 'retry_scheduled'
                   AND NOT (metadata ? 'runtime_host_id')
                   AND next_retry_at <= $1
                   AND attempt_count < max_attempts
                 ORDER BY next_retry_at ASC
                 LIMIT $2",
                &[&now, &limit],
            )
            .await?;
        Ok(rows.iter().map(command_from_row).collect())
    }

    /// Find commands in `retry_scheduled` status that have exhausted their
    /// max_attempts. These need to be moved to dead_letter.
    pub async fn find_exhausted_retry_commands(
        &self,
        limit: i64,
    ) -> SessionResult<Vec<AgentCommand>> {
        let client = self.connect().await?;
        let rows = client
            .query(
                "SELECT
                    command_id, route_id, session_key, agent_id,
                    conversation_id, message_id, turn_id,
                    status, current_attempt_id, current_epoch,
                    attempt_count, max_attempts,
                    prompt, metadata, next_retry_at, last_error,
                    created_at, updated_at
                 FROM agent_commands
                 WHERE status = 'retry_scheduled'
                   AND attempt_count >= max_attempts
                 ORDER BY updated_at ASC
                 LIMIT $1",
                &[&limit],
            )
            .await?;
        Ok(rows.iter().map(command_from_row).collect())
    }

    /// Atomically claim the oldest command assigned to a runtime host.
    ///
    /// Host placement is copied into command metadata when the command is
    /// inserted, so moving a binding never steals work that is already queued.
    pub async fn claim_runtime_host_command(
        &self,
        runtime_host_id: &str,
    ) -> SessionResult<Option<(AgentCommand, LeaseAssignment)>> {
        let mut client = self.connect().await?;
        let tx = client.transaction().await?;
        let now = Utc::now();
        let Some(candidate) = tx
            .query_opt(
                "SELECT c.agent_id
                 FROM agent_commands c
                 JOIN agent_runtime_bindings b
                   ON b.agent_principal_id = c.agent_id
                  AND b.state != 'disabled'
                  AND b.config_json->>'runtime_host_id' = $1
                 WHERE c.metadata->>'runtime_host_id' = $1
                   AND (c.status = 'pending'
                        OR (c.status = 'retry_scheduled' AND c.next_retry_at <= $2
                            AND c.attempt_count < c.max_attempts))
                   AND NOT EXISTS (
                       SELECT 1 FROM agent_commands active
                       WHERE active.agent_id = c.agent_id
                         AND active.status IN ('leased', 'started', 'heartbeating')
                   )
                 ORDER BY c.created_at, c.command_id
                 LIMIT 1",
                &[&runtime_host_id, &now],
            )
            .await?
        else {
            tx.commit().await?;
            return Ok(None);
        };
        let candidate_agent_id: String = candidate.get("agent_id");
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtext($1)::bigint)",
            &[&candidate_agent_id],
        )
        .await?;
        let Some(row) = tx
            .query_opt(
                "SELECT c.command_id, c.route_id, c.session_key, c.agent_id,
                        c.conversation_id, c.message_id, c.turn_id,
                        c.status, c.current_attempt_id, c.current_epoch,
                        c.attempt_count, c.max_attempts, c.prompt, c.metadata,
                        c.next_retry_at, c.last_error, c.created_at, c.updated_at
                 FROM agent_commands c
                 JOIN agent_runtime_bindings b
                   ON b.agent_principal_id = c.agent_id
                  AND b.state != 'disabled'
                  AND b.config_json->>'runtime_host_id' = $1
                 WHERE c.metadata->>'runtime_host_id' = $1
                   AND c.agent_id = $3
                   AND (c.status = 'pending'
                        OR (c.status = 'retry_scheduled' AND c.next_retry_at <= $2
                            AND c.attempt_count < c.max_attempts))
                   AND NOT EXISTS (
                       SELECT 1 FROM agent_commands active
                       WHERE active.agent_id = c.agent_id
                         AND active.status IN ('leased', 'started', 'heartbeating')
                   )
                 ORDER BY c.created_at, c.command_id
                 FOR UPDATE OF c SKIP LOCKED
                 LIMIT 1",
                &[&runtime_host_id, &now, &candidate_agent_id],
            )
            .await?
        else {
            tx.commit().await?;
            return Ok(None);
        };
        let command = command_from_row(&row);
        let session = tx
            .query_opt(
                "UPDATE session_registry
                 SET epoch = epoch + 1,
                     executor_node_id = $2,
                     status = 'active',
                     last_heartbeat_at = $3,
                     updated_at = $3
                 WHERE session_key = $1
                 RETURNING epoch",
                &[
                    &command.session_key,
                    &format!("runtime-host:{runtime_host_id}"),
                    &now,
                ],
            )
            .await?
            .ok_or_else(|| SessionError::SessionNotFound(command.session_key.clone()))?;
        let epoch: i32 = session.get("epoch");
        let attempt_id = Uuid::now_v7().to_string();
        let attempt_count = command.attempt_count + 1;
        tx.execute(
            "UPDATE agent_commands
             SET status = 'leased', current_attempt_id = $2,
                 current_epoch = $3, attempt_count = $4, updated_at = $5
             WHERE command_id = $1",
            &[
                &command.command_id,
                &attempt_id,
                &epoch,
                &attempt_count,
                &now,
            ],
        )
        .await?;
        tx.commit().await?;
        Ok(Some((
            command,
            LeaseAssignment {
                epoch,
                attempt_id,
                attempt_count,
            },
        )))
    }

    /// Commit a remote runtime result with the same attempt/epoch fences and
    /// turn-id dedup guarantees as the local writer.
    #[allow(clippy::too_many_arguments)]
    pub async fn complete_runtime_host_command(
        &self,
        runtime_host_id: &str,
        runtime_host_name: &str,
        command_id: &str,
        attempt_id: &str,
        succeeded: bool,
        contents: &[String],
        error: Option<&str>,
        tool_calls_count: i32,
        execution_duration_ms: i64,
        external_session_id: Option<&str>,
        clear_external_session: bool,
    ) -> SessionResult<()> {
        let mut client = self.connect().await?;
        let tx = client.transaction().await?;
        let now = Utc::now();
        let row = tx
            .query_opt(
                "SELECT command_id, session_key, agent_id, conversation_id,
                        turn_id, attempt_count, max_attempts, current_epoch
                 FROM agent_commands
                 WHERE command_id = $1 AND current_attempt_id = $2
                   AND metadata->>'runtime_host_id' = $3
                   AND status IN ('leased', 'started', 'heartbeating')
                   AND current_epoch = (
                       SELECT epoch FROM session_registry
                       WHERE session_key = agent_commands.session_key
                   )
                 FOR UPDATE",
                &[&command_id, &attempt_id, &runtime_host_id],
            )
            .await?
            .ok_or_else(|| SessionError::StaleAttempt {
                command_id: command_id.to_owned(),
                attempt_id: attempt_id.to_owned(),
            })?;
        let session_key: String = row.get("session_key");
        let epoch: Option<i32> = row.get("current_epoch");
        if !succeeded {
            if clear_external_session {
                let agent_id: String = row.get("agent_id");
                tx.execute(
                    "UPDATE agent_runtime_bindings
                     SET external_session_id = NULL,
                         config_json = config_json
                           - 'external_session_provenance'
                           - 'external_session_driver_type'
                           - 'external_session_binding_id'
                           - 'external_session_mode'
                           - 'external_session_captured_at',
                         updated_at = $3
                     WHERE agent_principal_id = $1
                       AND config_json->>'runtime_host_id' = $2
                       AND state <> 'disabled'",
                    &[&agent_id, &runtime_host_id, &now],
                )
                .await?;
            }
            let attempt_count: i32 = row.get("attempt_count");
            let max_attempts: i32 = row.get("max_attempts");
            let error_text = error.unwrap_or("remote runtime execution failed");
            let (status, next_retry) = if retry::is_exhausted(attempt_count, max_attempts) {
                ("dead_letter", None)
            } else {
                (
                    "retry_scheduled",
                    Some(retry::next_retry_at(now, attempt_count)),
                )
            };
            tx.execute(
                "UPDATE agent_commands
                 SET status = $3, current_attempt_id = NULL,
                     next_retry_at = $4, last_error = $5, updated_at = $6
                 WHERE command_id = $1 AND current_attempt_id = $2",
                &[
                    &command_id,
                    &attempt_id,
                    &status,
                    &next_retry,
                    &error_text,
                    &now,
                ],
            )
            .await?;
            if status == "dead_letter" {
                insert_dead_letter_in_transaction(
                    &tx,
                    &InsertDeadLetter {
                        source_type: "command".into(),
                        source_id: command_id.to_owned(),
                        payload: serde_json::json!({
                            "session_key": &session_key,
                            "runtime_host_id": runtime_host_id,
                            "attempt_id": attempt_id,
                            "turn_id": row.get::<_, String>("turn_id"),
                        }),
                        error: error_text.to_owned(),
                        attempt_count,
                    },
                    now,
                )
                .await?;
            }
            release_session_if_no_active_commands(&tx, &session_key, epoch, now).await?;
            tx.commit().await?;
            return Ok(());
        }

        let turn_id: String = row.get("turn_id");
        let conversation_id: String = row.get("conversation_id");
        let agent_id: String = row.get("agent_id");
        if let Some(session_id) = external_session_id {
            tx.execute(
                "UPDATE agent_runtime_bindings
                 SET external_session_id = $1,
                     config_json = config_json || jsonb_build_object(
                       'external_session_provenance', 'process_captured',
                       'external_session_driver_type', driver_type,
                       'external_session_binding_id', id,
                       'external_session_mode', 'headless',
                       'external_session_captured_at', $4::timestamptz
                     ),
                     updated_at = $4
                 WHERE agent_principal_id = $2
                   AND config_json->>'runtime_host_id' = $3
                   AND state <> 'disabled'",
                &[&session_id, &agent_id, &runtime_host_id, &now],
            )
            .await?;
        }
        let contents = contents
            .iter()
            .map(|content| content.trim())
            .filter(|content| !content.is_empty())
            .collect::<Vec<_>>();
        if !contents.is_empty() {
            tx.execute(
                "SELECT pg_advisory_xact_lock(hashtext($1)::bigint)",
                &[&conversation_id],
            )
            .await?;
            let first_seq: i64 = tx
                .query_one(
                    "SELECT COALESCE(MAX(seq), 0) + 1 AS seq
                     FROM conversation_events WHERE conversation_id = $1",
                    &[&conversation_id],
                )
                .await?
                .get("seq");
            for (reply_index, content) in contents.iter().enumerate() {
                let seq = first_seq + reply_index as i64;
                let event_id = Uuid::now_v7().to_string();
                let reply_turn_id = if reply_index == 0 {
                    turn_id.clone()
                } else {
                    format!("{turn_id}:remote-reply:{reply_index}")
                };
                let metadata = serde_json::json!({
                    "command_id": command_id,
                    "attempt_id": attempt_id,
                    "tool_calls_count": tool_calls_count,
                    "execution_duration_ms": execution_duration_ms,
                    "runtime_host_id": runtime_host_id,
                    "runtime_host_name": runtime_host_name,
                    "reply_index": reply_index,
                    "reply_count": contents.len(),
                });
                let inserted = tx
                    .execute(
                        "INSERT INTO conversation_events
                   (conversation_id, seq, event_id, event_type, sender_id,
                    content, content_type, metadata, turn_id, reply_event_id)
                 VALUES ($1, $2, $3, 'reply', $4, $5, 'text/plain', $6, $7, $3)
                 ON CONFLICT (turn_id) WHERE turn_id IS NOT NULL DO NOTHING",
                        &[
                            &conversation_id,
                            &seq,
                            &event_id,
                            &agent_id,
                            &content,
                            &metadata,
                            &reply_turn_id,
                        ],
                    )
                    .await?;
                if inserted == 1 {
                    let outbox_payload = serde_json::json!({
                        "message_id": event_id,
                        "conversation_id": conversation_id,
                        "sender_id": agent_id,
                        "content": content,
                        "content_type": "text/plain",
                        "seq": seq,
                        "metadata": metadata,
                    });
                    tx.execute(
                        "INSERT INTO event_outbox
                       (aggregate_type, aggregate_id, event_type, payload, created_at, published)
                     VALUES ('conversation_event', $1, 'message', $2, $3, FALSE)",
                        &[&conversation_id, &outbox_payload, &now],
                    )
                    .await?;
                    tx.execute(
                    "UPDATE conversation SET total_msg_count = total_msg_count + 1 WHERE id = $1",
                    &[&conversation_id],
                )
                    .await?;
                }
            }
        }
        tx.execute(
            "UPDATE agent_commands
             SET status = 'committed', current_attempt_id = NULL, updated_at = $3
             WHERE command_id = $1 AND current_attempt_id = $2",
            &[&command_id, &attempt_id, &now],
        )
        .await?;
        release_session_if_no_active_commands(&tx, &session_key, epoch, now).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Extend a remote command lease only when both the host and attempt still
    /// own it. A disconnected or reassigned host cannot keep work alive.
    pub async fn heartbeat_runtime_host_command(
        &self,
        runtime_host_id: &str,
        command_id: &str,
        attempt_id: &str,
    ) -> SessionResult<()> {
        let mut client = self.connect().await?;
        let tx = client.transaction().await?;
        let now = Utc::now();
        let row = tx
            .query_opt(
                "UPDATE agent_commands
                 SET status = 'heartbeating', updated_at = $4
                 WHERE command_id = $1 AND current_attempt_id = $2
                   AND metadata->>'runtime_host_id' = $3
                   AND status IN ('leased', 'started', 'heartbeating')
                   AND current_epoch = (
                       SELECT epoch FROM session_registry
                       WHERE session_key = agent_commands.session_key
                   )
                 RETURNING session_key, current_epoch",
                &[&command_id, &attempt_id, &runtime_host_id, &now],
            )
            .await?
            .ok_or_else(|| SessionError::StaleAttempt {
                command_id: command_id.to_owned(),
                attempt_id: attempt_id.to_owned(),
            })?;
        let session_key: String = row.get("session_key");
        let epoch: i32 = row.get("current_epoch");
        tx.execute(
            "UPDATE session_registry
             SET last_heartbeat_at = $3, updated_at = $3
             WHERE session_key = $1 AND epoch = $2 AND status = 'active'",
            &[&session_key, &epoch, &now],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Get a command by its ID.
    pub async fn get_command(&self, command_id: &str) -> SessionResult<Option<AgentCommand>> {
        let client = self.connect().await?;
        let row = client
            .query_opt(
                "SELECT
                    command_id, route_id, session_key, agent_id,
                    conversation_id, message_id, turn_id,
                    status, current_attempt_id, current_epoch,
                    attempt_count, max_attempts,
                    prompt, metadata, next_retry_at, last_error,
                    created_at, updated_at
                 FROM agent_commands
                 WHERE command_id = $1",
                &[&command_id],
            )
            .await?;
        Ok(row.map(|r| command_from_row(&r)))
    }

    /// Return whether an attempt still owns both its command and session epoch.
    ///
    /// This is an advisory snapshot for early fast-fail only. Correctness
    /// relies on the transactional attempt/epoch checks in subsequent writes.
    pub async fn command_attempt_is_current(
        &self,
        command_id: &str,
        attempt_id: &str,
    ) -> SessionResult<bool> {
        let client = self.connect().await?;
        let row = client
            .query_one(
                "SELECT EXISTS (
                    SELECT 1
                    FROM agent_commands ac
                    JOIN session_registry sr ON sr.session_key = ac.session_key
                    WHERE ac.command_id = $1
                      AND ac.current_attempt_id = $2
                      AND ac.current_epoch = sr.epoch
                 ) AS is_current",
                &[&command_id, &attempt_id],
            )
            .await?;
        Ok(row.get("is_current"))
    }

    // =======================================================================
    // D3: Lease assignment + epoch bump
    // =======================================================================

    /// Assign a lease for a command: bump the session epoch, generate an
    /// attempt_id, and transition the command to `leased` status.
    ///
    /// Returns the new `(epoch, attempt_id)`.
    pub async fn assign_lease(
        &self,
        command_id: &str,
        executor_node_id: &str,
    ) -> SessionResult<LeaseAssignment> {
        let mut client = self.connect().await?;
        let tx = client.transaction().await?;
        let now = Utc::now();

        // 1. Lock the command so concurrent dispatchers cannot lease it twice.
        let cmd_row = tx
            .query_opt(
                "SELECT session_key, attempt_count, status
                 FROM agent_commands
                 WHERE command_id = $1
                 FOR UPDATE",
                &[&command_id],
            )
            .await?
            .ok_or_else(|| SessionError::CommandNotFound(command_id.to_string()))?;
        let session_key: String = cmd_row.get("session_key");
        let attempt_count: i32 = cmd_row.get("attempt_count");
        let current_status: String = cmd_row.get("status");
        if current_status != CommandStatus::Pending.as_str() {
            tx.rollback().await?;
            return Err(SessionError::InvalidStateTransition {
                command_id: command_id.to_string(),
                current: current_status,
                target: CommandStatus::Leased.as_str().to_string(),
            });
        }

        // 2. Bump epoch on the session (atomic via UPDATE ... RETURNING)
        let session_row = tx
            .query_opt(
                "UPDATE session_registry
                 SET epoch = epoch + 1,
                     executor_node_id = $2,
                     status = 'active',
                     last_heartbeat_at = $3,
                     updated_at = $3
                 WHERE session_key = $1
                 RETURNING epoch",
                &[&session_key, &executor_node_id, &now],
            )
            .await?
            .ok_or_else(|| SessionError::SessionNotFound(session_key.clone()))?;
        let new_epoch: i32 = session_row.get("epoch");

        // 3. Generate attempt_id and update the command to leased
        let attempt_id = Uuid::now_v7().to_string();
        tx.execute(
            "UPDATE agent_commands
                 SET status = 'leased',
                     current_attempt_id = $2,
                     current_epoch = $3,
                     attempt_count = $4,
                     updated_at = $5
                 WHERE command_id = $1",
            &[
                &command_id,
                &attempt_id,
                &new_epoch,
                &(attempt_count + 1),
                &now,
            ],
        )
        .await?;
        tx.commit().await?;

        Ok(LeaseAssignment {
            epoch: new_epoch,
            attempt_id,
            attempt_count: attempt_count + 1,
        })
    }

    /// Assign leases for a batch of pending commands atomically.
    ///
    /// If any command is missing or no longer pending, the transaction rolls
    /// back so dispatchers never own or update a partial batch.
    pub async fn assign_batch_leases(
        &self,
        command_ids: &[String],
        executor_node_id: &str,
    ) -> SessionResult<HashMap<String, LeaseAssignment>> {
        if command_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let mut client = self.connect().await?;
        let tx = client.transaction().await?;
        let now = Utc::now();

        let rows = tx
            .query(
                "SELECT command_id, session_key, attempt_count, status
                 FROM agent_commands
                 WHERE command_id = ANY($1)
                 ORDER BY command_id
                 FOR UPDATE",
                &[&command_ids],
            )
            .await?;

        if rows.len() != command_ids.len() {
            tx.rollback().await?;
            let found: std::collections::HashSet<String> =
                rows.iter().map(|row| row.get("command_id")).collect();
            let missing = command_ids
                .iter()
                .find(|id| !found.contains(*id))
                .cloned()
                .unwrap_or_else(|| "<unknown>".to_string());
            return Err(SessionError::CommandNotFound(missing));
        }

        for row in &rows {
            let command_id: String = row.get("command_id");
            let current_status: String = row.get("status");
            if current_status != CommandStatus::Pending.as_str() {
                tx.rollback().await?;
                return Err(SessionError::InvalidStateTransition {
                    command_id,
                    current: current_status,
                    target: CommandStatus::Leased.as_str().to_string(),
                });
            }
        }

        let mut epochs_by_session = HashMap::new();
        for row in &rows {
            let session_key: String = row.get("session_key");
            if epochs_by_session.contains_key(&session_key) {
                continue;
            }
            let session_row = tx
                .query_opt(
                    "UPDATE session_registry
                     SET epoch = epoch + 1,
                         executor_node_id = $2,
                         status = 'active',
                         last_heartbeat_at = $3,
                         updated_at = $3
                     WHERE session_key = $1
                     RETURNING epoch",
                    &[&session_key, &executor_node_id, &now],
                )
                .await?
                .ok_or_else(|| SessionError::SessionNotFound(session_key.clone()))?;
            let new_epoch: i32 = session_row.get("epoch");
            epochs_by_session.insert(session_key, new_epoch);
        }

        let mut assignments = HashMap::with_capacity(rows.len());
        for row in rows {
            let command_id: String = row.get("command_id");
            let session_key: String = row.get("session_key");
            let attempt_count: i32 = row.get("attempt_count");
            let new_epoch = *epochs_by_session
                .get(&session_key)
                .ok_or_else(|| SessionError::SessionNotFound(session_key.clone()))?;
            let attempt_id = Uuid::now_v7().to_string();

            tx.execute(
                "UPDATE agent_commands
                 SET status = 'leased',
                     current_attempt_id = $2,
                     current_epoch = $3,
                     attempt_count = $4,
                     updated_at = $5
                 WHERE command_id = $1",
                &[
                    &command_id,
                    &attempt_id,
                    &new_epoch,
                    &(attempt_count + 1),
                    &now,
                ],
            )
            .await?;

            assignments.insert(
                command_id,
                LeaseAssignment {
                    epoch: new_epoch,
                    attempt_id,
                    attempt_count: attempt_count + 1,
                },
            );
        }

        tx.commit().await?;
        Ok(assignments)
    }

    /// Mark a command succeeded only for the attempt that currently owns it.
    pub async fn mark_command_succeeded_for_attempt(
        &self,
        command_id: &str,
        expected_attempt_id: &str,
    ) -> SessionResult<()> {
        let client = self.connect().await?;
        let now = Utc::now();
        let affected = client
            .execute(
                "UPDATE agent_commands
                 SET status = 'succeeded', updated_at = $3
                 WHERE command_id = $1 AND current_attempt_id = $2
                   AND status IN ('leased', 'started', 'heartbeating')
                   AND current_epoch = (
                       SELECT epoch FROM session_registry
                       WHERE session_key = agent_commands.session_key
                   )",
                &[&command_id, &expected_attempt_id, &now],
            )
            .await?;
        if affected == 0 {
            return Err(SessionError::StaleAttempt {
                command_id: command_id.to_string(),
                attempt_id: expected_attempt_id.to_string(),
            });
        }
        Ok(())
    }

    /// Commit a command and release its session only if the attempt is current.
    pub async fn mark_command_committed_for_attempt(
        &self,
        command_id: &str,
        expected_attempt_id: &str,
    ) -> SessionResult<()> {
        let mut client = self.connect().await?;
        let tx = client.transaction().await?;
        let now = Utc::now();
        let row = tx
            .query_opt(
                "UPDATE agent_commands
                 SET status = 'committed', current_attempt_id = NULL, updated_at = $3
                 WHERE command_id = $1 AND current_attempt_id = $2
                   AND current_epoch = (
                       SELECT epoch FROM session_registry
                       WHERE session_key = agent_commands.session_key
                   )
                 RETURNING session_key, current_epoch",
                &[&command_id, &expected_attempt_id, &now],
            )
            .await?;
        let Some(row) = row else {
            tx.rollback().await?;
            return Err(SessionError::StaleAttempt {
                command_id: command_id.to_string(),
                attempt_id: expected_attempt_id.to_string(),
            });
        };
        let session_key: String = row.get("session_key");
        let command_epoch: Option<i32> = row.get("current_epoch");
        release_session_if_no_active_commands(&tx, &session_key, command_epoch, now).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Validate that the provided epoch matches the session's current epoch.
    pub async fn validate_epoch(
        &self,
        session_key: &str,
        expected_epoch: i32,
    ) -> SessionResult<bool> {
        let client = self.connect().await?;
        let row = client
            .query_opt(
                "SELECT epoch FROM session_registry WHERE session_key = $1",
                &[&session_key],
            )
            .await?
            .ok_or_else(|| SessionError::SessionNotFound(session_key.to_string()))?;
        let current_epoch: i32 = row.get("epoch");
        Ok(current_epoch == expected_epoch)
    }

    /// Release a lease: reset the session to idle and mark the command as
    /// succeeded.
    pub async fn release_lease(&self, command_id: &str) -> SessionResult<()> {
        let client = self.connect().await?;
        let now = Utc::now();

        // Get session_key from command
        let cmd_row = client
            .query_opt(
                "SELECT session_key FROM agent_commands WHERE command_id = $1",
                &[&command_id],
            )
            .await?
            .ok_or_else(|| SessionError::CommandNotFound(command_id.to_string()))?;
        let session_key: String = cmd_row.get("session_key");

        // Mark command as succeeded
        client
            .execute(
                "UPDATE agent_commands
                 SET status = 'succeeded', updated_at = $2
                 WHERE command_id = $1",
                &[&command_id, &now],
            )
            .await?;

        // Reset session to idle
        client
            .execute(
                "UPDATE session_registry
                 SET status = 'idle',
                     executor_node_id = NULL,
                     last_heartbeat_at = NULL,
                     updated_at = $2
                 WHERE session_key = $1",
                &[&session_key, &now],
            )
            .await?;

        Ok(())
    }

    // =======================================================================
    // D4: Heartbeat monitoring + lease expiry detection
    // =======================================================================

    /// Find all sessions with active commands whose heartbeat has expired.
    ///
    /// A lease is considered expired if `last_heartbeat_at` is older than
    /// `timeout_secs` from the given `now` time.
    pub async fn check_expired_leases(
        &self,
        now: DateTime<Utc>,
        timeout_secs: i64,
    ) -> SessionResult<Vec<ExpiredLease>> {
        let client = self.connect().await?;
        let cutoff = now - chrono::TimeDelta::seconds(timeout_secs);
        let rows = client
            .query(
                "SELECT
                    sr.session_key,
                    ac.command_id,
                    sr.epoch,
                    ac.attempt_count,
                    ac.max_attempts
                 FROM session_registry sr
                 JOIN agent_commands ac
                   ON ac.session_key = sr.session_key
                  AND ac.status IN ('leased', 'started', 'heartbeating')
                 WHERE sr.status = 'active'
                   AND (sr.last_heartbeat_at IS NULL OR sr.last_heartbeat_at < $1)",
                &[&cutoff],
            )
            .await?;

        Ok(rows
            .iter()
            .map(|r| ExpiredLease {
                session_key: r.get("session_key"),
                command_id: r.get("command_id"),
                epoch: r.get("epoch"),
                attempt_count: r.get("attempt_count"),
                max_attempts: r.get("max_attempts"),
            })
            .collect())
    }

    /// Handle a single expired lease: either schedule a retry or dead-letter
    /// the command. Retry decisions use values reread from the locked command
    /// row; the corresponding [`ExpiredLease`] fields are logging snapshots.
    pub async fn handle_lease_expiry(&self, expired: &ExpiredLease) -> SessionResult<()> {
        let mut client = self.connect().await?;
        let tx = client.transaction().await?;
        let now = Utc::now();
        let command = tx
            .query_opt(
                "SELECT session_key, current_epoch, status, attempt_count, max_attempts
                 FROM agent_commands WHERE command_id = $1 FOR UPDATE",
                &[&expired.command_id],
            )
            .await?
            .ok_or_else(|| SessionError::CommandNotFound(expired.command_id.clone()))?;
        let session_key: String = command.get("session_key");
        let current_epoch: Option<i32> = command.get("current_epoch");
        let current_status: String = command.get("status");
        if current_epoch != Some(expired.epoch) {
            tx.rollback().await?;
            return Err(SessionError::EpochMismatch {
                expected: expired.epoch,
                actual: current_epoch.unwrap_or(-1),
            });
        }
        if !matches!(
            current_status.as_str(),
            "leased" | "started" | "heartbeating"
        ) {
            tx.rollback().await?;
            return Err(SessionError::InvalidStateTransition {
                command_id: expired.command_id.clone(),
                current: current_status,
                target: "retry_scheduled".to_string(),
            });
        }
        let attempt_count: i32 = command.get("attempt_count");
        let max_attempts: i32 = command.get("max_attempts");

        // Bump the session epoch only if this lease still owns it. A batch can
        // contain several commands with the same epoch; the first member
        // fences the executor and later members only finalize their own rows.
        tx.execute(
            "UPDATE session_registry
                 SET epoch = epoch + 1,
                     executor_node_id = NULL,
                     status = 'idle',
                     last_heartbeat_at = NULL,
                     updated_at = $2
                 WHERE session_key = $1 AND epoch = $3",
            &[&session_key, &now, &expired.epoch],
        )
        .await?;

        if retry::is_exhausted(attempt_count, max_attempts) {
            tx.execute(
                "UPDATE agent_commands
                     SET status = 'dead_letter', next_retry_at = NULL,
                         current_attempt_id = NULL,
                         last_error = 'max attempts exceeded after lease expiry',
                         updated_at = $3
                     WHERE command_id = $1 AND current_epoch = $2",
                &[&expired.command_id, &expired.epoch, &now],
            )
            .await?;
            let payload = serde_json::json!({
                "session_key": session_key,
                "epoch": expired.epoch,
                "attempt_count": attempt_count,
            });
            insert_dead_letter_in_transaction(
                &tx,
                &InsertDeadLetter {
                    source_type: "command".to_string(),
                    source_id: expired.command_id.clone(),
                    payload,
                    error: "max attempts exceeded after lease expiry".to_string(),
                    attempt_count,
                },
                now,
            )
            .await?;
        } else {
            let retry_at = retry::next_retry_at(now, attempt_count);
            tx.execute(
                "UPDATE agent_commands
                     SET status = 'retry_scheduled',
                         next_retry_at = $2,
                         current_attempt_id = NULL,
                         last_error = 'lease expired',
                         updated_at = $4
                     WHERE command_id = $1 AND current_epoch = $3",
                &[&expired.command_id, &retry_at, &expired.epoch, &now],
            )
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    // =======================================================================
    // D6: Dead letter operations
    // =======================================================================

    /// Insert a dead letter record.
    pub async fn insert_dead_letter(&self, input: &InsertDeadLetter) -> SessionResult<DeadLetter> {
        let mut client = self.connect().await?;
        let tx = client.transaction().await?;
        let now = Utc::now();
        let dead_letter = insert_dead_letter_in_transaction(&tx, input, now).await?;
        tx.commit().await?;
        Ok(dead_letter)
    }

    /// Atomically dead-letter a command only when the attempt still owns it.
    pub async fn dead_letter_command_for_attempt(
        &self,
        input: &InsertDeadLetter,
        expected_attempt_id: &str,
    ) -> SessionResult<DeadLetter> {
        let mut client = self.connect().await?;
        let tx = client.transaction().await?;
        let now = Utc::now();
        let row = tx
            .query_opt(
                "UPDATE agent_commands
                 SET status = 'dead_letter', next_retry_at = NULL,
                     current_attempt_id = NULL, last_error = $2, updated_at = $3
                 WHERE command_id = $1 AND current_attempt_id = $4
                   AND current_epoch = (
                       SELECT epoch FROM session_registry
                       WHERE session_key = agent_commands.session_key
                   )
                 RETURNING session_key, current_epoch",
                &[&input.source_id, &input.error, &now, &expected_attempt_id],
            )
            .await?;
        let Some(row) = row else {
            tx.rollback().await?;
            return Err(SessionError::StaleAttempt {
                command_id: input.source_id.clone(),
                attempt_id: expected_attempt_id.to_string(),
            });
        };
        let session_key: String = row.get("session_key");
        let command_epoch: Option<i32> = row.get("current_epoch");
        release_session_if_no_active_commands(&tx, &session_key, command_epoch, now).await?;

        let dead_letter = insert_dead_letter_in_transaction(&tx, input, now).await?;
        tx.commit().await?;
        Ok(dead_letter)
    }

    /// List unresolved dead letters, ordered by creation time.
    pub async fn list_dead_letters(&self, limit: i64) -> SessionResult<Vec<DeadLetter>> {
        let client = self.connect().await?;
        let rows = client
            .query(
                "SELECT
                    id, source_type, source_id, payload, error,
                    attempt_count, created_at, resolved_at, resolved_by
                 FROM dead_letters
                 WHERE resolved_at IS NULL
                 ORDER BY created_at ASC
                 LIMIT $1",
                &[&limit],
            )
            .await?;
        Ok(rows.iter().map(dead_letter_from_row).collect())
    }

    /// List dead letters created after a given timestamp.
    pub async fn list_dead_letters_since(
        &self,
        since: DateTime<Utc>,
        limit: i64,
    ) -> SessionResult<Vec<DeadLetter>> {
        let client = self.connect().await?;
        let rows = client
            .query(
                "SELECT
                    id, source_type, source_id, payload, error,
                    attempt_count, created_at, resolved_at, resolved_by
                 FROM dead_letters
                 WHERE created_at >= $1
                 ORDER BY created_at ASC
                 LIMIT $2",
                &[&since, &limit],
            )
            .await?;
        Ok(rows.iter().map(dead_letter_from_row).collect())
    }

    // =======================================================================
    // D6b: Efficient counting + TTL sweep
    // =======================================================================

    /// Count agent_commands in `pending` status using a COUNT(*) query
    /// instead of fetching all rows.
    pub async fn count_pending_commands(&self) -> SessionResult<i64> {
        let client = self.connect().await?;
        let row = client
            .query_one(
                "SELECT COUNT(*) FROM agent_commands WHERE status = 'pending'",
                &[],
            )
            .await?;
        Ok(row.get::<_, i64>(0))
    }

    /// Count unresolved dead letters using a COUNT(*) query instead of
    /// fetching all rows.
    pub async fn count_dead_letters(&self) -> SessionResult<i64> {
        let client = self.connect().await?;
        let row = client
            .query_one(
                "SELECT COUNT(*) FROM dead_letters WHERE resolved_at IS NULL",
                &[],
            )
            .await?;
        Ok(row.get::<_, i64>(0))
    }

    /// Dead-letter pending commands older than `max_age_secs` seconds.
    /// Returns the number of commands affected.
    ///
    /// PostgreSQL does not support LIMIT directly in UPDATE, so we use a
    /// subquery to select the oldest stale commands.
    pub async fn dead_letter_stale_pending_commands(
        &self,
        max_age_secs: i64,
        limit: i64,
    ) -> SessionResult<i64> {
        let client = self.connect().await?;
        let cutoff = Utc::now() - chrono::TimeDelta::seconds(max_age_secs);
        let affected = client
            .execute(
                "UPDATE agent_commands
                 SET status = 'dead_letter', last_error = 'TTL expired'
                 WHERE command_id IN (
                     SELECT command_id FROM agent_commands
                     WHERE status = 'pending' AND created_at < $1
                     ORDER BY created_at
                     LIMIT $2
                 )",
                &[&cutoff, &limit],
            )
            .await?;
        Ok(affected as i64)
    }

    // =======================================================================
    // D7: Executor registry (in-memory)
    // =======================================================================

    /// Register an executor node as available.
    pub async fn register_executor(
        &self,
        node_id: &str,
        capabilities: serde_json::Value,
    ) -> SessionResult<Executor> {
        let now = Utc::now();
        let executor = Executor {
            node_id: node_id.to_string(),
            capabilities,
            status: ExecutorStatus::Available,
            last_heartbeat_at: now,
            registered_at: now,
        };
        let mut executors = self.executors.write().await;
        executors.insert(node_id.to_string(), executor.clone());
        Ok(executor)
    }

    /// Find an available executor. For now, returns the first available
    /// executor. In the future, this should match by agent_id capabilities.
    pub async fn find_available_executor(&self, _agent_id: &str) -> SessionResult<Executor> {
        let executors = self.executors.read().await;
        executors
            .values()
            .find(|e| e.status == ExecutorStatus::Available)
            .cloned()
            .ok_or_else(|| SessionError::NoAvailableExecutor(_agent_id.to_string()))
    }

    /// Deregister an executor node.
    pub async fn deregister_executor(&self, node_id: &str) -> SessionResult<()> {
        let mut executors = self.executors.write().await;
        executors.remove(node_id);
        Ok(())
    }

    /// Update an executor's heartbeat timestamp.
    pub async fn update_executor_heartbeat(&self, node_id: &str) -> SessionResult<()> {
        let mut executors = self.executors.write().await;
        if let Some(executor) = executors.get_mut(node_id) {
            executor.last_heartbeat_at = Utc::now();
            Ok(())
        } else {
            Err(SessionError::NoAvailableExecutor(node_id.to_string()))
        }
    }

    // =======================================================================
    // Health check
    // =======================================================================

    /// Run a lightweight `SELECT 1` query to verify database connectivity.
    pub async fn health_check(&self) -> SessionResult<()> {
        let client = self.connect().await?;
        client
            .execute("SELECT 1", &[])
            .await
            .map_err(|e| SessionError::Database(format!("health check failed: {e}")))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Row mappers
// ---------------------------------------------------------------------------

fn session_from_row(row: &Row) -> Session {
    let status_str: String = row.get("status");
    Session {
        session_key: row.get("session_key"),
        agent_id: row.get("agent_id"),
        conversation_id: row.get("conversation_id"),
        executor_node_id: row.get("executor_node_id"),
        epoch: row.get("epoch"),
        status: SessionStatus::parse(&status_str).unwrap_or(SessionStatus::Idle),
        workspace_snapshot_id: row.get("workspace_snapshot_id"),
        memory_context_id: row.get("memory_context_id"),
        last_heartbeat_at: row.get("last_heartbeat_at"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn command_from_row(row: &Row) -> AgentCommand {
    let status_str: String = row.get("status");
    AgentCommand {
        command_id: row.get("command_id"),
        route_id: row.get("route_id"),
        session_key: row.get("session_key"),
        agent_id: row.get("agent_id"),
        conversation_id: row.get("conversation_id"),
        message_id: row.get("message_id"),
        turn_id: row.get("turn_id"),
        status: CommandStatus::parse(&status_str).unwrap_or(CommandStatus::Pending),
        current_attempt_id: row.get("current_attempt_id"),
        current_epoch: row.get("current_epoch"),
        attempt_count: row.get("attempt_count"),
        max_attempts: row.get("max_attempts"),
        prompt: row.get("prompt"),
        metadata: row.get("metadata"),
        next_retry_at: row.get("next_retry_at"),
        last_error: row.get("last_error"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn runtime_status_command_from_command(
    command: &AgentCommand,
    now: DateTime<Utc>,
) -> RuntimeStatusCommand {
    RuntimeStatusCommand {
        command_id: command.command_id.clone(),
        message_id: command.message_id.clone(),
        turn_id: command.turn_id.clone(),
        status: command.status.as_str().to_string(),
        created_at: command.created_at,
        updated_at: command.updated_at,
        lease_age_seconds: now
            .signed_duration_since(command.updated_at)
            .num_seconds()
            .max(0),
        attempt_count: command.attempt_count,
        last_error: command.last_error.clone(),
    }
}

fn dead_letter_from_row(row: &Row) -> DeadLetter {
    DeadLetter {
        id: row.get("id"),
        source_type: row.get("source_type"),
        source_id: row.get("source_id"),
        payload: row.get("payload"),
        error: row.get("error"),
        attempt_count: row.get("attempt_count"),
        created_at: row.get("created_at"),
        resolved_at: row.get("resolved_at"),
        resolved_by: row.get("resolved_by"),
    }
}

async fn runtime_host_metadata(
    tx: &tokio_postgres::Transaction<'_>,
    agent_id: &str,
    metadata: &serde_json::Value,
) -> SessionResult<serde_json::Value> {
    let runtime_host_id = tx
        .query_opt(
            "SELECT config_json->>'runtime_host_id' AS runtime_host_id
             FROM agent_runtime_bindings
             WHERE agent_principal_id = $1 AND state != 'disabled'
             ORDER BY updated_at DESC
             LIMIT 1",
            &[&agent_id],
        )
        .await?
        .and_then(|row| row.get::<_, Option<String>>("runtime_host_id"));
    let mut metadata = metadata.as_object().cloned().ok_or_else(|| {
        SessionError::Internal("agent command metadata must be a JSON object".into())
    })?;
    if let Some(runtime_host_id) = runtime_host_id {
        metadata.insert(
            "runtime_host_id".into(),
            serde_json::Value::String(runtime_host_id),
        );
    }
    Ok(serde_json::Value::Object(metadata))
}

async fn insert_dead_letter_in_transaction(
    tx: &tokio_postgres::Transaction<'_>,
    input: &InsertDeadLetter,
    now: DateTime<Utc>,
) -> SessionResult<DeadLetter> {
    let id = Uuid::now_v7().to_string();
    let row = tx
        .query_one(
            "INSERT INTO dead_letters (
                id, source_type, source_id, payload, error,
                attempt_count, created_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7)
             RETURNING id, source_type, source_id, payload, error,
                       attempt_count, created_at, resolved_at, resolved_by",
            &[
                &id,
                &input.source_type,
                &input.source_id,
                &input.payload,
                &input.error,
                &input.attempt_count,
                &now,
            ],
        )
        .await?;
    Ok(dead_letter_from_row(&row))
}

async fn release_session_if_no_active_commands(
    tx: &tokio_postgres::Transaction<'_>,
    session_key: &str,
    command_epoch: Option<i32>,
    now: DateTime<Utc>,
) -> SessionResult<()> {
    tx.execute(
        "UPDATE session_registry sr
         SET status = 'idle', executor_node_id = NULL,
             last_heartbeat_at = NULL, updated_at = $2
         WHERE sr.session_key = $1 AND ($3::int IS NULL OR sr.epoch = $3)
           AND NOT EXISTS (
               SELECT 1 FROM agent_commands ac
               WHERE ac.session_key = sr.session_key
                 AND ac.status IN ('leased', 'started', 'heartbeating')
           )",
        &[&session_key, &now, &command_epoch],
    )
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Pool builder
// ---------------------------------------------------------------------------

fn build_pool(database_url: &str) -> Pool {
    let mut pg_config: tokio_postgres::Config = database_url
        .parse()
        .expect("invalid database connection string");
    pg_config.connect_timeout(std::time::Duration::from_secs(5));
    let mgr_config = ManagerConfig {
        // Verified: run "SELECT 1" before handing out a connection.
        // Prevents "db error" from stale/dead connections in the pool.
        recycling_method: RecyclingMethod::Verified,
    };
    let mgr = Manager::from_config(pg_config, tokio_postgres::NoTls, mgr_config);
    Pool::builder(mgr)
        .max_size(16)
        .build()
        .expect("failed to build connection pool")
}
