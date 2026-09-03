//! PostgreSQL-backed MemberProvider and DecisionSink for the router.
//!
//! These implementations bridge the `choruz-router` traits to the real
//! database tables used by the Choruz platform.

use choruz_router::{
    AgentPolicy, AssignedTaskHint, AssigneeRosterEntry, AutoMode, ChannelTaskStatus,
    ConversationMember, ConversationRoutingPolicy, DecisionSink, GroupWorkflowTask,
    GroupWorkflowTaskParticipant, MailboxVisibility, MemberProvider, RouteDecision, RouterError,
    RouterResult, UntaggedHumanMode,
};
use choruz_session::{InsertCommand, PgSessionStore};
use choruz_store::EventStore;

/// Provides conversation membership and policy data from PostgreSQL.
#[derive(Clone)]
pub struct PgMemberProvider {
    store: EventStore,
}

impl PgMemberProvider {
    pub fn new(store: EventStore) -> Self {
        Self { store }
    }
}

impl MemberProvider for PgMemberProvider {
    async fn list_agent_members(
        &self,
        conversation_id: &str,
    ) -> RouterResult<Vec<ConversationMember>> {
        let client = self.store.connect().await.map_err(RouterError::Store)?;

        let rows = client
            .query(
                "SELECT cm.conv_id, cm.principal_id, p.type AS principal_type,
                        p.name AS display_name, cm.joined_at, cm.removed_at
                 FROM conversation_member cm
                 JOIN principal p ON p.id = cm.principal_id
                 JOIN conversation c ON c.id = cm.conv_id
                 WHERE cm.conv_id = $1
                   AND p.type = 'agent'
                   AND p.workspace_id = c.workspace_id
                   AND cm.removed_at IS NULL
                   AND p.disabled = FALSE
                   AND p.deleted_at IS NULL
                   AND p.channel_visibility != 'internal'",
                &[&conversation_id],
            )
            .await
            .map_err(|e| RouterError::Internal(format!("list_agent_members: {e}")))?;

        Ok(rows
            .into_iter()
            .map(|r| ConversationMember {
                conversation_id: r.get("conv_id"),
                principal_id: r.get("principal_id"),
                principal_type: r.get::<_, String>("principal_type"),
                display_name: r.get::<_, Option<String>>("display_name"),
                joined_at: r.get("joined_at"),
                left_at: r.get("removed_at"),
            })
            .collect())
    }

    async fn get_agent_policy(
        &self,
        agent_id: &str,
        conversation_id: &str,
    ) -> RouterResult<AgentPolicy> {
        let client = self.store.connect().await.map_err(RouterError::Store)?;

        // 0. Check if this is a direct conversation — DMs always trigger the agent
        let conv_type_row = client
            .query_opt(
                "SELECT type FROM conversation WHERE id = $1",
                &[&conversation_id],
            )
            .await
            .map_err(|e| RouterError::Internal(format!("get conversation type: {e}")))?;
        let is_direct = conv_type_row
            .as_ref()
            .and_then(|r| r.get::<_, Option<String>>("type"))
            .as_deref()
            == Some("direct");

        // 1. DMs auto-trigger; groups check agent_policies then conversation_runtime_policies
        let auto_mode = if is_direct {
            AutoMode::AllMessages
        } else {
            // Check agent-specific policy first
            let agent_policy_row = client
                .query_opt(
                    "SELECT auto_mode FROM agent_policies
                     WHERE agent_id = $1 AND conversation_id = $2",
                    &[&agent_id, &conversation_id],
                )
                .await
                .map_err(|e| RouterError::Internal(format!("get agent policy: {e}")))?;

            if let Some(r) = agent_policy_row {
                let mode_str: String = r.get("auto_mode");
                match mode_str.as_str() {
                    "all_messages" => AutoMode::AllMessages,
                    "manual" => AutoMode::Manual,
                    _ => AutoMode::MentionedOnly,
                }
            } else {
                // Fallback to conversation-level policy
                let policy_row = client
                    .query_opt(
                        "SELECT auto_mode FROM conversation_runtime_policies
                         WHERE conversation_id = $1",
                        &[&conversation_id],
                    )
                    .await
                    .map_err(|e| RouterError::Internal(format!("get conversation policy: {e}")))?;

                match policy_row {
                    Some(r) => {
                        let mode_str: String = r.get("auto_mode");
                        match mode_str.as_str() {
                            "disabled" => AutoMode::Manual,
                            "metadata_only" => AutoMode::Manual,
                            _ => AutoMode::MentionedOnly,
                        }
                    }
                    None => AutoMode::MentionedOnly,
                }
            }
        };

        // 2. Get mention aliases: first from binding config_json, fallback to principal.name.
        //    Bindings are workspace-scoped (one row per agent, migration 0018),
        //    so no conversation_id predicate is needed. The `conversation_id`
        //    column is retained on the row as creation metadata only.
        //
        //    A partial UNIQUE index (0018) guarantees at most one non-disabled
        //    binding per agent in steady state, but provision flows that
        //    disable-then-insert can briefly expose two rows to a concurrent
        //    reader. `ORDER BY updated_at DESC LIMIT 1` guarantees ≤1 row at
        //    SQL level so `query_opt` never blows up during that window.
        let alias_row = client
            .query_opt(
                "SELECT config_json->>'mention_aliases' AS aliases
                 FROM agent_runtime_bindings
                 WHERE agent_principal_id = $1
                   AND state != 'disabled'
                 ORDER BY updated_at DESC
                 LIMIT 1",
                &[&agent_id],
            )
            .await
            .map_err(|e| RouterError::Internal(format!("get binding aliases: {e}")))?;

        // Distinguish "field missing" (legitimate — fall back to the
        // principal's display name below) from "field is malformed JSON"
        // (a provision-side bug we want to surface, not swallow).
        let mut mention_aliases: Vec<String> = match alias_row
            .and_then(|r| r.get::<_, Option<String>>("aliases"))
        {
            Some(raw) => match serde_json::from_str::<Vec<String>>(&raw) {
                Ok(list) => list,
                Err(e) => {
                    tracing::warn!(
                        agent_id,
                        raw = %raw,
                        error = %e,
                        "binding.config_json.mention_aliases is not a JSON string array; falling back to principal.name"
                    );
                    Vec::new()
                }
            },
            None => Vec::new(),
        };

        // If no aliases from binding, use agent's display name
        if mention_aliases.is_empty() {
            let name_row = client
                .query_opt("SELECT name FROM principal WHERE id = $1", &[&agent_id])
                .await
                .map_err(|e| RouterError::Internal(format!("get agent name: {e}")))?;
            if let Some(name) = name_row.and_then(|r| r.get::<_, Option<String>>("name")) {
                mention_aliases.push(name);
            }
        }

        Ok(AgentPolicy {
            agent_id: agent_id.into(),
            conversation_id: conversation_id.into(),
            auto_mode,
            mention_aliases,
        })
    }

