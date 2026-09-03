//! Outbox command processing for headless-mode agents.

use crate::pg_member_provider::PgMemberProvider;
use choruz_router::{AssigneeRosterEntry, MemberProvider};
use choruz_store::EventStore;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const PROCESSING_STALE_AFTER: Duration = Duration::from_secs(60);

#[cfg(test)]
fn parse_dev_env_value(contents: &str, key: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        let (name, value) = line.split_once('=')?;
        if name.trim() != key {
            return None;
        }
        Some(
            value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string(),
        )
    })
}

/// Logs in as the configured operator and returns a gateway-issued session
/// token, or `None` on failure. Credentials come from `CHORUZ_OPERATOR_USER` /
/// `CHORUZ_OPERATOR_PASSWORD` (matching gateway `local_auth::from_env`). Never
/// hardcode them here — pipeline and gateway must stay in sync via env.
async fn operator_session_token(gateway_base_url: &str) -> Option<String> {
    let username = std::env::var("CHORUZ_OPERATOR_USER").unwrap_or_else(|_| "operator".to_string());
    let password =
        std::env::var("CHORUZ_OPERATOR_PASSWORD").unwrap_or_else(|_| "choruz-local".to_string());
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/auth/local/login", gateway_base_url))
        .json(&serde_json::json!({
            "username": username,
            "password": password,
        }))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: serde_json::Value = resp.json().await.ok()?;
    body.get("session_token")
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

fn web_base_url_for_outbox(gateway_base_url: &str) -> String {
    std::env::var("CHORUZ_WEB_BASE_URL")
        .unwrap_or_else(|_| gateway_base_url.replace(":3000", ":3100"))
}

/// Scan the Maildir outbox and dispatch commands.
///
/// Returns the reply text (if any) that the conversation writer should commit.
/// - A `"send"` to a group INSERTs directly and returns an empty string so the
///   writer skips it (the group already saw the message).
/// - A `"send"` without a group (DM) returns its content so the writer inserts
///   it as the agent's reply.
/// - `provision_agent` / `share_file` return a status or markdown string so the
///   writer commits it as the agent's reply.
/// - `set_cron` / `create_group` are side effects only — they
///   return an empty string and the writer skips insertion.
/// - No outbox command at all returns an empty string.  The CLI's raw stdout
///   is intentionally NOT used as a fallback reply — agents must go through
///   `.choruz/send` to speak into a conversation.
#[cfg(test)]
async fn process_outbox_commands(
    session_key: &str,
    agent_id: &str,
    work_dir: &Path,
    gateway_base_url: &str,
    event_store: Option<&EventStore>,
) -> String {
    process_outbox_commands_with_stats(
        session_key,
        agent_id,
        work_dir,
        gateway_base_url,
        event_store,
    )
    .await
    .reply
}

pub(crate) struct OutboxProcessResult {
    pub reply: String,
    pub processed_count: usize,
    pub command_results: Vec<serde_json::Value>,
}

pub(crate) async fn process_outbox_commands_with_stats(
    session_key: &str,
    agent_id: &str,
    work_dir: &Path,
    gateway_base_url: &str,
    event_store: Option<&EventStore>,
) -> OutboxProcessResult {
    // Scan .choruz-outbox/new/ for command files, sorted by filename so that
    // `.choruz/send` (which timestamps-ulids its filenames) produces stable
    // ordering.
    let maildir_new = work_dir.join(".choruz-outbox").join("new");
    if maildir_new.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&maildir_new) {
            let mut files: Vec<_> = entries
                .flatten()
                .filter(|e| {
                    matches!(
                        e.path().extension().and_then(|x| x.to_str()),
                        Some("json" | "processing")
                    )
                })
                .collect();
            files.sort_by_key(|e| e.file_name());
            let paths = files
                .into_iter()
                .map(|entry| entry.path())
                .collect::<Vec<_>>();

            return process_outbox_command_files(
                session_key,
                agent_id,
                work_dir,
                gateway_base_url,
                event_store,
                &paths,
            )
            .await;
        }
    }

    // No outbox command produced a reply — drop the CLI's raw stdout instead of
    // leaking it into the conversation.  Agents that want to speak must use
    // `.choruz/send`; stdout is internal chatter (task-complete notes, thinking)
    // and the writer's empty-content guard will skip the insert.
    OutboxProcessResult {
        reply: String::new(),
        processed_count: 0,
        command_results: Vec::new(),
    }
}

pub(crate) async fn process_outbox_command_files(
    session_key: &str,
    agent_id: &str,
    work_dir: &Path,
    gateway_base_url: &str,
    event_store: Option<&EventStore>,
    files: &[PathBuf],
) -> OutboxProcessResult {
    // Drain the selected Maildir batch. Every claimed file gets processed so
    // side-effect commands all fire when an agent emits multiple commands in a
    // single turn; all non-empty replies are joined into the writer-visible
    // response.
    let mut replies: Vec<String> = Vec::new();
    let mut command_results: Vec<serde_json::Value> = Vec::new();
    let mut processed_count = 0;
    for path in files {
        let Some(claimed_path) = claim_outbox_file(path).await else {
            continue;
        };

        let raw = match tokio::fs::read_to_string(&claimed_path).await {
            Ok(s) if !s.trim().is_empty() => s,
            _ => {
                let _ = tokio::fs::remove_file(&claimed_path).await;
                continue;
            }
        };

        let parsed = serde_json::from_str::<serde_json::Value>(&raw)
            .or_else(|_| serde_json::from_str::<serde_json::Value>(&raw.replace("\\\"", "\"")));
        if let Ok(cmd) = parsed {
            processed_count += 1;
            let cmd_type = cmd
                .get("type")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            if matches!(cmd_type, "task_create" | "task_update" | "task_transfer") {
                let command_result = process_channel_task_command(
                    session_key,
                    agent_id,
                    work_dir,
                    gateway_base_url,
                    event_store,
                    &cmd,
                )
                .await;
                command_results.push(command_result);
                let _ = tokio::fs::remove_file(&claimed_path).await;
                continue;
            }
            if let Some(text) = process_single_outbox_command(
                session_key,
                agent_id,
                work_dir,
                gateway_base_url,
                event_store,
                &cmd,
            )
            .await
            {
                if !text.is_empty() {
                    replies.push(text);
                }
            }
        }
        let _ = tokio::fs::remove_file(&claimed_path).await;
    }

    OutboxProcessResult {
        reply: replies.join("\n\n"),
        processed_count,
        command_results,
    }
}

async fn claim_outbox_file(path: &Path) -> Option<std::path::PathBuf> {
    let extension = path.extension().and_then(|x| x.to_str());
    match extension {
        Some("json") => {
            let file_name = path.file_name()?.to_string_lossy();
            let claimed_path = path.with_file_name(format!(
                "{}.{}.processing",
                file_name,
                choruz_ids::MessageId::new()
            ));
            tokio::fs::rename(path, &claimed_path).await.ok()?;
            refresh_file_mtime(&claimed_path).await;
            Some(claimed_path)
        }
        Some("processing") if is_stale_processing_file(path) => {
            let file_name = path.file_name()?.to_string_lossy();
            let claimed_path = path.with_file_name(format!(
                "{}.retry-{}.processing",
                file_name,
                choruz_ids::MessageId::new()
            ));
            tokio::fs::rename(path, &claimed_path).await.ok()?;
            refresh_file_mtime(&claimed_path).await;
            Some(claimed_path)
        }
        _ => None,
    }
}

async fn refresh_file_mtime(path: &Path) {
    let path = path.to_path_buf();
    let _ = tokio::task::spawn_blocking(move || {
        let file = std::fs::OpenOptions::new().write(true).open(&path)?;
        file.set_times(
            std::fs::FileTimes::new()
                .set_accessed(SystemTime::now())
                .set_modified(SystemTime::now()),
        )
    })
    .await;
}

fn is_stale_processing_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    SystemTime::now()
        .duration_since(modified)
        .map(|age| age >= PROCESSING_STALE_AFTER)
        .unwrap_or(false)
}