    async fn list_assignee_roster(
        &self,
        conversation_id: &str,
    ) -> RouterResult<Vec<AssigneeRosterEntry>> {
        let client = self.store.connect().await.map_err(RouterError::Store)?;

        let rows = client
            .query(
                "SELECT p.id, p.type, p.name,
                        COALESCE(rh.name, 'This computer') AS runtime_host_name
                 FROM principal p
                 JOIN conversation c ON c.id = $1
                 JOIN conversation_member cm
                   ON cm.conv_id = c.id
                  AND cm.principal_id = p.id
                  AND cm.removed_at IS NULL
                 LEFT JOIN LATERAL (
                   SELECT config_json
                   FROM agent_runtime_bindings
                   WHERE agent_principal_id = p.id AND state != 'disabled'
                   ORDER BY updated_at DESC, id DESC
                   LIMIT 1
                 ) arb ON TRUE
                 LEFT JOIN runtime_host rh
                   ON rh.id = arb.config_json->>'runtime_host_id' AND rh.revoked_at IS NULL
                 WHERE p.type = 'agent'
                   AND p.workspace_id = c.workspace_id
                   AND p.disabled = FALSE
                   AND p.deleted_at IS NULL
                   AND p.channel_visibility != 'internal'
                 ORDER BY lower(p.name), p.id",
                &[&conversation_id],
            )
            .await
            .map_err(|e| RouterError::Internal(format!("list_assignee_roster: {e}")))?;

        Ok(rows
            .into_iter()
            .map(|row| AssigneeRosterEntry {
                principal_id: row.get("id"),
                principal_type: row.get("type"),
                display_name: row.get("name"),
                runtime_host_name: Some(row.get("runtime_host_name")),
            })
            .collect())
    }

    async fn resolve_principal_name(&self, principal_id: &str) -> Option<String> {
        let client = self.store.connect().await.ok()?;
        let row = client
            .query_opt("SELECT name FROM principal WHERE id = $1", &[&principal_id])
            .await
            .ok()?;
        row.map(|r| r.get::<_, String>("name"))
    }

    async fn resolve_conversation_name(&self, conversation_id: &str) -> Option<String> {
        let client = self.store.connect().await.ok()?;
        let row = client
            .query_opt(
                "SELECT name, type FROM conversation WHERE id = $1",
                &[&conversation_id],
            )
            .await
            .ok()??;
        let conv_type: String = row.get("type");
        if conv_type == "direct" {
            // Return "[DM]" so the prompt format signals a direct chat
            Some("[DM]".to_string())
        } else {
            row.get::<_, Option<String>>("name")
        }
    }

    async fn resolve_principal_type(
        &self,
        principal_id: &str,
        conversation_id: &str,
    ) -> RouterResult<Option<String>> {
        let client = self.store.connect().await.map_err(RouterError::Store)?;
        let row = client
            .query_opt(
                "SELECT p.type
                 FROM principal p
                 JOIN conversation c ON c.id = $2
                 JOIN conversation_member cm
                   ON cm.conv_id = c.id
                  AND cm.principal_id = p.id
                  AND cm.removed_at IS NULL
                 WHERE p.id = $1
                   AND (p.workspace_id = c.workspace_id OR p.type = 'human')
                   AND p.disabled = FALSE
                   AND p.deleted_at IS NULL",
                &[&principal_id, &conversation_id],
            )
            .await
            .map_err(|e| RouterError::Internal(format!("resolve principal type: {e}")))?;
        Ok(row.map(|row| row.get("type")))
    }

    async fn get_conversation_routing_policy(
        &self,
        conversation_id: &str,
    ) -> RouterResult<ConversationRoutingPolicy> {
        let client = self.store.connect().await.map_err(RouterError::Store)?;
        let row = client
            .query_opt(
                "SELECT default_coordinator_agent_id, untagged_human_mode
                 FROM conversation_runtime_policies
                 WHERE conversation_id = $1",
                &[&conversation_id],
            )
            .await
            .map_err(|e| RouterError::Internal(format!("get conversation routing policy: {e}")))?;

        Ok(row
            .map(|row| ConversationRoutingPolicy {
                conversation_id: conversation_id.to_string(),
                default_coordinator_agent_id: row.get("default_coordinator_agent_id"),
                untagged_human_mode: UntaggedHumanMode::from_db_value(
                    row.get::<_, String>("untagged_human_mode").as_str(),
                ),
            })
            .unwrap_or_else(|| ConversationRoutingPolicy::default_for(conversation_id)))
    }

    async fn find_workflow_task(
        &self,
        conversation_id: &str,
        task_id: Option<&str>,
        task_key: Option<&str>,
    ) -> RouterResult<Option<GroupWorkflowTask>> {
        let client = self.store.connect().await.map_err(RouterError::Store)?;

        if let (Some(task_id), Some(task_key)) = (task_id, task_key) {
            let row = client
                .query_opt(
                    "SELECT gwt.id, gwt.conversation_id, gwt.task_key, gwt.status,
                            gwt.assignee_principal_id,
                            assignee.type AS assignee_principal_type
                     FROM group_workflow_task gwt
                     JOIN conversation c ON c.id = gwt.conversation_id
                     JOIN principal assignee ON assignee.id = gwt.assignee_principal_id
                     JOIN conversation_member assignee_cm
                       ON assignee_cm.conv_id = gwt.conversation_id
                      AND assignee_cm.principal_id = gwt.assignee_principal_id
                      AND assignee_cm.removed_at IS NULL
                     WHERE gwt.conversation_id = $1 AND gwt.id = $2 AND gwt.task_key = $3
                       AND gwt.status IN ('todo', 'in_progress', 'blocked', 'in_review', 'done')
                       AND assignee.workspace_id = c.workspace_id
                       AND assignee.disabled = FALSE
                       AND assignee.deleted_at IS NULL
                       AND NOT (assignee.type = 'agent' AND assignee.channel_visibility = 'internal')",
                    &[&conversation_id, &task_id, &task_key],
                )
                .await
                .map_err(|e| RouterError::Internal(format!("find workflow task by id/key: {e}")))?;
            return row.map(group_workflow_task_from_row).transpose();
        }

        if let Some(task_id) = task_id {
            if let Some(row) = client
                .query_opt(
                    "SELECT gwt.id, gwt.conversation_id, gwt.task_key, gwt.status,
                            gwt.assignee_principal_id,
                            assignee.type AS assignee_principal_type
                     FROM group_workflow_task gwt
                     JOIN conversation c ON c.id = gwt.conversation_id
                     JOIN principal assignee ON assignee.id = gwt.assignee_principal_id
                     JOIN conversation_member assignee_cm
                       ON assignee_cm.conv_id = gwt.conversation_id
                      AND assignee_cm.principal_id = gwt.assignee_principal_id
                      AND assignee_cm.removed_at IS NULL
                     WHERE gwt.conversation_id = $1 AND gwt.id = $2
                       AND gwt.status IN ('todo', 'in_progress', 'blocked', 'in_review', 'done')
                       AND assignee.workspace_id = c.workspace_id
                       AND assignee.disabled = FALSE
                       AND assignee.deleted_at IS NULL
                       AND NOT (assignee.type = 'agent' AND assignee.channel_visibility = 'internal')",
                    &[&conversation_id, &task_id],
                )
                .await
                .map_err(|e| RouterError::Internal(format!("find workflow task by id: {e}")))?
            {
                return group_workflow_task_from_row(row).map(Some);
            }
        }

        let Some(task_key) = task_key else {
            return Ok(None);
        };
        let row = client
            .query_opt(
                "SELECT gwt.id, gwt.conversation_id, gwt.task_key, gwt.status,
                        gwt.assignee_principal_id,
                        assignee.type AS assignee_principal_type
                 FROM group_workflow_task gwt
                 JOIN conversation c ON c.id = gwt.conversation_id
                 JOIN principal assignee ON assignee.id = gwt.assignee_principal_id
                 JOIN conversation_member assignee_cm
                   ON assignee_cm.conv_id = gwt.conversation_id
                  AND assignee_cm.principal_id = gwt.assignee_principal_id
                  AND assignee_cm.removed_at IS NULL
                 WHERE gwt.conversation_id = $1 AND gwt.task_key = $2
                   AND gwt.status IN ('todo', 'in_progress', 'blocked', 'in_review', 'done')
                   AND assignee.workspace_id = c.workspace_id
                   AND assignee.disabled = FALSE
                   AND assignee.deleted_at IS NULL
                   AND NOT (assignee.type = 'agent' AND assignee.channel_visibility = 'internal')",
                &[&conversation_id, &task_key],
            )
            .await
            .map_err(|e| RouterError::Internal(format!("find workflow task by key: {e}")))?;
        row.map(group_workflow_task_from_row).transpose()
    }

    async fn list_workflow_task_participants(
        &self,
        task_id: &str,
    ) -> RouterResult<Vec<GroupWorkflowTaskParticipant>> {
        let client = self.store.connect().await.map_err(RouterError::Store)?;
        let rows = client
            .query(
                "SELECT gwtp.task_id, gwtp.principal_id, gwtp.role_key, p.type AS principal_type
                 FROM group_workflow_task_participant gwtp
                 JOIN group_workflow_task gwt ON gwt.id = gwtp.task_id
                 JOIN conversation c ON c.id = gwt.conversation_id
                 JOIN principal p ON p.id = gwtp.principal_id
                 JOIN conversation_member cm
                   ON cm.conv_id = gwt.conversation_id
                  AND cm.principal_id = gwtp.principal_id
                  AND cm.removed_at IS NULL
                 WHERE gwtp.task_id = $1
                   AND gwtp.role_key <> 'owner'
                   AND p.workspace_id = c.workspace_id
                   AND p.disabled = FALSE
                   AND p.deleted_at IS NULL
                   AND NOT (p.type = 'agent' AND p.channel_visibility = 'internal')
                 ORDER BY gwtp.role_key, gwtp.principal_id",
                &[&task_id],
            )
            .await
            .map_err(|e| RouterError::Internal(format!("list workflow participants: {e}")))?;

        Ok(rows
            .into_iter()
            .map(|row| GroupWorkflowTaskParticipant {
                task_id: row.get("task_id"),
                principal_id: row.get("principal_id"),
                role_key: row.get("role_key"),
                principal_type: row.get("principal_type"),
            })
            .collect())
    }