async fn send_to_group(
    session_key: &str,
    agent_id: &str,
    store: &EventStore,
    group_name: &str,
    content: &str,
    content_type: &str,
    metadata: serde_json::Value,
) -> Result<(), String> {
    let client = match store.connect().await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(session_key, group = group_name, error = %e, "send_to_group: DB connect failed");
            return Err(format!("database connection failed: {e}"));
        }
    };

    // Resolve group name → conversation_id within the sender agent's
    // workspace. Agents must address groups by name, not conversation ID; a
    // UUID-shaped group name is still valid because this is a name lookup.
    let rows = match client
        .query(
            "SELECT c.id
             FROM conversation c
             JOIN principal p ON p.id = $2
             JOIN conversation_member cm
               ON cm.conv_id = c.id
              AND cm.principal_id = p.id
              AND cm.removed_at IS NULL
             WHERE c.name = $1
               AND c.type = 'group'
               AND c.workspace_id = p.workspace_id
               AND p.deleted_at IS NULL
             ORDER BY c.created_at ASC
             LIMIT 2",
            &[&group_name, &agent_id],
        )
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(session_key, group = group_name, error = %e, "send_to_group: lookup failed");
            return Err(format!("group lookup failed for '{}': {e}", group_name));
        }
    };
    let conv_id: String = match rows.as_slice() {
        [row] => row.get(0),
        [] => {
            tracing::warn!(
                session_key,
                group = group_name,
                "send_to_group: group not found or agent is not a member"
            );
            return Err(format!(
                "group '{}' was not found in this workspace, or this agent is not a member",
                group_name
            ));
        }
        _ => {
            tracing::warn!(
                session_key,
                group = group_name,
                "send_to_group: ambiguous group name"
            );
            return Err(format!(
                "group name '{}' is ambiguous in this workspace",
                group_name
            ));
        }
    };

    // Write to conversation_events + event_outbox in a transaction
    let message_id = choruz_ids::MessageId::new().to_string();
    let event_type = "message";
    let content_opt: Option<&str> = Some(content);
    let client_msg_id: Option<&str> = None;
    let turn_id: Option<&str> = None;

    // Thread discriminators, parsed by
    // the shared choruz-store helper so this write path and
    // DbService::send_message cannot drift. The outbox `send` command's
    // `thread` param was already folded into metadata by the caller; here
    // we mirror send_message's write-path semantics: canonicalize the
    // root inside the lock, and gate the unread bump for quiet
    // (non-broadcast) replies.
    let thread_flags = choruz_store::ThreadFlags::from_metadata(&metadata);
    let is_thread_reply = thread_flags.is_thread_reply;
    let is_broadcast = thread_flags.is_broadcast;
    let thread_target: Option<String> = metadata
        .get("reply_to_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    if is_thread_reply && thread_target.is_none() {
        return Err("threaded send requires a thread target (reply_to_id)".to_string());
    }

    let mut client = match store.connect().await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(session_key, error = %e, "send_to_group: DB connect for write failed");
            return Err(format!(
                "database connection failed while sending to group: {e}"
            ));
        }
    };

    let tx = match client.transaction().await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(session_key, error = %e, "send_to_group: begin tx failed");
            return Err(format!("failed to start group send transaction: {e}"));
        }
    };

    // Serialize concurrent writers targeting the same conversation so the
    // COALESCE(MAX(seq), 0) + 1 allocation below cannot race to the same
    // value and collide on the (conversation_id, seq) unique constraint.
    if let Err(e) = tx
        .execute(
            "SELECT pg_advisory_xact_lock(hashtext($1)::bigint)",
            &[&conv_id],
        )
        .await
    {
        tracing::warn!(session_key, group = group_name, error = %e, "send_to_group: advisory lock failed");
        tx.rollback().await.ok();
        return Err(format!(
            "failed to lock group '{}' for send: {e}",
            group_name
        ));
    }

    let active_member = match tx
        .query_opt(
            "SELECT 1
             FROM conversation c
             JOIN principal p ON p.id = $2
             JOIN conversation_member cm
               ON cm.conv_id = c.id
              AND cm.principal_id = p.id
              AND cm.removed_at IS NULL
             WHERE c.id = $1
               AND c.type = 'group'
               AND c.workspace_id = p.workspace_id
               AND p.deleted_at IS NULL
             LIMIT 1",
            &[&conv_id, &agent_id],
        )
        .await
    {
        Ok(row) => row.is_some(),
        Err(e) => {
            tracing::warn!(session_key, group = group_name, error = %e, "send_to_group: membership recheck failed");
            tx.rollback().await.ok();
            return Err(format!(
                "failed to verify group membership for '{}': {e}",
                group_name
            ));
        }
    };
    if !active_member {
        tracing::warn!(
            session_key,
            group = group_name,
            "send_to_group: membership recheck denied send"
        );
        tx.rollback().await.ok();
        return Err(format!(
            "group '{}' was not found in this workspace, or this agent is not a member",
            group_name
        ));
    }

    // Canonicalize the thread root inside the advisory lock — shared
    // helper with DbService::send_message, so agent thread replies obey
    // the same flat-thread + scoping rules as human ones.
    let reply_event_id: Option<String> = if is_thread_reply {
        let target = thread_target.as_deref().unwrap_or_default();
        match choruz_store::EventStore::canonicalize_thread_root_in_tx(&*tx, &conv_id, target).await
        {
            Ok(Some(root)) => Some(root),
            Ok(None) => {
                tx.rollback().await.ok();
                return Err(format!(
                    "thread target '{}' was not found in group '{}'",
                    target, group_name
                ));
            }
            Err(e) => {
                tracing::warn!(session_key, group = group_name, error = %e, "send_to_group: thread canonicalization failed");
                tx.rollback().await.ok();
                return Err(format!("failed to resolve thread target: {e}"));
            }
        }
    } else {
        None
    };

    let seq: i64 = match tx
        .query_one(
            "INSERT INTO conversation_events
                (conversation_id, seq, event_id, event_type, sender_id,
                 content, content_type, metadata, client_msg_id, turn_id,
                 reply_event_id, created_at)
             VALUES (
                $1,
                COALESCE((SELECT MAX(seq) FROM conversation_events WHERE conversation_id = $1), 0) + 1,
                $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW()
             )
             RETURNING seq",
            &[
                &conv_id, &message_id, &event_type, &agent_id,
                &content_opt, &content_type, &metadata,
                &client_msg_id, &turn_id, &reply_event_id,
            ],
        )
        .await
    {
        Ok(row) => row.get(0),
        Err(e) => {
            tracing::warn!(session_key, group = group_name, error = %e, "send_to_group: insert event failed");
            tx.rollback().await.ok();
            return Err(format!("failed to insert group message for '{}': {e}", group_name));
        }
    };

    let outbox_payload = serde_json::json!({
        "message_id": message_id,
        "conversation_id": conv_id,
        "sender_id": agent_id,
        "content": content,
        "content_type": content_type,
        "seq": seq,
        "metadata": metadata,
    });
    let aggregate_type = "conversation_event";
    let outbox_event_type = "message";

    if let Err(e) = tx
        .execute(
            "INSERT INTO event_outbox
                (aggregate_type, aggregate_id, event_type, payload, created_at, published)
             VALUES ($1, $2, $3, $4, NOW(), FALSE)",
            &[
                &aggregate_type,
                &conv_id,
                &outbox_event_type,
                &outbox_payload,
            ],
        )
        .await
    {
        tracing::warn!(session_key, error = %e, "send_to_group: insert outbox failed");
        tx.rollback().await.ok();
        return Err(format!(
            "failed to enqueue group message for '{}': {e}",
            group_name
        ));
    }

    // Increment conversation.total_msg_count so unread badges fire for
    // other members when the agent sends a message through the outbox.
    // (Mattermost pattern — matches what DbService::send_message does.)
    //
    // Quiet (non-broadcast) threaded replies skip the bump — same gate
    // as DbService::send_message: thread traffic must not light up the
    // conversation badge; thread-level unread covers it instead.
    let bumps_conversation_unread = thread_flags.bumps_conversation_unread();
    if bumps_conversation_unread
        && let Err(e) = tx
            .execute(
                "UPDATE conversation SET total_msg_count = total_msg_count + 1 WHERE id = $1",
                &[&conv_id],
            )
            .await
    {
        tracing::warn!(session_key, error = %e, "send_to_group: increment total_msg_count failed");
        tx.rollback().await.ok();
        return Err(format!(
            "failed to update group '{}' message count: {e}",
            group_name
        ));
    }

    if let Err(e) = tx.commit().await {
        tracing::warn!(session_key, error = %e, "send_to_group: commit failed");
        return Err(format!(
            "failed to commit group send for '{}': {e}",
            group_name
        ));
    }

    tracing::info!(
        session_key,
        group = group_name,
        conv_id = %conv_id,
        seq,
        content_len = content.len(),
        is_thread_reply,
        is_broadcast,
        thread_root = reply_event_id.as_deref().unwrap_or(""),
        "send_to_group: message delivered"
    );
    process_group_send_workflow_metadata(
        session_key,
        store,
        &conv_id,
        agent_id,
        &message_id,
        &metadata,
    )
    .await;
    Ok(())
}