    async fn list_open_tasks_for_agent(
        &self,
        conversation_id: &str,
        principal_id: &str,
    ) -> RouterResult<Vec<AssignedTaskHint>> {
        let client = self.store.connect().await.map_err(RouterError::Store)?;
        // Mirror find_workflow_task's visibility filters (workspace match,
        // disabled, deleted, internal-only agents) so the hint never surfaces
        // a card the receiving agent cannot actually own. Filter `done` out
        // — the envelope is for cards the agent should still act on or
        // update, not closed history. The (conversation_id,
        // assignee_principal_id) index from migration 0025 covers this
        // predicate.
        let rows = client
            .query(
                "SELECT gwt.task_key, gwt.title, gwt.status
                 FROM group_workflow_task gwt
                 JOIN conversation c ON c.id = gwt.conversation_id
                 JOIN principal assignee ON assignee.id = gwt.assignee_principal_id
                 JOIN conversation_member assignee_cm
                   ON assignee_cm.conv_id = gwt.conversation_id
                  AND assignee_cm.principal_id = gwt.assignee_principal_id
                  AND assignee_cm.removed_at IS NULL
                 WHERE gwt.conversation_id = $1
                   AND gwt.assignee_principal_id = $2
                   AND gwt.status IN ('todo', 'in_progress', 'blocked', 'in_review')
                   AND assignee.workspace_id = c.workspace_id
                   AND assignee.disabled = FALSE
                   AND assignee.deleted_at IS NULL
                   AND NOT (assignee.type = 'agent' AND assignee.channel_visibility = 'internal')
                 ORDER BY gwt.task_key",
                &[&conversation_id, &principal_id],
            )
            .await
            .map_err(|e| RouterError::Internal(format!("list open tasks for agent: {e}")))?;

        let mut hints = Vec::with_capacity(rows.len());
        for row in rows {
            let status_raw: String = row.get("status");
            let Some(status) = ChannelTaskStatus::from_db_value(&status_raw) else {
                // Out-of-board statuses are not actionable for the agent and
                // would just bloat the prompt. Skip silently — the SQL
                // WHERE clause already filters the known set, so any miss
                // here is a future schema addition.
                continue;
            };
            hints.push(AssignedTaskHint {
                task_key: row.get("task_key"),
                title: row.get("title"),
                status,
            });
        }
        Ok(hints)
    }
}

/// Writes routing outputs (visibility, decisions, commands) to PostgreSQL.
#[derive(Clone)]
pub struct PgDecisionSink {
    store: EventStore,
    session_store: PgSessionStore,
}

impl PgDecisionSink {
    pub fn new(store: EventStore, session_store: PgSessionStore) -> Self {
        Self {
            store,
            session_store,
        }
    }
}

impl DecisionSink for PgDecisionSink {
    async fn write_visibility(&self, v: &MailboxVisibility) -> RouterResult<()> {
        let client = self.store.connect().await.map_err(RouterError::Store)?;

        // Upsert: if the same (agent_id, conversation_id, message_id) exists, skip.
        client
            .execute(
                "INSERT INTO mailbox_visibility
                    (agent_id, conversation_id, message_id, event_seq, visible_at)
                 VALUES ($1, $2, $3, $4, NOW())
                 ON CONFLICT (agent_id, conversation_id, message_id) DO NOTHING",
                &[&v.agent_id, &v.conversation_id, &v.message_id, &v.event_seq],
            )
            .await
            .map_err(|e| RouterError::Internal(format!("write_visibility: {e}")))?;

        Ok(())
    }

    async fn write_decision(&self, d: &RouteDecision) -> RouterResult<RouteDecision> {
        let mut client = self.store.connect().await.map_err(RouterError::Store)?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| RouterError::Internal(format!("write_decision tx: {e}")))?;
        let idempotency_key = format!("{}:{}", d.message_id, d.agent_id);
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtext($1)::bigint)",
            &[&idempotency_key],
        )
        .await
        .map_err(|e| RouterError::Internal(format!("write_decision lock: {e}")))?;

        if let Some(row) = tx
            .query_opt(
                "SELECT route_id, message_id, agent_id, conversation_id,
                        decision, reason, policy_snapshot
                 FROM route_decisions
                 WHERE message_id = $1 AND agent_id = $2",
                &[&d.message_id, &d.agent_id],
            )
            .await
            .map_err(|e| RouterError::Internal(format!("write_decision existing: {e}")))?
        {
            let decision = route_decision_from_row(&row);
            tx.commit()
                .await
                .map_err(|e| RouterError::Internal(format!("write_decision commit: {e}")))?;
            return Ok(decision);
        }

        let row = tx
            .query_one(
                "INSERT INTO route_decisions
                    (route_id, message_id, agent_id, conversation_id,
                     decision, reason, policy_snapshot, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
                 RETURNING route_id, message_id, agent_id, conversation_id,
                           decision, reason, policy_snapshot",
                &[
                    &d.route_id,
                    &d.message_id,
                    &d.agent_id,
                    &d.conversation_id,
                    &d.decision,
                    &d.reason,
                    &d.policy_snapshot,
                ],
            )
            .await
            .map_err(|e| RouterError::Internal(format!("write_decision: {e}")))?;
        let decision = route_decision_from_row(&row);
        tx.commit()
            .await
            .map_err(|e| RouterError::Internal(format!("write_decision commit: {e}")))?;

        Ok(decision)
    }

    async fn write_command(&self, cmd: &InsertCommand) -> RouterResult<()> {
        // Also upsert the session
        self.session_store
            .upsert_session(&cmd.session_key, &cmd.agent_id, &cmd.conversation_id)
            .await?;

        self.session_store.insert_command(cmd).await?;
        Ok(())
    }
}

fn route_decision_from_row(row: &tokio_postgres::Row) -> RouteDecision {
    RouteDecision {
        route_id: row.get("route_id"),
        message_id: row.get("message_id"),
        agent_id: row.get("agent_id"),
        conversation_id: row.get("conversation_id"),
        decision: row.get("decision"),
        reason: row.get("reason"),
        policy_snapshot: row.get("policy_snapshot"),
    }
}