async fn process_group_send_workflow_metadata(
    session_key: &str,
    store: &EventStore,
    conversation_id: &str,
    actor_id: &str,
    source_message_id: &str,
    metadata: &serde_json::Value,
) {
    let Some(workflow) = metadata.get("workflow").and_then(|value| value.as_object()) else {
        return;
    };
    let Some(kind) = workflow
        .get("kind")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|kind| !kind.is_empty())
    else {
        return;
    };
    let payload = serde_json::Value::Object(workflow.clone());
    let task_key = workflow
        .get("task_key")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let db = choruz_application::DbService::new(store.clone());
    let request = choruz_application::AppendGroupWorkflowEventRequest {
        kind: kind.to_string(),
        task_key,
        source_message_id: Some(source_message_id.to_string()),
        actor_principal_id: None,
        payload,
    };
    let result = db
        .append_group_workflow_event_for_conversation(conversation_id, actor_id, request)
        .await;
    if let Err(error) = result {
        tracing::warn!(
            session_key,
            conversation_id,
            actor_id,
            source_message_id,
            workflow_kind = kind,
            error = %error,
            "send_to_group: workflow metadata processing failed"
        );
    }
}

fn metadata_for_group_send_command(
    cmd: &serde_json::Value,
) -> Result<serde_json::Value, &'static str> {
    let mut metadata = match cmd.get("metadata") {
        None => serde_json::json!({}),
        Some(metadata) if metadata.is_object() => metadata.clone(),
        Some(_) => return Err("metadata must be a JSON object."),
    };

    // Thread support.
    // An agent replies into a thread with:
    //   {"type":"send","group":"…","content":"…","thread":"<root_event_id>"}
    // Optional "broadcast": false demotes the reply to thread-only.
    //
    // Broadcast DEFAULTS TO TRUE for agent thread replies — operators
    // keep full main-timeline visibility of agent activity unless the
    // agent explicitly opts a noisy intermediate update out.
    if let Some(thread_value) = cmd.get("thread") {
        let Some(thread_root) = thread_value.as_str().filter(|s| !s.is_empty()) else {
            return Err("thread must be a non-empty message id string.");
        };
        // Strict like the `thread` field above: a non-boolean broadcast
        // (e.g. the string "false") must error, not silently coerce to
        // the default — coercion would invert an explicit quiet-reply
        // intent into a broadcast.
        let broadcast = match cmd.get("broadcast") {
            None => true,
            Some(serde_json::Value::Bool(b)) => *b,
            Some(_) => return Err("broadcast must be a JSON boolean."),
        };
        let obj = metadata
            .as_object_mut()
            .expect("metadata validated as object above");
        obj.insert(
            "reply_to_id".to_string(),
            serde_json::Value::String(thread_root.to_string()),
        );
        obj.insert("thread".to_string(), serde_json::Value::Bool(true));
        obj.insert("broadcast".to_string(), serde_json::Value::Bool(broadcast));
    }

    Ok(metadata)
}

/// Process a single outbox command (used by the Maildir scanner).
///
/// Return contract:
/// - `Some(text)` — writer should commit `text` as the agent's reply (an empty
///   string means "already delivered elsewhere, writer should skip the insert").
/// - `None` — command was handled as a side effect (or was unknown); the
///   scanner should continue to the next file.
async fn process_single_outbox_command(
    session_key: &str,
    agent_id: &str,
    work_dir: &Path,
    gateway_base_url: &str,
    event_store: Option<&EventStore>,
    cmd: &serde_json::Value,
) -> Option<String> {
    let cmd_type = cmd.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match cmd_type {
        "send" => {
            let content = cmd.get("content").and_then(|c| c.as_str()).unwrap_or("");
            let group = cmd.get("group").and_then(|g| g.as_str()).unwrap_or("");
            if content.is_empty() {
                return None;
            }
            if !group.is_empty() {
                let metadata = match metadata_for_group_send_command(cmd) {
                    Ok(metadata) => metadata,
                    Err(error) => {
                        return Some(format!("Failed to send to group '{}': {}", group, error));
                    }
                };
                if let Some(store) = event_store {
                    if let Err(error) = send_to_group(
                        session_key,
                        agent_id,
                        store,
                        group,
                        content,
                        "text/plain",
                        metadata,
                    )
                    .await
                    {
                        return Some(format!("Failed to send to group '{}': {}", group, error));
                    }
                } else {
                    return Some(format!(
                        "Failed to send to group '{}': event store is unavailable.",
                        group
                    ));
                }
                // Delivered straight to the group — writer should NOT echo it
                // again as a reply in the originating conversation.
                Some(String::new())
            } else {
                // DM: let the writer commit `content` as the agent's reply.
                Some(content.to_string())
            }
        }
        "provision_agent" => {
            let agent_name = cmd.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let driver = cmd
                .get("driver_type")
                .or_else(|| cmd.get("driver"))
                .and_then(|v| v.as_str())
                .unwrap_or("claude_terminal");
            let instructions = cmd
                .get("instructions")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let model = cmd.get("model").and_then(|v| v.as_str());
            if agent_name.is_empty() {
                tracing::warn!(session_key, "outbox provision_agent: missing name");
                return None;
            }
            let channel_visibility = match provision_agent_channel_visibility(cmd) {
                Ok(value) => value,
                Err(message) => {
                    return Some(format!("Failed to create agent '{agent_name}': {message}"));
                }
            };

            // Look up the requesting agent's workspace so the new agent is
            // created in the same company instead of operator's ws-local
            // (the previous behaviour silently misrouted agents and broke
            // create_group's name resolution downstream).
            let workspace_id = lookup_agent_workspace(event_store, agent_id).await;

            tracing::info!(
                session_key,
                agent_name,
                driver,
                ?workspace_id,
                "outbox: provisioning agent via gateway API"
            );
            let client = reqwest::Client::new();
            let payload = provision_agent_payload(
                agent_name,
                driver,
                instructions,
                workspace_id.as_deref(),
                channel_visibility == Some("internal"),
                model,
            );
            // Provision goes through Next.js API route (port 3100) which handles
            // the full multi-step flow: principal + token + workspace + binding.
            // Login through the gateway and pass the session token as the
            // choruz_session cookie expected by the Next.js API route.
            let web_base = web_base_url_for_outbox(gateway_base_url);
            let session_token = operator_session_token(gateway_base_url).await;
            let req = if let Some(ref token) = session_token {
                client
                    .post(format!("{}/api/agents/provision", web_base))
                    .header("Cookie", format!("choruz_session={}", token))
            } else {
                tracing::warn!(
                    session_key,
                    "outbox: could not get session token for provision"
                );
                client.post(format!("{}/api/agents/provision", web_base))
            }
            .header(
                "x-choruz-internal-provision-token",
                internal_provision_token(),
            );
            match req.json(&payload).send().await {
                Ok(resp) if resp.status().is_success() => {
                    tracing::info!(
                        session_key,
                        agent_name,
                        "outbox: agent provisioned successfully"
                    );
                    Some(format!("Agent '{}' created successfully.", agent_name))
                }
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    tracing::warn!(session_key, agent_name, %status, body, "outbox: provision_agent API call failed");
                    Some(format!(
                        "Failed to create agent '{}': {} {}",
                        agent_name, status, body
                    ))
                }
                Err(e) => {
                    tracing::warn!(session_key, agent_name, error = %e, "outbox: provision_agent HTTP error");
                    Some(format!("Failed to create agent '{}': {}", agent_name, e))
                }
            }
        }
        "share_file" => {
            let rel_path = cmd.get("path").and_then(|v| v.as_str()).unwrap_or("");
            if rel_path.is_empty() {
                tracing::warn!(session_key, "outbox share_file: missing path");
                return None;
            }
            let group = cmd.get("group").and_then(|v| v.as_str()).unwrap_or("");

            // Path traversal guard. The path string comes from the agent's
            // outbox JSON — untrusted. Reject absolute paths, reject any
            // component that would climb out of the workspace, and
            // canonicalize both sides to defeat symlink-based escape.
            let rel = std::path::Path::new(rel_path);
            if rel.is_absolute()
                || rel
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
                || rel_path.contains('\0')
            {
                tracing::warn!(
                    session_key,
                    rel_path,
                    "outbox share_file: rejected path (absolute / .. / NUL)"
                );
                return None;
            }
            let ws_root = match tokio::fs::canonicalize(&work_dir).await {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!(session_key, work_dir = %work_dir.display(), error = %e, "outbox share_file: canonicalize workspace failed");
                    return None;
                }
            };
            let file_path = match tokio::fs::canonicalize(work_dir.join(rel_path)).await {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!(session_key, rel_path, error = %e, "outbox share_file: file not found or not readable");
                    return None;
                }
            };
            if !file_path.starts_with(&ws_root) {
                tracing::warn!(
                    session_key,
                    rel_path,
                    resolved = %file_path.display(),
                    ws = %ws_root.display(),
                    "outbox share_file: path escapes workspace — rejected"
                );
                return None;
            }

            match tokio::fs::read(&file_path).await {
                Ok(bytes) => {
                    let filename = std::path::Path::new(rel_path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("file");
                    let ext = std::path::Path::new(filename)
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    // Detect binary: any null byte, or a known binary extension.
                    // Matches the heuristic that the retired runner used (commit
                    // 6ea2253) before runner was merged into pipeline.
                    let is_binary = bytes.contains(&0)
                        || matches!(
                            ext.as_str(),
                            "png"
                                | "jpg"
                                | "jpeg"
                                | "gif"
                                | "webp"
                                | "svg"
                                | "pdf"
                                | "zip"
                                | "tar"
                                | "gz"
                                | "mp4"
                                | "mp3"
                                | "wav"
                                | "ogg"
                                | "wasm"
                                | "exe"
                                | "bin"
                                | "ico"
                        );

                    if !is_binary {
                        // Text: embed in reply body as a markdown code block
                        // (same as the pre-runner-merge behaviour).
                        tracing::info!(
                            session_key,
                            rel_path,
                            group,
                            size = bytes.len(),
                            "outbox: sharing text file"
                        );
                        return Some(format!(
                            "**{}**\n```\n{}\n```",
                            filename,
                            String::from_utf8_lossy(&bytes),
                        ));
                    }

                    // Binary: upload to /v1/attachments, then post to the group
                    // with an attachment-typed message so message-bubble.tsx
                    // renders <img>/<video>/<audio> inline.
                    let mime = match ext.as_str() {
                        "png" => "image/png",
                        "jpg" | "jpeg" => "image/jpeg",
                        "gif" => "image/gif",
                        "webp" => "image/webp",
                        "svg" => "image/svg+xml",
                        "pdf" => "application/pdf",
                        "mp4" => "video/mp4",
                        "mp3" => "audio/mpeg",
                        "wav" => "audio/wav",
                        "ogg" => "audio/ogg",
                        _ => "application/octet-stream",
                    };
                    use base64::Engine;
                    let data_b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

                    // Look up the agent's own bearer token from
                    // .choruz-runtime/agent_tokens.json. require_actor on
                    // /v1/attachments enforces principal.id == actor_id with
                    // no principal-type bypass, so acting as the agent from an
                    // operator session would 401. Loading the agent's secret
                    // here is the same pattern the retired runner used.
                    let tokens_path = std::env::var("CHORUZ_AGENT_TOKENS_FILE")
                        .unwrap_or_else(|_| ".choruz-runtime/agent_tokens.json".into());
                    let token = match tokio::fs::read_to_string(&tokens_path).await {
                        Ok(raw) => serde_json::from_str::<serde_json::Value>(&raw)
                            .ok()
                            .and_then(|v| v.get(agent_id)?.as_str().map(String::from)),
                        Err(e) => {
                            tracing::warn!(session_key, tokens_path, error = %e, "outbox share_file: failed to read agent tokens file");
                            None
                        }
                    };
                    let Some(token) = token else {
                        tracing::warn!(
                            session_key,
                            agent_id,
                            "outbox share_file: no agent token found for upload"
                        );
                        return None;
                    };

                    // Upload attachment
                    let upload = reqwest::Client::new()
                        .post(format!("{}/v1/attachments", gateway_base_url))
                        .bearer_auth(&token)
                        .json(&serde_json::json!({
                            "actor_id": agent_id,
                            "filename": filename,
                            "content_type": mime,
                            "data_base64": data_b64,
                        }))
                        .send()
                        .await;
                    let attachment: serde_json::Value = match upload {
                        Ok(r) if r.status().is_success() => r.json().await.unwrap_or_default(),
                        Ok(r) => {
                            let status = r.status();
                            let body = r.text().await.unwrap_or_default();
                            tracing::warn!(session_key, rel_path, %status, body, "outbox share_file: attachment upload failed");
                            return None;
                        }
                        Err(e) => {
                            tracing::warn!(session_key, rel_path, error = %e, "outbox share_file: attachment upload HTTP error");
                            return None;
                        }
                    };
                    let att_id = attachment.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let download_path = attachment
                        .get("download_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    let size = bytes.len();
                    let metadata = serde_json::json!({
                        "attachment_id": att_id,
                        "filename": filename,
                        "mime_type": mime,
                        "size_bytes": size,
                        "download_path": download_path,
                    });
                    let caption = format!("📎 {} ({} bytes)", filename, size);

                    if group.is_empty() {
                        // DM: no send_to_group path; the writer has no way to
                        // attach metadata to its reply, so fall back to a
                        // plain markdown link. The attachment is still
                        // persisted and downloadable at `download_path`.
                        return Some(format!("📎 [{}]({})", filename, download_path));
                    }

                    if let Some(store) = event_store {
                        if let Err(error) = send_to_group(
                            session_key,
                            agent_id,
                            store,
                            group,
                            &caption,
                            "attachment",
                            metadata,
                        )
                        .await
                        {
                            return Some(format!(
                                "Failed to share file to group '{}': {}",
                                group, error
                            ));
                        }
                    } else {
                        return Some(format!(
                            "Failed to share file to group '{}': event store is unavailable.",
                            group
                        ));
                    }
                    tracing::info!(
                        session_key,
                        rel_path,
                        group,
                        size,
                        mime,
                        "outbox: shared binary file as attachment"
                    );
                    // Delivered to the group; tell the writer to skip its own
                    // reply so the message doesn't get echoed twice.
                    return Some(String::new());
                }
                Err(e) => {
                    tracing::warn!(session_key, rel_path, error = %e, "outbox share_file: failed to read file");
                    None
                }
            }
        }
        "create_group" => {
            let group_name = cmd.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if group_name.is_empty() {
                return None;
            }
            // Per CLAUDE.md the field is `members` and accepts NAMES. Older
            // agents may still send `member_ids` (raw IDs); accept both.
            let members_raw: Vec<String> = cmd
                .get("members")
                .or_else(|| cmd.get("member_ids"))
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            let workspace_id = match lookup_agent_workspace(event_store, agent_id).await {
                Some(ws) => ws,
                None => {
                    let message = format!(
                        "Failed to create group '{}': could not resolve agent workspace.",
                        group_name
                    );
                    tracing::warn!(
                        session_key,
                        group_name,
                        "outbox create_group: cannot resolve agent's workspace"
                    );
                    return Some(message);
                }
            };

            // Resolve entries as names before falling back to raw IDs; a valid
            // display name may itself be UUID-shaped.
            let mut member_ids =
                resolve_names_to_ids(event_store, &workspace_id, &members_raw).await;
            if !member_ids.contains(&agent_id.to_string()) {
                member_ids.push(agent_id.to_string());
            }
            // Agent token is required: gateway's /v1/groups requires
            // bearer auth scoped to the actor.
            let token = match read_agent_token(agent_id).await {
                Some(t) => t,
                None => {
                    let message = format!(
                        "Failed to create group '{}': no agent token found.",
                        group_name
                    );
                    tracing::warn!(
                        session_key,
                        agent_id,
                        "outbox create_group: no agent token; skip"
                    );
                    return Some(message);
                }
            };

            tracing::info!(
                session_key,
                group_name,
                workspace_id,
                member_count = member_ids.len(),
                "outbox: creating group via gateway"
            );
            let payload = serde_json::json!({
                "actor_id": agent_id,
                "name": group_name,
                "description": cmd.get("description").and_then(|v| v.as_str()).unwrap_or(""),
                "member_ids": member_ids,
                "workspace_id": workspace_id,
            });
            match reqwest::Client::new()
                .post(format!("{}/v1/groups", gateway_base_url))
                .bearer_auth(&token)
                .json(&payload)
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    tracing::info!(
                        session_key,
                        group_name,
                        "outbox: group created successfully"
                    );
                    None
                }
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    tracing::warn!(session_key, group_name, %status, body, "outbox: create_group failed");
                    Some(format!(
                        "Failed to create group '{}': {} {}",
                        group_name, status, body
                    ))
                }
                Err(e) => {
                    tracing::warn!(session_key, group_name, error = %e, "outbox: create_group HTTP error");
                    Some(format!("Failed to create group '{}': {}", group_name, e))
                }
            }
        }
        "set_cron" => {
            let cron_name = cmd
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unnamed");
            let schedule = cmd.get("schedule").and_then(|v| v.as_str()).unwrap_or("");
            let cron_message = cmd.get("message").and_then(|v| v.as_str()).unwrap_or("");
            if schedule.is_empty() || cron_message.is_empty() {
                return None;
            }
            if let Some(store) = event_store {
                if let Ok(client) = store.connect().await {
                    let id = choruz_common::new_id();
                    // Determine schedule_type from format
                    let schedule_type = if schedule.contains(' ') {
                        "cron"
                    } else {
                        "every"
                    };
                    let next_run = compute_next_run_simple(schedule_type, schedule);
                    let Some(conv_id) =
                        resolve_cron_conversation_id(&client, session_key, agent_id).await
                    else {
                        return Some(format!(
                            "Failed to create cron job '{}': could not resolve conversation.",
                            cron_name
                        ));
                    };
                    match client.execute(
                        "INSERT INTO agent_cron_job (id, agent_id, conversation_id, name, schedule_type, schedule_value, message, next_run_at)
                         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                        &[&id, &agent_id, &conv_id, &cron_name, &schedule_type, &schedule, &cron_message, &next_run],
                    ).await {
                        Ok(_) => tracing::info!(session_key, name = cron_name, schedule, "cron job created via outbox"),
                        Err(e) => tracing::warn!(session_key, name = cron_name, schedule, error = %e, "set_cron: INSERT agent_cron_job failed"),
                    }
                }
            }
            None
        }
        // Channel-task commands (`task_create` / `task_update` / `task_transfer`) are dispatched
        // by the fast path in `process_outbox_commands_with_stats` so their structured envelopes
        // are pushed into `OutboxProcessResult.command_results`. Routing them here would build a
        // valid envelope, drop it on the floor, and silently bypass the result batch — so we
        // intentionally do not match them in this single-command helper.
        other => {
            if !other.is_empty() {
                tracing::warn!(
                    session_key,
                    cmd_type = other,
                    "outbox: unknown command type"
                );
            }
            None
        }
    }
}