fn group_workflow_task_from_row(row: tokio_postgres::Row) -> RouterResult<GroupWorkflowTask> {
    let raw_status: String = row.get("status");
    let status = ChannelTaskStatus::from_db_value(&raw_status).ok_or_else(|| {
        RouterError::Internal(format!("workflow task has non-board status: {raw_status}"))
    })?;
    Ok(GroupWorkflowTask {
        id: row.get("id"),
        conversation_id: row.get("conversation_id"),
        task_key: row.get("task_key"),
        status,
        assignee_principal_id: row.get("assignee_principal_id"),
        assignee_principal_type: row.get("assignee_principal_type"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use choruz_router::{InMemoryDecisionSink, route_event};
    use choruz_store::ConversationEventRow;
    use chrono::Utc;
    use std::{
        env, fs,
        path::PathBuf,
        process::Command,
        sync::OnceLock,
        time::{SystemTime, UNIX_EPOCH},
    };
    use tokio_postgres::NoTls;

    struct TestDatabase {
        database_url: String,
        admin_database_url: String,
        database_name: String,
    }

    impl TestDatabase {
        async fn create() -> Self {
            let admin_database_url = connection_string("postgres");
            let database_name = format!(
                "choruz_pipeline_member_{}_{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("system clock before epoch")
                    .as_nanos()
            );
            let (admin_client, connection) = connect_admin_database(&admin_database_url).await;
            tokio::spawn(async move {
                let _ = connection.await;
            });
            admin_client
                .execute(&format!("CREATE DATABASE {database_name}"), &[])
                .await
                .expect("create temp db");

            let database_url = connection_string(&database_name);
            let db = Self {
                database_url,
                admin_database_url,
                database_name,
            };
            db.apply_migrations().await;
            db
        }

        async fn apply_migrations(&self) {
            let (client, connection) = tokio_postgres::connect(&self.database_url, NoTls)
                .await
                .expect("connect temp db");
            tokio::spawn(async move {
                let _ = connection.await;
            });

            let mut files = fs::read_dir(migrations_dir())
                .expect("read migrations dir")
                .map(|entry| entry.expect("migration dir entry").path())
                .collect::<Vec<_>>();
            files.sort();

            for file in files {
                let sql = fs::read_to_string(&file).expect("read migration file");
                if sql.contains("CONCURRENTLY") {
                    let executable_sql = sql
                        .lines()
                        .filter(|line| !line.trim_start().starts_with("--"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    for statement in executable_sql
                        .split(';')
                        .map(str::trim)
                        .filter(|statement| !statement.is_empty())
                    {
                        client
                            .batch_execute(statement)
                            .await
                            .expect("apply non-transactional migration statement");
                    }
                } else {
                    client.batch_execute(&sql).await.expect("apply migration");
                }
            }
        }
    }

    impl Drop for TestDatabase {
        fn drop(&mut self) {
            let admin_database_url = self.admin_database_url.clone();
            let database_name = self.database_name.clone();
            let handle = std::thread::spawn(move || {
                let runtime = tokio::runtime::Runtime::new().expect("create cleanup runtime");
                runtime.block_on(async move {
                    let (client, connection) = tokio_postgres::connect(&admin_database_url, NoTls)
                        .await
                        .expect("connect admin db for cleanup");
                    tokio::spawn(async move {
                        let _ = connection.await;
                    });
                    let _ = client
                        .execute(
                            "SELECT pg_terminate_backend(pid)
                             FROM pg_stat_activity
                             WHERE datname = $1
                               AND pid <> pg_backend_pid()",
                            &[&database_name],
                        )
                        .await;
                    let _ = client
                        .execute(&format!("DROP DATABASE IF EXISTS {database_name}"), &[])
                        .await;
                });
            });
            let _ = handle.join();
        }
    }

    fn migrations_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
    }

    fn host_start_script() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../infra/host/start.sh")
    }

    fn connection_string(database_name: &str) -> String {
        let host = env::var("CHORUZ_PG_HOST").unwrap_or_else(|_| "127.0.0.1".into());
        let port = env::var("CHORUZ_PG_PORT").unwrap_or_else(|_| "5432".into());
        let user = env::var("CHORUZ_PG_USER")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| env::var("USER").ok())
            .unwrap_or_else(|| "postgres".into());
        let password = env::var("CHORUZ_PG_PASSWORD").ok();

        let mut connection = format!("host={host} port={port} user={user} dbname={database_name}");
        if let Some(password) = password.filter(|value| !value.trim().is_empty()) {
            connection.push_str(" password=");
            connection.push_str(&password);
        }
        connection
    }

    async fn connect_admin_database(
        admin_database_url: &str,
    ) -> (
        tokio_postgres::Client,
        tokio_postgres::Connection<tokio_postgres::Socket, tokio_postgres::tls::NoTlsStream>,
    ) {
        match tokio_postgres::connect(admin_database_url, NoTls).await {
            Ok(connection) => connection,
            Err(first_error) => {
                ensure_postgres_started().unwrap_or_else(|message| {
                    panic!("failed to auto-start postgres: {message}; initial error: {first_error}")
                });
                tokio_postgres::connect(admin_database_url, NoTls)
                    .await
                    .unwrap_or_else(|error| panic!("connect admin db after auto-start: {error}"))
            }
        }
    }

    fn ensure_postgres_started() -> Result<(), String> {
        static START_ONCE: OnceLock<Result<(), String>> = OnceLock::new();
        START_ONCE
            .get_or_init(|| {
                let output = Command::new("bash")
                    .arg(host_start_script())
                    .output()
                    .map_err(|error| format!("spawn start.sh: {error}"))?;
                if output.status.success() {
                    Ok(())
                } else {
                    Err(format!(
                        "start.sh exited with {}: {}",
                        output.status,
                        String::from_utf8_lossy(&output.stderr)
                    ))
                }
            })
            .clone()
    }

    #[tokio::test]
    async fn test_get_agent_policy_returns_default_for_unknown() {
        // This test requires a running DB. Skip in CI.
        let Ok(db_url) = std::env::var("CHORUZ_DATABASE_URL") else {
            return;
        };
        let store = EventStore::new(&db_url);
        let provider = PgMemberProvider::new(store);

        // Should NOT error — should return default MentionedOnly policy
        let policy = provider
            .get_agent_policy("nonexistent-agent", "nonexistent-conv")
            .await;
        assert!(
            policy.is_ok(),
            "get_agent_policy should not error: {:?}",
            policy.err()
        );
        let p = policy.unwrap();
        assert_eq!(p.auto_mode, AutoMode::MentionedOnly);
    }

    #[tokio::test]
    async fn write_decision_returns_existing_route_id_for_message_agent_retry() {
        let Ok(db_url) = std::env::var("CHORUZ_DATABASE_URL") else {
            return;
        };

        let store = EventStore::new(&db_url);
        let session_store = PgSessionStore::new(&db_url);
        let sink = PgDecisionSink::new(store.clone(), session_store);
        let client = store
            .connect()
            .await
            .expect("connect for route id retry test");
        let message_id = choruz_common::new_id();
        let agent_id = choruz_common::new_id();
        let conversation_id = choruz_common::new_id();
        let legacy_route_id = choruz_common::new_id();
        client
            .execute(
                "INSERT INTO route_decisions
                    (route_id, message_id, agent_id, conversation_id,
                     decision, reason, policy_snapshot, created_at)
                 VALUES ($1, $2, $3, $4, 'trigger', 'legacy', '{}', NOW())",
                &[&legacy_route_id, &message_id, &agent_id, &conversation_id],
            )
            .await
            .expect("seed legacy route decision");

        let returned = sink
            .write_decision(&RouteDecision {
                route_id: choruz_common::new_id(),
                message_id: message_id.clone(),
                agent_id: agent_id.clone(),
                conversation_id,
                decision: "trigger".into(),
                reason: "retry".into(),
                policy_snapshot: serde_json::json!({}),
            })
            .await
            .expect("write decision retry");

        assert_eq!(returned.route_id, legacy_route_id);
        assert_eq!(returned.decision, "trigger");
    }

    #[tokio::test]
    async fn cross_workspace_agent_member_is_not_routed_even_when_mentioned() {
        let database = TestDatabase::create().await;
        let db_url = database.database_url.clone();

        let suffix = choruz_common::new_id();
        let ws_group = format!("ws-group-{suffix}");
        let ws_other = format!("ws-other-{suffix}");
        let human_id = format!("human-{suffix}");
        let outsider_id = format!("agent-outsider-{suffix}");
        let conversation_id = format!("conv-{suffix}");
        let event_id = format!("evt-{suffix}");

        let (client, connection) = tokio_postgres::connect(&db_url, NoTls)
            .await
            .expect("connect for cross-workspace routing seed");
        tokio::spawn(async move {
            let _ = connection.await;
        });

        client
            .execute(
                "INSERT INTO principal (id, workspace_id, type, name, disabled, created_at, updated_at)
                 VALUES ($1, $2, 'human', 'Human', FALSE, NOW(), NOW()),
                        ($3, $4, 'agent', 'Outside Agent', FALSE, NOW(), NOW())",
                &[&human_id, &ws_group, &outsider_id, &ws_other],
            )
            .await
            .expect("seed principals");
        client
            .execute(
                "INSERT INTO conversation (id, workspace_id, type, name, creator_id, created_at, updated_at)
                 VALUES ($1, $2, 'group', 'Mention Guard', $3, NOW(), NOW())",
                &[&conversation_id, &ws_group, &human_id],
            )
            .await
            .expect("seed conversation");
        client
            .execute(
                "INSERT INTO conversation_member (conv_id, principal_id, joined_at)
                 VALUES ($1, $2, NOW())",
                &[&conversation_id, &outsider_id],
            )
            .await
            .expect("seed cross-workspace member row");

        let provider = PgMemberProvider::new(EventStore::new(&db_url));
        let sink = InMemoryDecisionSink::default();
        let event = ConversationEventRow {
            conversation_id: conversation_id.clone(),
            seq: 1,
            event_id,
            event_type: "message".into(),
            sender_id: human_id.clone(),
            content: Some("@Outside Agent please do not wake".into()),
            content_type: "text/plain".into(),
            metadata: serde_json::json!({}),
            client_msg_id: None,
            turn_id: None,
            reply_event_id: None,
            created_at: Utc::now(),
        };

        let outcomes = route_event(&event, &provider, &sink).await;
        let roster = provider
            .list_assignee_roster(&conversation_id)
            .await
            .expect("list assignee roster");
        let commands_empty = sink.commands.lock().await.is_empty();
        let decisions_empty = sink.decisions.lock().await.is_empty();
        let visibilities_empty = sink.visibilities.lock().await.is_empty();

        client
            .execute(
                "DELETE FROM conversation WHERE id = $1",
                &[&conversation_id],
            )
            .await
            .expect("cleanup seeded conversation");
        client
            .execute(
                "DELETE FROM principal WHERE id IN ($1, $2)",
                &[&human_id, &outsider_id],
            )
            .await
            .expect("cleanup seeded principals");

        let outcomes = outcomes.unwrap();
        assert!(
            outcomes.is_empty(),
            "cross-workspace agent should not be considered a routable member"
        );
        assert!(
            roster.is_empty(),
            "cross-workspace member rows should not appear in the runtime assignee roster"
        );
        assert!(commands_empty);
        assert!(decisions_empty);
        assert!(visibilities_empty);
    }

    #[tokio::test]
    async fn hybrid_routing_provider_loads_policy_task_and_participants() {
        let database = TestDatabase::create().await;
        let db_url = database.database_url.clone();

        let suffix = choruz_common::new_id();
        let workspace_id = format!("ws-hybrid-{suffix}");
        let human_id = format!("human-{suffix}");
        let coordinator_id = format!("agent-coordinator-{suffix}");
        let owner_id = format!("agent-owner-{suffix}");
        let stale_owner_id = format!("agent-stale-owner-{suffix}");
        let internal_agent_id = format!("agent-internal-{suffix}");
        let disabled_agent_id = format!("agent-disabled-{suffix}");
        let approver_human_id = format!("human-approver-{suffix}");
        let operator_id = format!("operator-{suffix}");
        let conversation_id = format!("conv-{suffix}");
        let task_id = format!("task-{suffix}");

        let (client, connection) = tokio_postgres::connect(&db_url, NoTls)
            .await
            .expect("connect for hybrid provider seed");
        tokio::spawn(async move {
            let _ = connection.await;
        });

        client
            .execute(
                "INSERT INTO principal
                    (id, workspace_id, type, name, disabled, channel_visibility, created_at, updated_at)
                 VALUES ($1, $2, 'human', 'Human', FALSE, 'visible', NOW(), NOW()),
                        ($3, $2, 'agent', 'Coordinator', FALSE, 'visible', NOW(), NOW()),
                        ($4, $2, 'agent', 'Owner', FALSE, 'visible', NOW(), NOW()),
                        ($5, $2, 'agent', 'Disabled Agent', TRUE, 'visible', NOW(), NOW()),
                        ($6, $2, 'human', 'Human Approver', FALSE, 'visible', NOW(), NOW()),
                        ($7, 'ws-local', 'human', 'Operator', FALSE, 'visible', NOW(), NOW()),
                        ($8, $2, 'agent', 'Stale Owner', FALSE, 'visible', NOW(), NOW()),
                        ($9, $2, 'agent', 'Internal Helper', FALSE, 'internal', NOW(), NOW())",
                &[
                    &human_id,
                    &workspace_id,
                    &coordinator_id,
                    &owner_id,
                    &disabled_agent_id,
                    &approver_human_id,
                    &operator_id,
                    &stale_owner_id,
                    &internal_agent_id,
                ],
            )
            .await
            .expect("seed principals");
        client
            .execute(
                "INSERT INTO conversation (id, workspace_id, type, name, creator_id, created_at, updated_at)
                 VALUES ($1, $2, 'group', 'Hybrid Routing', $3, NOW(), NOW())",
                &[&conversation_id, &workspace_id, &human_id],
            )
            .await
            .expect("seed conversation");
        client
            .execute(
                "INSERT INTO conversation_member (conv_id, principal_id, joined_at)
                 VALUES ($1, $2, NOW()),
                        ($1, $3, NOW()),
                        ($1, $4, NOW()),
                        ($1, $5, NOW()),
                        ($1, $6, NOW()),
                        ($1, $7, NOW()),
                        ($1, $8, NOW()),
                        ($1, $9, NOW())",
                &[
                    &conversation_id,
                    &human_id,
                    &coordinator_id,
                    &owner_id,
                    &disabled_agent_id,
                    &approver_human_id,
                    &operator_id,
                    &stale_owner_id,
                    &internal_agent_id,
                ],
            )
            .await
            .expect("seed memberships");
        client
            .execute(
                "INSERT INTO conversation_runtime_policies
                    (conversation_id, default_coordinator_agent_id, untagged_human_mode)
                 VALUES ($1, $2, 'coordinator_only')",
                &[&conversation_id, &coordinator_id],
            )
            .await
            .expect("seed conversation policy");
        client
            .execute(
                "INSERT INTO group_workflow_task
                    (id, conversation_id, task_key, title, status, assignee_principal_id, created_by)
                 VALUES ($1, $2, 'DOC-P0-03', 'Draft doc', 'in_review', $3, $4)",
                &[&task_id, &conversation_id, &owner_id, &human_id],
            )
            .await
            .expect("seed workflow task");
        client
            .execute(
                "INSERT INTO group_workflow_task
                    (id, conversation_id, task_key, title, status, assignee_principal_id, created_by)
                 VALUES ($1, $2, 'INTERNAL-1', 'Internal task', 'todo', $3, $4)",
                &[
                    &format!("task-internal-{suffix}"),
                    &conversation_id,
                    &internal_agent_id,
                    &human_id,
                ],
            )
            .await
            .expect("seed internal-assignee workflow task");
        for (idx, (task_key, status, _expected)) in [
            ("STATUS-TODO", "todo", ChannelTaskStatus::Todo),
            (
                "STATUS-IN-PROGRESS",
                "in_progress",
                ChannelTaskStatus::InProgress,
            ),
            ("STATUS-BLOCKED", "blocked", ChannelTaskStatus::Blocked),
            ("STATUS-IN-REVIEW", "in_review", ChannelTaskStatus::InReview),
            ("STATUS-DONE", "done", ChannelTaskStatus::Done),
        ]
        .into_iter()
        .enumerate()
        {
            client
                .execute(
                    "INSERT INTO group_workflow_task
                        (id, conversation_id, task_key, title, status, assignee_principal_id, created_by)
                     VALUES ($1, $2, $3, $4, $5, $6, $7)",
                    &[
                        &format!("task-status-{idx}-{suffix}"),
                        &conversation_id,
                        &task_key,
                        &format!("Task status {status}"),
                        &status,
                        &owner_id,
                        &human_id,
                    ],
                )
                .await
                .expect("seed status workflow task");
        }
        client
            .execute(
                "INSERT INTO group_workflow_task_participant
                    (id, task_id, principal_id, role_key)
                 VALUES ($1, $2, $3, 'coordinator'),
                        ($4, $2, $5, 'owner'),
                        ($6, $2, $7, 'reviewer'),
                        ($8, $2, $9, 'approver')",
                &[
                    &format!("part-coordinator-{suffix}"),
                    &task_id,
                    &coordinator_id,
                    &format!("part-owner-{suffix}"),
                    &stale_owner_id,
                    &format!("part-disabled-{suffix}"),
                    &disabled_agent_id,
                    &format!("part-approver-{suffix}"),
                    &approver_human_id,
                ],
            )
            .await
            .expect("seed workflow participants");

        let provider = PgMemberProvider::new(EventStore::new(&db_url));

        assert_eq!(
            provider
                .resolve_principal_type(&human_id, &conversation_id)
                .await
                .expect("resolve sender type")
                .as_deref(),
            Some("human")
        );
        assert_eq!(
            provider
                .resolve_principal_type(&operator_id, &conversation_id)
                .await
                .expect("resolve human sender type")
                .as_deref(),
            Some("human"),
            "the installation user should qualify as a human sender when they are a conversation member"
        );
        assert!(
            provider
                .resolve_principal_type(&disabled_agent_id, &conversation_id)
                .await
                .expect("disabled sender type lookup")
                .is_none(),
            "disabled principals should not qualify as routable senders"
        );
        let agent_members = provider
            .list_agent_members(&conversation_id)
            .await
            .expect("list agent members");
        assert!(
            !agent_members
                .iter()
                .any(|member| member.principal_id == internal_agent_id),
            "internal agents should not be projected as visible routable members"
        );
        let roster = provider
            .list_assignee_roster(&conversation_id)
            .await
            .expect("list assignee roster");
        let roster_ids = roster
            .iter()
            .map(|entry| entry.principal_id.as_str())
            .collect::<Vec<_>>();
        assert!(roster_ids.contains(&coordinator_id.as_str()));
        assert!(roster_ids.contains(&owner_id.as_str()));
        assert!(roster_ids.contains(&stale_owner_id.as_str()));
        assert!(
            !roster_ids.contains(&human_id.as_str()),
            "humans should not appear in the agent runtime assignee roster"
        );
        assert!(
            !roster_ids.contains(&approver_human_id.as_str()),
            "humans should not appear in the agent runtime assignee roster"
        );
        assert!(
            !roster_ids.contains(&disabled_agent_id.as_str()),
            "disabled principals should not appear in the runtime assignee roster"
        );
        assert!(
            !roster_ids.contains(&internal_agent_id.as_str()),
            "internal agents should not appear in the runtime assignee roster"
        );
        assert!(
            !roster_ids.contains(&operator_id.as_str()),
            "humans are not valid channel task assignees"
        );
        let policy = provider
            .get_conversation_routing_policy(&conversation_id)
            .await
            .expect("load routing policy");
        assert_eq!(
            policy.default_coordinator_agent_id.as_deref(),
            Some(coordinator_id.as_str())
        );
        assert_eq!(
            policy.untagged_human_mode,
            UntaggedHumanMode::CoordinatorOnly
        );

        let task = provider
            .find_workflow_task(&conversation_id, Some(&task_id), Some("DOC-P0-03"))
            .await
            .expect("find task")
            .expect("task exists");
        assert_eq!(task.task_key, "DOC-P0-03");
        assert_eq!(task.status, ChannelTaskStatus::InReview);
        assert_eq!(task.assignee_principal_id, owner_id);
        assert_eq!(task.assignee_principal_type.as_deref(), Some("agent"));
        assert!(
            provider
                .find_workflow_task(&conversation_id, Some(&task_id), Some("OTHER"))
                .await
                .expect("find mismatched task")
                .is_none(),
            "task_id and task_key must identify the same row"
        );
        assert!(
            provider
                .find_workflow_task(&conversation_id, None, Some("INTERNAL-1"))
                .await
                .expect("find internal-assignee task")
                .is_none(),
            "internal assignees must not be visible through router task projection"
        );
        for (task_key, _status, expected) in [
            ("STATUS-TODO", "todo", ChannelTaskStatus::Todo),
            (
                "STATUS-IN-PROGRESS",
                "in_progress",
                ChannelTaskStatus::InProgress,
            ),
            ("STATUS-BLOCKED", "blocked", ChannelTaskStatus::Blocked),
            ("STATUS-IN-REVIEW", "in_review", ChannelTaskStatus::InReview),
            ("STATUS-DONE", "done", ChannelTaskStatus::Done),
        ] {
            let status_task = provider
                .find_workflow_task(&conversation_id, None, Some(task_key))
                .await
                .expect("find status task")
                .expect("status task exists");
            assert_eq!(status_task.status, expected, "task {task_key}");
        }

        let participants = provider
            .list_workflow_task_participants(&task_id)
            .await
            .expect("list participants");
        let participants_by_role = participants
            .iter()
            .map(|participant| {
                (
                    participant.role_key.as_str(),
                    participant.principal_id.as_str(),
                    participant.principal_type.as_deref(),
                )
            })
            .collect::<Vec<_>>();

        assert!(participants_by_role.contains(&(
            "coordinator",
            coordinator_id.as_str(),
            Some("agent")
        )));
        assert!(
            !participants
                .iter()
                .any(|participant| participant.role_key == "owner"),
            "provider participant projection should not expose stored owner rows"
        );
        assert!(participants_by_role.contains(&(
            "approver",
            approver_human_id.as_str(),
            Some("human")
        )));
        assert!(
            !participants
                .iter()
                .any(|participant| participant.principal_id == disabled_agent_id),
            "disabled agent participants should not be routable"
        );

        let sink = InMemoryDecisionSink::default();
        let event = ConversationEventRow {
            conversation_id: conversation_id.clone(),
            seq: 1,
            event_id: format!("evt-workflow-{suffix}"),
            event_type: "message".into(),
            sender_id: human_id.clone(),
            content: Some("workflow feedback".into()),
            content_type: "text/plain".into(),
            metadata: serde_json::json!({
                "workflow": {
                    "kind": "task.feedback",
                    "task_key": "DOC-P0-03"
                }
            }),
            client_msg_id: None,
            turn_id: None,
            reply_event_id: None,
            created_at: Utc::now(),
        };

        route_event(&event, &provider, &sink)
            .await
            .expect("route provider-backed workflow event");
        let mut command_agent_ids = sink
            .commands
            .lock()
            .await
            .iter()
            .map(|command| command.agent_id.clone())
            .collect::<Vec<_>>();
        command_agent_ids.sort();
        assert_eq!(command_agent_ids, vec![coordinator_id.clone(), owner_id]);
        assert!(
            !command_agent_ids.contains(&stale_owner_id),
            "stale owner participant row must not override canonical assignee"
        );
    }
}