async fn process_channel_task_command(
    session_key: &str,
    agent_id: &str,
    work_dir: &Path,
    gateway_base_url: &str,
    event_store: Option<&EventStore>,
    cmd: &serde_json::Value,
) -> serde_json::Value {
    let command_type = cmd
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    if !choruz_common::plugins::plugin_enabled(choruz_common::plugins::KANBAN_PLUGIN_ID) {
        let failure = channel_task_command_failure(
            command_type,
            "channel_tasks_disabled",
            choruz_common::plugins::KANBAN_PLUGIN_DISABLED_DETAIL,
            cmd,
        );
        persist_outbox_command_result(work_dir, &failure).await;
        return failure;
    }
    let conversation_id =
        match resolve_task_command_conversation_id(session_key, agent_id, event_store, cmd).await {
            Ok(conversation_id) => conversation_id,
            Err(failure) => {
                persist_outbox_command_result(work_dir, &failure).await;
                return failure;
            }
        };
    let token = match read_agent_token(agent_id).await {
        Some(token) => token,
        None => {
            let failure = channel_task_command_failure(
                command_type,
                "agent_token_unavailable",
                "No agent token found for channel task command.",
                cmd,
            );
            persist_outbox_command_result(work_dir, &failure).await;
            return failure;
        }
    };
    let client = reqwest::Client::new();
    let result_context: serde_json::Value;
    let response = match command_type {
        "task_create" => {
            result_context = cmd.clone();
            let payload =
                match channel_task_create_payload(agent_id, event_store, &conversation_id, cmd)
                    .await
                {
                    Ok(payload) => payload,
                    Err(failure) => {
                        persist_outbox_command_result(work_dir, &failure).await;
                        return failure;
                    }
                };
            client
                .post(format!(
                    "{}/v1/conversations/{}/tasks",
                    gateway_base_url, conversation_id
                ))
                .bearer_auth(&token)
                .json(&payload)
                .send()
                .await
        }
        "task_update" | "task_transfer" => {
            let task_ref = match resolve_channel_task_ref(
                session_key,
                agent_id,
                gateway_base_url,
                &client,
                &token,
                &conversation_id,
                cmd,
            )
            .await
            {
                Ok(task_ref) => task_ref,
                Err(failure) => {
                    persist_outbox_command_result(work_dir, &failure).await;
                    return failure;
                }
            };
            result_context = channel_task_command_context_with_task_ref(cmd, &task_ref);
            let payload = match channel_task_patch_payload(
                agent_id,
                event_store,
                &conversation_id,
                &result_context,
            )
            .await
            {
                Ok(payload) => payload,
                Err(failure) => {
                    persist_outbox_command_result(work_dir, &failure).await;
                    return failure;
                }
            };
            client
                .patch(format!(
                    "{}/v1/tasks/{}",
                    gateway_base_url, task_ref.task_id
                ))
                .bearer_auth(&token)
                .json(&payload)
                .send()
                .await
        }
        _ => {
            let failure = channel_task_command_failure(
                command_type,
                "unsupported_command",
                "Unsupported channel task command.",
                cmd,
            );
            persist_outbox_command_result(work_dir, &failure).await;
            return failure;
        }
    };
    let result = match response {
        Ok(resp) if resp.status().is_success() => {
            let task: serde_json::Value = resp.json().await.unwrap_or_default();
            serde_json::json!({
                "command_type": command_type,
                "ok": true,
                "task_key": task.get("task_key").and_then(|value| value.as_str())
                    .or_else(|| result_context.get("task_key").and_then(|value| value.as_str())),
                "task_id": task.get("task_id").and_then(|value| value.as_str())
                    .or_else(|| result_context.get("task_id").and_then(|value| value.as_str())),
                "idempotency_key": result_context.get("idempotency_key").and_then(|value| value.as_str()),
                "emitted_at": current_emitted_at(),
            })
        }
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            tracing::warn!(
                session_key,
                agent_id,
                command_type,
                %status,
                body,
                "outbox: channel task gateway call failed"
            );
            let message = extract_gateway_error_detail(&body)
                .unwrap_or_else(|| format!("Channel task command failed with {status}."));
            channel_task_command_failure(
                command_type,
                channel_task_error_code_for_status(status.as_u16()),
                &message,
                &result_context,
            )
        }
        Err(error) => {
            tracing::warn!(
                session_key,
                agent_id,
                command_type,
                error = %error,
                "outbox: channel task gateway HTTP error"
            );
            channel_task_command_failure(
                command_type,
                "gateway_unavailable",
                &format!("Channel task gateway request failed: {error}"),
                &result_context,
            )
        }
    };
    persist_outbox_command_result(work_dir, &result).await;
    result
}

async fn resolve_task_command_conversation_id(
    session_key: &str,
    agent_id: &str,
    event_store: Option<&EventStore>,
    cmd: &serde_json::Value,
) -> Result<String, serde_json::Value> {
    let command_type = cmd
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    if let Some(conversation_id) = cmd.get("conversation_id").and_then(|value| value.as_str())
        && !conversation_id.trim().is_empty()
    {
        return Ok(conversation_id.trim().to_string());
    }
    let Some(group_name) = cmd
        .get("group")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Err(channel_task_command_failure(
            command_type,
            "missing_target",
            "Channel task commands require group or conversation_id.",
            cmd,
        ));
    };
    let Some(store) = event_store else {
        return Err(channel_task_command_failure(
            command_type,
            "event_store_unavailable",
            "Cannot resolve group name for channel task command without an event store.",
            cmd,
        ));
    };
    resolve_group_conversation_id(session_key, agent_id, store, group_name)
        .await
        .map_err(|message| {
            channel_task_command_failure(command_type, "group_not_found", &message, cmd)
        })
}

async fn resolve_group_conversation_id(
    session_key: &str,
    agent_id: &str,
    store: &EventStore,
    group_name: &str,
) -> Result<String, String> {
    let client = store.connect().await.map_err(|error| {
        tracing::warn!(session_key, group = group_name, error = %error, "task command: DB connect failed");
        format!("database connection failed: {error}")
    })?;
    let rows = client
        .query(
            "SELECT c.id
             FROM conversation c
             JOIN principal p ON p.id = $2
             JOIN conversation_member cm
               ON cm.conv_id = c.id
              AND cm.principal_id = p.id
              AND cm.removed_at IS NULL
             WHERE c.name = $1
               AND c.type = 'group'
               AND c.workspace_id = p.workspace_id
               AND p.deleted_at IS NULL
             ORDER BY c.created_at ASC
             LIMIT 2",
            &[&group_name, &agent_id],
        )
        .await
        .map_err(|error| {
            tracing::warn!(session_key, group = group_name, error = %error, "task command: group lookup failed");
            format!("group lookup failed for '{group_name}': {error}")
        })?;
    match rows.as_slice() {
        [row] => Ok(row.get(0)),
        [] => Err(format!(
            "group '{group_name}' was not found in this workspace, or this agent is not a member"
        )),
        _ => Err(format!(
            "group name '{group_name}' is ambiguous in this workspace"
        )),
    }
}

async fn agent_is_active_conversation_member(
    store: &EventStore,
    conversation_id: &str,
    agent_id: &str,
) -> bool {
    let Ok(client) = store.connect().await else {
        return false;
    };
    client
        .query_one(
            "SELECT EXISTS (
               SELECT 1
               FROM conversation c
               JOIN principal p ON p.id = $2
               JOIN conversation_member cm
                 ON cm.conv_id = c.id
                AND cm.principal_id = p.id
                AND cm.removed_at IS NULL
               WHERE c.id = $1
                 AND c.workspace_id = p.workspace_id
                 AND p.deleted_at IS NULL
             ) AS allowed",
            &[&conversation_id, &agent_id],
        )
        .await
        .map(|row| row.get::<_, bool>("allowed"))
        .unwrap_or(false)
}

async fn channel_task_create_payload(
    agent_id: &str,
    event_store: Option<&EventStore>,
    conversation_id: &str,
    cmd: &serde_json::Value,
) -> Result<serde_json::Value, serde_json::Value> {
    let command_type = cmd
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let mut payload = serde_json::json!({
        "title": cmd.get("title").and_then(|value| value.as_str()).unwrap_or(""),
        "idempotency_key": cmd.get("idempotency_key").and_then(|value| value.as_str()).unwrap_or(""),
    });
    if let Some(task_key) = cmd
        .get("task_key")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        payload["task_key"] = serde_json::Value::String(task_key.to_string());
    }
    if let Some(status) = cmd.get("status").and_then(|value| value.as_str()) {
        payload["status"] = serde_json::Value::String(status.to_string());
    }
    if let Some(context_label) = cmd.get("context_label").and_then(|value| value.as_str()) {
        payload["context_label"] = serde_json::Value::String(context_label.to_string());
    }
    if let Some(assignee_id) = cmd
        .get("assignee_principal_id")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
    {
        let assignee_id =
            resolve_task_assignee_principal_id(agent_id, event_store, conversation_id, assignee_id)
                .await
                .ok_or_else(|| {
                    channel_task_command_failure(
                        command_type,
                        "invalid_assignee",
                        "Could not resolve task assignee in the agent workspace.",
                        cmd,
                    )
                })?;
        payload["assignee_principal_id"] = serde_json::Value::String(assignee_id);
        return Ok(payload);
    }
    if let Some(assignee) = cmd
        .get("assignee")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
    {
        let assignee_id = resolve_task_assignee(agent_id, event_store, conversation_id, assignee)
            .await
            .ok_or_else(|| {
                channel_task_command_failure(
                    command_type,
                    "invalid_assignee",
                    "Could not resolve task assignee in the agent workspace.",
                    cmd,
                )
            })?;
        payload["assignee_principal_id"] = serde_json::Value::String(assignee_id);
    }
    Ok(payload)
}

async fn channel_task_patch_payload(
    agent_id: &str,
    event_store: Option<&EventStore>,
    conversation_id: &str,
    cmd: &serde_json::Value,
) -> Result<serde_json::Value, serde_json::Value> {
    let command_type = cmd
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let mut payload = serde_json::json!({});
    if let Some(status) = cmd.get("status").and_then(|value| value.as_str()) {
        payload["status"] = serde_json::Value::String(status.to_string());
    }
    if let Some(blocked_reason) = cmd.get("blocked_reason") {
        payload["blocked_reason"] = blocked_reason.clone();
    }
    if let Some(context_label) = cmd.get("context_label") {
        payload["context_label"] = context_label.clone();
    }
    if command_type == "task_transfer" {
        let assignee_id =
            channel_task_command_assignee_id(agent_id, event_store, conversation_id, cmd).await?;
        payload["assignee_principal_id"] = serde_json::Value::String(assignee_id);
    }
    Ok(payload)
}

async fn channel_task_command_assignee_id(
    agent_id: &str,
    event_store: Option<&EventStore>,
    conversation_id: &str,
    cmd: &serde_json::Value,
) -> Result<String, serde_json::Value> {
    let command_type = cmd
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    if let Some(assignee_id) = cmd
        .get("assignee_principal_id")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
    {
        return resolve_task_assignee_principal_id(
            agent_id,
            event_store,
            conversation_id,
            assignee_id,
        )
        .await
        .ok_or_else(|| {
            channel_task_command_failure(
                command_type,
                "invalid_assignee",
                "Could not resolve task assignee in the agent workspace.",
                cmd,
            )
        });
    }
    let Some(assignee) = cmd
        .get("assignee")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
    else {
        return Err(channel_task_command_failure(
            command_type,
            "missing_assignee",
            "task_transfer requires assignee or assignee_principal_id.",
            cmd,
        ));
    };
    resolve_task_assignee(agent_id, event_store, conversation_id, assignee)
        .await
        .ok_or_else(|| {
            channel_task_command_failure(
                command_type,
                "invalid_assignee",
                "Could not resolve task assignee in the agent workspace.",
                cmd,
            )
        })
}

struct ResolvedChannelTaskRef {
    task_id: String,
    task_key: Option<String>,
}

fn channel_task_command_context_with_task_ref(
    cmd: &serde_json::Value,
    task_ref: &ResolvedChannelTaskRef,
) -> serde_json::Value {
    let mut context = cmd.clone();
    if let Some(object) = context.as_object_mut() {
        object.insert(
            "task_id".to_string(),
            serde_json::Value::String(task_ref.task_id.clone()),
        );
        if let Some(task_key) = &task_ref.task_key {
            object.insert(
                "task_key".to_string(),
                serde_json::Value::String(task_key.clone()),
            );
        }
    }
    context
}

async fn resolve_channel_task_ref(
    session_key: &str,
    agent_id: &str,
    gateway_base_url: &str,
    client: &reqwest::Client,
    token: &str,
    conversation_id: &str,
    cmd: &serde_json::Value,
) -> Result<ResolvedChannelTaskRef, serde_json::Value> {
    let command_type = cmd
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let requested_task_id = cmd
        .get("task_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let requested_task_key = cmd
        .get("task_key")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if requested_task_id.is_none() && requested_task_key.is_none() {
        return Err(channel_task_command_failure(
            command_type,
            "missing_task",
            "task_update and task_transfer require task_id or task_key.",
            cmd,
        ));
    }
    let response = client
        .get(format!(
            "{}/v1/conversations/{}/tasks",
            gateway_base_url, conversation_id
        ))
        .bearer_auth(token)
        .send()
        .await;
    match response {
        Ok(resp) if resp.status().is_success() => {
            let tasks: serde_json::Value = resp.json().await.unwrap_or_default();
            tasks
                .as_array()
                .and_then(|items| {
                    items.iter().find_map(|task| {
                        let task_id = task.get("task_id").and_then(|value| value.as_str())?;
                        let task_key = task.get("task_key").and_then(|value| value.as_str());
                        let id_matches = requested_task_id
                            .map(|requested| requested == task_id)
                            .unwrap_or(true);
                        let key_matches = requested_task_key
                            .map(|requested| task_key == Some(requested))
                            .unwrap_or(true);
                        (id_matches && key_matches).then(|| ResolvedChannelTaskRef {
                            task_id: task_id.to_string(),
                            task_key: task_key.map(str::to_string),
                        })
                    })
                })
                .ok_or_else(|| {
                    let message = match (requested_task_key, requested_task_id) {
                        (Some(task_key), _) => {
                            format!("No task with key {task_key} exists in this conversation.")
                        }
                        (None, Some(task_id)) => {
                            format!("No task with id {task_id} exists in this conversation.")
                        }
                        (None, None) => {
                            "task_update and task_transfer require task_id or task_key.".to_string()
                        }
                    };
                    channel_task_command_failure(command_type, "task_not_found", &message, cmd)
                })
        }
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            tracing::warn!(
                session_key,
                agent_id,
                command_type,
                %status,
                body,
                "outbox: channel task lookup failed"
            );
            let message = extract_gateway_error_detail(&body)
                .unwrap_or_else(|| format!("Channel task lookup failed with {status}."));
            Err(channel_task_command_failure(
                command_type,
                channel_task_error_code_for_status(status.as_u16()),
                &message,
                cmd,
            ))
        }
        Err(error) => {
            tracing::warn!(
                session_key,
                agent_id,
                command_type,
                error = %error,
                "outbox: channel task lookup HTTP error"
            );
            Err(channel_task_command_failure(
                command_type,
                "gateway_unavailable",
                &format!("Channel task gateway request failed: {error}"),
                cmd,
            ))
        }
    }
}

async fn resolve_task_assignee(
    agent_id: &str,
    event_store: Option<&EventStore>,
    conversation_id: &str,
    assignee: &str,
) -> Option<String> {
    let trimmed = assignee.trim_start_matches('@').trim();
    let roster = task_assignee_roster(agent_id, event_store, conversation_id, trimmed).await?;
    let matches = roster
        .into_iter()
        .filter(|entry| {
            entry.display_name.eq_ignore_ascii_case(trimmed) || entry.principal_id == trimmed
        })
        .map(|entry| entry.principal_id)
        .collect::<Vec<_>>();
    unique_task_assignee_match(agent_id, conversation_id, trimmed, matches)
}

async fn resolve_task_assignee_principal_id(
    agent_id: &str,
    event_store: Option<&EventStore>,
    conversation_id: &str,
    assignee_id: &str,
) -> Option<String> {
    let trimmed = assignee_id.trim();
    let roster = task_assignee_roster(agent_id, event_store, conversation_id, trimmed).await?;
    let matches = roster
        .into_iter()
        .filter(|entry| entry.principal_id == trimmed)
        .map(|entry| entry.principal_id)
        .collect::<Vec<_>>();
    unique_task_assignee_match(agent_id, conversation_id, trimmed, matches)
}

async fn task_assignee_roster(
    agent_id: &str,
    event_store: Option<&EventStore>,
    conversation_id: &str,
    assignee: &str,
) -> Option<Vec<AssigneeRosterEntry>> {
    let store = event_store?;
    if conversation_id.is_empty() {
        tracing::warn!(
            agent_id,
            assignee = %assignee,
            "task command assignee resolution requires a conversation-scoped roster"
        );
        return None;
    }
    if !agent_is_active_conversation_member(store, conversation_id, agent_id).await {
        tracing::warn!(
            agent_id,
            conversation_id,
            assignee = %assignee,
            "task command assignee resolution requires active conversation membership"
        );
        return None;
    }
    let provider = PgMemberProvider::new(store.clone());
    provider.list_assignee_roster(conversation_id).await.ok()
}

fn unique_task_assignee_match(
    agent_id: &str,
    conversation_id: &str,
    assignee: &str,
    matches: Vec<String>,
) -> Option<String> {
    match matches.as_slice() {
        [id] => Some(id.clone()),
        [] => None,
        _ => {
            tracing::warn!(
                agent_id,
                conversation_id,
                assignee = %assignee,
                "task command assignee resolved to multiple roster entries"
            );
            None
        }
    }
}

fn extract_gateway_error_detail(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(|error| error.get("detail"))
                .or_else(|| value.get("detail"))
                .and_then(|detail| detail.as_str())
                .map(str::to_string)
        })
}

fn channel_task_error_code_for_status(status: u16) -> &'static str {
    match status {
        400 => "validation_failed",
        401 => "unauthorized",
        403 => "forbidden",
        404 => "not_found",
        409 => "idempotency_conflict",
        _ => "gateway_error",
    }
}

fn internal_provision_token() -> String {
    std::env::var("CHORUZ_INTERNAL_PROVISION_TOKEN").unwrap_or_default()
}

fn provision_agent_payload(
    agent_name: &str,
    driver: &str,
    instructions: &str,
    workspace_id: Option<&str>,
    internal: bool,
    model: Option<&str>,
) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "name": agent_name,
        "driver_type": driver,
        "instructions": instructions,
    });
    if let Some(ws) = workspace_id {
        payload["workspace_id"] = serde_json::Value::String(ws.to_string());
    }
    if internal {
        payload["channel_visibility"] = serde_json::Value::String("internal".to_string());
    }
    if let Some(model) = model.filter(|model| !model.trim().is_empty()) {
        payload["model"] = serde_json::Value::String(model.trim().to_string());
    }
    payload
}

fn provision_agent_channel_visibility(
    command: &serde_json::Value,
) -> Result<Option<&str>, &'static str> {
    match command.get("channel_visibility") {
        None => Ok(None),
        Some(serde_json::Value::String(value))
            if matches!(value.as_str(), "visible" | "internal") =>
        {
            Ok(Some(value))
        }
        Some(_) => Err("channel_visibility must be 'visible' or 'internal'."),
    }
}

async fn persist_outbox_command_result(work_dir: &Path, result: &serde_json::Value) {
    let results_dir = work_dir.join(".choruz-outbox").join("results");
    if let Err(error) = tokio::fs::create_dir_all(&results_dir).await {
        tracing::warn!(
            results_dir = %results_dir.display(),
            error = %error,
            "outbox: failed to create command result directory"
        );
        return;
    }
    let final_name = format!("{}.json", choruz_ids::MessageId::new());
    let final_path = results_dir.join(&final_name);
    // Write to a `.partial-<id>` sibling first and rename into place so a concurrent reader
    // listing the dir cannot observe a half-written JSON file. POSIX `rename` is atomic within
    // a single filesystem, which the results dir always is.
    let tmp_path = results_dir.join(format!(
        "{final_name}.partial-{}",
        choruz_ids::MessageId::new()
    ));
    if let Err(error) = tokio::fs::write(&tmp_path, result.to_string()).await {
        tracing::warn!(
            tmp_path = %tmp_path.display(),
            error = %error,
            "outbox: failed to write command result tempfile"
        );
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return;
    }
    if let Err(error) = tokio::fs::rename(&tmp_path, &final_path).await {
        tracing::warn!(
            tmp_path = %tmp_path.display(),
            final_path = %final_path.display(),
            error = %error,
            "outbox: failed to rename command result into place"
        );
        let _ = tokio::fs::remove_file(&tmp_path).await;
    }
}

fn channel_task_command_failure(
    command_type: &str,
    error_code: &str,
    message: &str,
    cmd: &serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "command_type": command_type,
        "ok": false,
        "error_code": error_code,
        "message": sanitize_command_result_message(message),
        "task_key": cmd.get("task_key").and_then(|value| value.as_str()),
        "task_id": cmd.get("task_id").and_then(|value| value.as_str()),
        "idempotency_key": cmd.get("idempotency_key").and_then(|value| value.as_str()),
        "emitted_at": current_emitted_at(),
    })
}

/// RFC3339 UTC timestamp the envelope was constructed. Lets agents disambiguate the current
/// turn's results from older files left in `.choruz-outbox/results/` and gives operators a
/// monotonic ordering signal even if the maildir id allocator ever changes.
fn current_emitted_at() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Scrub message strings before they reach the persisted envelope. Static internal messages
/// (e.g. "Channel task commands require group or conversation_id.") never trigger these rules,
/// so this is purely a defense-in-depth wrapper around `extract_gateway_error_detail` output —
/// upstream gateway bodies are user-influenceable (echoed assignee names, validation payloads)
/// and could surface a bearer token or `Authorization:` header verbatim through `error.detail`.
/// The replacement is irreversible; we lose some fidelity in the recovery hint but guarantee
/// the persisted file is safe for agents that will later read, log, or share it.
fn sanitize_command_result_message(message: &str) -> String {
    use regex::Regex;
    use std::sync::LazyLock;
    static PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
        vec![
            Regex::new(r"(?i)bearer\s+[A-Za-z0-9._\-+/=]+").unwrap(),
            Regex::new(r"(?i)authorization\s*[:=]\s*\S+").unwrap(),
            Regex::new(r"(?i)\b(?:session[_-]?)?token\b\s*[:=]\s*\S+").unwrap(),
            Regex::new(r#"(?i)"(?:session[_-]?)?token"\s*:\s*"[^"]+""#).unwrap(),
            Regex::new(r"(?i)agent[_-]?token\s*[:=]\s*\S+").unwrap(),
            Regex::new(r"(?i)password\s*[:=]\s*\S+").unwrap(),
            Regex::new(r"(?i)secret\s*[:=]\s*\S+").unwrap(),
            Regex::new(r"(?i)private[_-]?key\s*[:=]\s*\S+").unwrap(),
        ]
    });
    let mut scrubbed = message.to_string();
    for pattern in PATTERNS.iter() {
        scrubbed = pattern.replace_all(&scrubbed, "[redacted]").into_owned();
    }
    scrubbed
}

async fn resolve_cron_conversation_id(
    client: &tokio_postgres::Client,
    session_key: &str,
    agent_id: &str,
) -> Option<String> {
    if let Some(binding_id) = session_key.strip_prefix("watcher:") {
        return client
            .query_opt(
                "SELECT conversation_id
                 FROM agent_runtime_bindings
                 WHERE id = $1 AND agent_principal_id = $2",
                &[&binding_id, &agent_id],
            )
            .await
            .ok()
            .flatten()
            .map(|row| row.get(0));
    }

    session_key
        .split_once(':')
        .map(|(_, conv_id)| conv_id.to_string())
}

/// Simple next-run computation for outbox-created cron jobs.
pub fn compute_next_run_simple(
    schedule_type: &str,
    schedule_value: &str,
) -> Option<chrono::DateTime<chrono::Utc>> {
    let now = chrono::Utc::now();
    match schedule_type {
        "every" => {
            let s = schedule_value.trim();
            if s.is_empty() {
                return None;
            }
            let (num_str, unit) = s.split_at(s.len() - 1);
            let num: i64 = num_str.parse().ok()?;
            let duration = match unit {
                "s" => chrono::Duration::seconds(num),
                "m" => chrono::Duration::minutes(num),
                "h" => chrono::Duration::hours(num),
                "d" => chrono::Duration::days(num),
                _ => {
                    let num: i64 = s.parse().ok()?;
                    chrono::Duration::minutes(num)
                }
            };
            Some(now + duration)
        }
        "cron" => {
            // Schedule first run in 1 minute for cron expressions
            Some(now + chrono::Duration::minutes(1))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests;

/// Look up an agent's workspace_id from the principal table.
async fn lookup_agent_workspace(
    event_store: Option<&EventStore>,
    agent_id: &str,
) -> Option<String> {
    let store = event_store?;
    let client = store.connect().await.ok()?;
    let row = client
        .query_opt(
            "SELECT workspace_id FROM principal WHERE id = $1 AND deleted_at IS NULL",
            &[&agent_id],
        )
        .await
        .ok()??;
    Some(row.get(0))
}

/// For each entry: resolve as a principal name in the given workspace first.
/// If no name matches and it looks like an ID, pass it through as a raw ID.
/// Unresolvable names are dropped (logged) — caller decides whether to abort.
async fn resolve_names_to_ids(
    event_store: Option<&EventStore>,
    workspace_id: &str,
    raw: &[String],
) -> Vec<String> {
    let mut out = Vec::with_capacity(raw.len());
    let store = match event_store {
        Some(s) => s,
        None => return raw.to_vec(),
    };
    let client = match store.connect().await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "resolve_names_to_ids: DB connect failed");
            return raw.to_vec();
        }
    };
    for entry in raw {
        let trimmed = entry.trim_start_matches('@').trim();
        match client
            .query_opt(
                "SELECT id FROM principal
                 WHERE workspace_id = $1 AND lower(name) = lower($2) AND deleted_at IS NULL
                 LIMIT 1",
                &[&workspace_id, &trimmed],
            )
            .await
        {
            Ok(Some(row)) => {
                let id: String = row.get(0);
                out.push(id);
            }
            Ok(None) => {
                if is_principal_id_like(trimmed) {
                    out.push(trimmed.to_string());
                    continue;
                }
                tracing::warn!(
                    workspace_id,
                    name = trimmed,
                    "resolve_names_to_ids: no principal in workspace"
                );
            }
            Err(e) => {
                tracing::warn!(workspace_id, name = trimmed, error = %e, "resolve_names_to_ids: query failed");
            }
        }
    }
    out
}

fn is_principal_id_like(value: &str) -> bool {
    let mut parts = value.split('-');
    let Some(a) = parts.next() else {
        return false;
    };
    let Some(b) = parts.next() else {
        return false;
    };
    let Some(c) = parts.next() else {
        return false;
    };
    let Some(d) = parts.next() else {
        return false;
    };
    let Some(e) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }

    [a, b, c, d, e].map(str::len) == [8, 4, 4, 4, 12]
        && value.chars().all(|ch| ch == '-' || ch.is_ascii_hexdigit())
}

/// Read the agent's bearer token from `.choruz-runtime/agent_tokens.json`. Same
/// pattern share_file already uses; centralised here so create_group and
/// any future agent-acting outbox commands can reuse it.
async fn read_agent_token(agent_id: &str) -> Option<String> {
    let tokens_path = std::env::var("CHORUZ_AGENT_TOKENS_FILE")
        .unwrap_or_else(|_| ".choruz-runtime/agent_tokens.json".into());
    let raw = tokio::fs::read_to_string(&tokens_path).await.ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v.get(agent_id)?.as_str().map(String::from)
}
