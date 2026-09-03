//! Dispatch loop: polls pending commands, assigns leases, executes via
//! the persistent session executor, and forwards results to the writer.
//!
//! Per-agent batching: when an idle agent has multiple pending commands
//! (typical during a fan-out @-mention cascade), they are merged into a
//! single `claude --print` spawn whose prompt concatenates every queued
//! message. The agent reads the whole inbox at once and produces one
//! coherent reply. All commands in the batch share the same execution and
//! all transition together (leased → succeeded or → retry_scheduled).
//! This is what claude-code's `InProcessTeammate` does via the `Stop hook`
//! and what Choruz's pre-v2 inbox model did before spawn-per-message
//! replaced it.

use std::collections::HashMap;
use std::sync::Arc;

use choruz_session::{
    AgentCommand, CommandStatus, CommandStatusUpdate, InsertDeadLetter, PgSessionStore,
    SessionError,
};
use choruz_writer::{AgentResult, CommandAttemptRef};
use tokio::sync::mpsc;

use crate::config::DISPATCH_HEARTBEAT_INTERVAL_SECS;
use crate::executor::{ExecutorContext, execute_command, is_auto_retriable_error};

fn should_retry_failed_command(cmd: &AgentCommand, auto_retriable: bool) -> bool {
    auto_retriable && !choruz_session::is_exhausted(cmd.attempt_count, cmd.max_attempts)
}

/// Build the prompt for a batched dispatch of N >= 2 messages for the
/// same agent. For N == 1 the single command's prompt is returned unchanged.
///
/// Format: numbered brackets per message so the agent can address each
/// sender separately without losing that they arrived as one inbox sweep.
fn build_batched_prompt(group: &[AgentCommand]) -> String {
    if group.len() == 1 {
        return group[0].prompt.clone();
    }
    let n = group.len();
    let approx_size = group.iter().map(|c| c.prompt.len() + 32).sum::<usize>() + 256;
    let mut out = String::with_capacity(approx_size);
    out.push_str(&format!(
        "[choruz-batch] You have {n} pending messages waiting for you. \
         They are ordered oldest to newest. Read them all before acting, \
         reconstruct the latest state of each topic, and let later messages \
         supersede earlier instructions about the same work. Never restart, \
         reopen, or redelegate work that a later message or task snapshot \
         marks completed. Then reply once per unresolved topic via \
         `.choruz/send`; combine senders asking about the same topic.\n\n"
    ));
    for (i, cmd) in group.iter().enumerate() {
        out.push_str(&format!("[{}/{}] {}\n\n", i + 1, n, cmd.prompt));
    }
    out
}

/// Run the dispatch loop until cancelled.
///
/// This continuously polls for pending commands, assigns leases, spawns
/// per-command execution tasks (with heartbeat), and sends results to the
/// writer channel.
pub async fn run_dispatch_loop(
    session_store: PgSessionStore,
    executor_ctx: Arc<ExecutorContext>,
    writer_tx: mpsc::Sender<AgentResult>,
    dispatch_wake: Arc<tokio::sync::Notify>,
    dispatch_interval: std::time::Duration,
    dispatch_batch: i64,
    dispatch_node_id: String,
) {
    tracing::info!("dispatch loop started (persistent session executor)");
    let mut interval = tokio::time::interval(dispatch_interval);

    loop {
        // Wake on NOTIFY or fallback timer
        tokio::select! {
            _ = dispatch_wake.notified() => {}
            _ = interval.tick() => {}
        }

        // Find pending commands
        let commands = match session_store.find_pending_commands(dispatch_batch).await {
            Ok(cmds) => cmds,
            Err(e) => {
                tracing::error!(error = %e, "dispatch: failed to find pending commands");
                continue;
            }
        };

        if commands.is_empty() {
            continue;
        }

        tracing::info!(count = commands.len(), "dispatch: found pending commands");

        // Group by agent_id, preserving FIFO order within each group
        // (SQL already orders by created_at ASC).
        let mut groups: HashMap<String, Vec<AgentCommand>> = HashMap::new();
        for cmd in commands {
            groups.entry(cmd.agent_id.clone()).or_default().push(cmd);
        }

        for (agent_id, group) in groups {
            // Pick the oldest command as the "primary" — its session_key,
            // metadata, turn_id are the ones that carry the AgentResult.
            // Secondaries share the same execution; we mark each of their
            // DB rows succeeded/retried in lockstep after the primary returns.
            let primary = group[0].clone();
            let batch_size = group.len();

            // Build the batched prompt (no-op if batch_size == 1) and
            // hand it to the executor via primary.prompt.
            let batched_prompt = build_batched_prompt(&group);
            let mut primary_for_exec = primary.clone();
            primary_for_exec.prompt = batched_prompt;

            // Assign the whole batch in one store transaction. If another
            // dispatcher already leased any member, the entire batch rolls
            // back so this dispatcher never executes or updates rows it does
            // not own.
            let command_ids: Vec<String> = group.iter().map(|cmd| cmd.command_id.clone()).collect();
            let leases = match session_store
                .assign_batch_leases(&command_ids, &dispatch_node_id)
                .await
            {
                Ok(leases) => leases,
                Err(e) => {
                    tracing::warn!(
                        agent_id = %agent_id,
                        batch_size,
                        error = %e,
                        "dispatch: failed to assign atomic batch lease"
                    );
                    continue;
                }
            };
            let Some(assignments) = group
                .iter()
                .map(|cmd| leases.get(&cmd.command_id).cloned())
                .collect::<Option<Vec<_>>>()
            else {
                tracing::error!(
                    primary_command_id = %primary.command_id,
                    agent_id = %agent_id,
                    "dispatch: batch lease assignment omitted a command; skipping batch"
                );
                continue;
            };
            let lease = assignments[0].clone();

            tracing::info!(
                primary_command_id = %primary.command_id,
                batch_size,
                epoch = lease.epoch,
                attempt_id = %lease.attempt_id,
                agent_id = %agent_id,
                "dispatch: batch lease assigned, executing"
            );

            primary_for_exec.current_attempt_id = Some(lease.attempt_id.clone());
            primary_for_exec.current_epoch = Some(lease.epoch);
            primary_for_exec.attempt_count = lease.attempt_count;

            // Transition the primary command to `started` (the batched
            // execution is "started" the moment we spawn). Secondaries
            // stay `leased` until the batch finishes — they don't have
            // their own claude process so `started` would be misleading.
            {
                let update = choruz_session::CommandStatusUpdate {
                    command_id: primary.command_id.clone(),
                    status: choruz_session::CommandStatus::Started,
                    ..Default::default()
                };
                if let Err(e) = session_store
                    .update_command_status_for_attempt(&update, &lease.attempt_id)
                    .await
                {
                    match e {
                        SessionError::StaleAttempt { .. } => tracing::warn!(
                            command_id = %primary.command_id,
                            attempt_id = %lease.attempt_id,
                            "dispatch: primary attempt was superseded before execution; dropping batch"
                        ),
                        error => tracing::error!(
                            command_id = %primary.command_id,
                            error = %error,
                            "dispatch: failed to persist started status; leaving lease for expiry recovery"
                        ),
                    }
                    continue;
                }
            }

            // Heartbeat keeps every leased session row alive while the
            // batched spawn is in flight. Cross-conversation batches may have
            // multiple session_keys, and each one must avoid lease expiry.
            let hb_session_store = session_store.clone();
            let mut heartbeat_sessions = HashMap::new();
            let mut conflicting_epoch = None;
            for (cmd, assignment) in group.iter().zip(&assignments) {
                if let Some(existing_epoch) =
                    heartbeat_sessions.insert(cmd.session_key.clone(), assignment.epoch)
                    && existing_epoch != assignment.epoch
                {
                    conflicting_epoch =
                        Some((cmd.session_key.clone(), existing_epoch, assignment.epoch));
                    break;
                }
            }
            if let Some((session_key, first_epoch, second_epoch)) = conflicting_epoch {
                tracing::error!(
                    session_key,
                    first_epoch,
                    second_epoch,
                    "dispatch: atomic batch returned conflicting epochs; leaving leases for expiry recovery"
                );
                continue;
            }
            let hb_command_id = primary.command_id.clone();
            let hb_attempt_id = lease.attempt_id.clone();
            let heartbeat_handle = tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(
                    DISPATCH_HEARTBEAT_INTERVAL_SECS as u64,
                ));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                // Tokio's first interval tick is immediate; the lease write
                // already supplied the initial heartbeat.
                interval.tick().await;
                let mut first_heartbeat = true;
                loop {
                    interval.tick().await;
                    let mut superseded_sessions = Vec::new();
                    for (session_key, epoch) in &heartbeat_sessions {
                        if let Err(e) = hb_session_store
                            .update_session_heartbeat_for_epoch(session_key, *epoch)
                            .await
                        {
                            match e {
                                error @ (SessionError::EpochMismatch { .. }
                                | SessionError::SessionInactive { .. }
                                | SessionError::SessionNotFound(_)) => {
                                    tracing::warn!(
                                        session_key = %session_key,
                                        error = %error,
                                        "dispatch: session lease was superseded; removing it from the batch heartbeat"
                                    );
                                    superseded_sessions.push(session_key.clone());
                                }
                                error => tracing::error!(
                                    session_key = %session_key,
                                    error = %error,
                                    "dispatch: transient session heartbeat failure; retrying on next interval"
                                ),
                            }
                        }
                    }
                    for session_key in superseded_sessions {
                        heartbeat_sessions.remove(&session_key);
                    }
                    if heartbeat_sessions.is_empty() {
                        // Lease loss does not cancel the CLI process. Its
                        // eventual result is rejected by attempt/epoch guards.
                        return;
                    }
                    if first_heartbeat {
                        first_heartbeat = false;
                        let update = choruz_session::CommandStatusUpdate {
                            command_id: hb_command_id.clone(),
                            status: choruz_session::CommandStatus::Heartbeating,
                            ..Default::default()
                        };
                        if let Err(e) = hb_session_store
                            .update_command_status_for_attempt(&update, &hb_attempt_id)
                            .await
                        {
                            match e {
                                SessionError::StaleAttempt { .. } => tracing::warn!(
                                    command_id = %hb_command_id,
                                    attempt_id = %hb_attempt_id,
                                    "dispatch: primary heartbeat attempt was superseded; stopping heartbeat"
                                ),
                                error => tracing::error!(
                                    command_id = %hb_command_id,
                                    error = %error,
                                    "dispatch: failed to persist heartbeating status; stopping heartbeat for lease expiry recovery"
                                ),
                            }
                            return;
                        }
                    }
                }
            });

            // Spawn the actual claude --print process for the batch.
            let spawn_executor = Arc::clone(&executor_ctx);
            let spawn_writer = writer_tx.clone();
            let spawn_session = session_store.clone();
            let mut spawn_group = group.clone();
            for (cmd, assignment) in spawn_group.iter_mut().zip(&assignments) {
                cmd.current_attempt_id = Some(assignment.attempt_id.clone());
                cmd.current_epoch = Some(assignment.epoch);
                cmd.attempt_count = assignment.attempt_count;
            }
            let spawn_primary_id = primary.command_id.clone();
            tokio::spawn(async move {
                let result = execute_command(&spawn_executor, &primary_for_exec).await;
                heartbeat_handle.abort();

                let succeeded = result.status == choruz_writer::AgentResultStatus::Succeeded;

                // All commands in the batch transition together.
                // - succeeded → every row marked succeeded; the writer
                //   picks up the single primary AgentResult, commits one
                //   reply_event, then closes both primary and secondary
                //   command lifecycles.
                // - retryable failure → every row uses bounded exponential
                //   backoff. Deterministic environment failures are terminal
                //   immediately so they cannot block this agent's queue.
                let mut committable_secondary = Vec::new();
                let ordered_group = spawn_group
                    .iter()
                    .find(|cmd| cmd.command_id == spawn_primary_id)
                    .into_iter()
                    .chain(
                        spawn_group
                            .iter()
                            .filter(|cmd| cmd.command_id != spawn_primary_id),
                    );
                if succeeded {
                    for cmd in ordered_group {
                        let Some(attempt_id) = cmd.current_attempt_id.as_deref() else {
                            tracing::error!(
                                command_id = %cmd.command_id,
                                primary = %spawn_primary_id,
                                "dispatch: leased batch member has no attempt id"
                            );
                            if cmd.command_id == spawn_primary_id {
                                return;
                            }
                            continue;
                        };
                        if let Err(e) = spawn_session
                            .mark_command_succeeded_for_attempt(&cmd.command_id, attempt_id)
                            .await
                        {
                            let stale = matches!(e, SessionError::StaleAttempt { .. });
                            match e {
                                SessionError::StaleAttempt { .. } => tracing::warn!(
                                    command_id = %cmd.command_id,
                                    primary = %spawn_primary_id,
                                    attempt_id,
                                    "dispatch: succeeded batch member was superseded; dropping stale result"
                                ),
                                error => tracing::error!(
                                    command_id = %cmd.command_id,
                                    primary = %spawn_primary_id,
                                    error = %error,
                                    "dispatch: failed to persist succeeded status; leaving lease for expiry recovery"
                                ),
                            }
                            // A transient persistence error may occur after
                            // PostgreSQL accepted the update. Let the writer's
                            // atomic attempt/epoch check and turn-id dedup
                            // settle that ambiguity; only proven staleness
                            // must discard the primary result.
                            if cmd.command_id == spawn_primary_id && stale {
                                return;
                            }
                            continue;
                        }
                        if cmd.command_id != spawn_primary_id {
                            committable_secondary.push(CommandAttemptRef {
                                command_id: cmd.command_id.clone(),
                                attempt_id: attempt_id.to_string(),
                            });
                        }
                    }
                } else {
                    let auto_retriable = is_auto_retriable_error(result.error.as_deref());
                    for cmd in ordered_group {
                        let Some(attempt_id) = cmd.current_attempt_id.as_deref() else {
                            tracing::error!(
                                command_id = %cmd.command_id,
                                primary = %spawn_primary_id,
                                "dispatch: leased batch member has no attempt id"
                            );
                            if cmd.command_id == spawn_primary_id {
                                return;
                            }
                            continue;
                        };
                        if should_retry_failed_command(cmd, auto_retriable) {
                            let next_retry = choruz_session::next_retry_at(
                                chrono::Utc::now(),
                                cmd.attempt_count,
                            );
                            let update = CommandStatusUpdate {
                                command_id: cmd.command_id.clone(),
                                status: CommandStatus::RetryScheduled,
                                last_error: result.error.clone(),
                                next_retry_at: Some(Some(next_retry)),
                                ..Default::default()
                            };
                            if let Err(e) = spawn_session
                                .update_command_status_for_attempt(&update, attempt_id)
                                .await
                            {
                                match e {
                                    SessionError::StaleAttempt { .. } => tracing::warn!(
                                        command_id = %cmd.command_id,
                                        primary = %spawn_primary_id,
                                        attempt_id,
                                        "dispatch: failed batch member was superseded; skipping stale retry"
                                    ),
                                    error => tracing::error!(
                                        command_id = %cmd.command_id,
                                        primary = %spawn_primary_id,
                                        error = %error,
                                        "dispatch: failed to persist retry schedule; leaving lease for expiry recovery"
                                    ),
                                }
                                if cmd.command_id == spawn_primary_id {
                                    return;
                                }
                            }
                            continue;
                        }

                        let error = result
                            .error
                            .clone()
                            .unwrap_or_else(|| "agent execution failed".into());
                        let dead_letter = InsertDeadLetter {
                            source_type: "command".into(),
                            source_id: cmd.command_id.clone(),
                            payload: serde_json::json!({
                                "session_key": cmd.session_key,
                                "agent_id": cmd.agent_id,
                                "attempt_count": cmd.attempt_count,
                                "last_error": error,
                            }),
                            error,
                            attempt_count: cmd.attempt_count,
                        };
                        if let Err(e) = spawn_session
                            .dead_letter_command_for_attempt(&dead_letter, attempt_id)
                            .await
                        {
                            match e {
                                SessionError::StaleAttempt { .. } => tracing::warn!(
                                    command_id = %cmd.command_id,
                                    primary = %spawn_primary_id,
                                    attempt_id,
                                    "dispatch: terminal batch member was superseded; skipping stale dead letter"
                                ),
                                error => tracing::error!(
                                    command_id = %cmd.command_id,
                                    primary = %spawn_primary_id,
                                    error = %error,
                                    "dispatch: failed to persist dead letter; leaving lease for expiry recovery"
                                ),
                            }
                            if cmd.command_id == spawn_primary_id {
                                return;
                            }
                        }
                    }
                }

                // One result → one reply_event for the whole batch.
                let mut result = result;
                if succeeded {
                    result.secondary_command_attempts = committable_secondary;
                }
                if spawn_writer.send(result).await.is_err() {
                    tracing::error!("dispatch: writer channel closed");
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use choruz_session::AgentCommand;

    fn mk_cmd(command_id: &str, agent_id: &str, prompt: &str) -> AgentCommand {
        AgentCommand {
            command_id: command_id.into(),
            route_id: "r".into(),
            session_key: format!("sk-{agent_id}"),
            agent_id: agent_id.into(),
            conversation_id: "c1".into(),
            message_id: "m1".into(),
            turn_id: format!("t-{command_id}"),
            status: choruz_session::CommandStatus::Pending,
            current_attempt_id: None,
            current_epoch: None,
            attempt_count: 0,
            max_attempts: choruz_session::DEFAULT_MAX_ATTEMPTS,
            prompt: prompt.into(),
            metadata: serde_json::json!({}),
            next_retry_at: None,
            last_error: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn retries_only_transient_executor_failures() {
        assert!(is_auto_retriable_error(Some(
            "headless CLI failed [kind=rate_limited]"
        )));
        assert!(is_auto_retriable_error(Some(
            "headless CLI resume failure [kind=resume_failure]"
        )));
        assert!(is_auto_retriable_error(Some(
            "headless CLI failed [kind=process_failed]"
        )));
        assert!(is_auto_retriable_error(Some("CLI timeout [kind=timeout]")));
        assert!(!is_auto_retriable_error(Some(
            "headless CLI failed [kind=auth]"
        )));
        assert!(!is_auto_retriable_error(Some(
            "headless CLI could not start [kind=driver_unavailable]"
        )));
        assert!(!is_auto_retriable_error(Some(
            "headless CLI failed [kind=configuration]"
        )));
    }

    #[test]
    fn final_retriable_attempt_is_dead_lettered_without_rescheduling() {
        let mut cmd = mk_cmd("c1", "A1", "hello");
        cmd.attempt_count = choruz_session::DEFAULT_MAX_ATTEMPTS;

        assert!(!should_retry_failed_command(&cmd, true));
    }

    // build_batched_prompt --------------------------------------------------

    #[test]
    fn batched_prompt_for_single_command_is_unchanged() {
        let cmd = mk_cmd("c1", "A1", "[choruz-incoming] from:@X | hello");
        let out = build_batched_prompt(std::slice::from_ref(&cmd));
        assert_eq!(out, cmd.prompt, "single-command batch must be a no-op");
    }

    #[test]
    fn batched_prompt_concatenates_all_messages_with_numbered_brackets() {
        let group = vec![
            mk_cmd("c1", "A", "[choruz-incoming] from:@X | first"),
            mk_cmd("c2", "A", "[choruz-incoming] from:@Y | second"),
            mk_cmd("c3", "A", "[choruz-incoming] from:@Z | third"),
        ];
        let out = build_batched_prompt(&group);
        assert!(out.starts_with("[choruz-batch] You have 3 pending messages"));
        assert!(out.contains("[1/3] [choruz-incoming] from:@X | first"));
        assert!(out.contains("[2/3] [choruz-incoming] from:@Y | second"));
        assert!(out.contains("[3/3] [choruz-incoming] from:@Z | third"));
    }

    #[test]
    fn batched_prompt_preserves_message_order() {
        let group = vec![
            mk_cmd("c1", "A", "FIRST"),
            mk_cmd("c2", "A", "SECOND"),
            mk_cmd("c3", "A", "THIRD"),
        ];
        let out = build_batched_prompt(&group);
        let first = out.find("FIRST").unwrap();
        let second = out.find("SECOND").unwrap();
        let third = out.find("THIRD").unwrap();
        assert!(first < second && second < third, "order must be preserved");
    }

    #[test]
    fn batched_prompt_requires_latest_state_before_action() {
        let group = vec![
            mk_cmd("c1", "A", "Start TASK-2"),
            mk_cmd("c2", "A", "TASK-2 is done"),
        ];
        let out = build_batched_prompt(&group);

        assert!(out.contains("ordered oldest to newest"));
        assert!(out.contains("later messages supersede earlier instructions"));
        assert!(out.contains("Never restart, reopen, or redelegate work"));
        assert!(out.contains("reply once per unresolved topic"));
    }

    #[test]
    fn batched_prompt_size_scales_linearly_with_group_size() {
        let single = mk_cmd("c1", "A", "x".repeat(100).as_str());
        let small = build_batched_prompt(std::slice::from_ref(&single));
        let group: Vec<AgentCommand> = (0..10)
            .map(|i| mk_cmd(&format!("c{i}"), "A", &"x".repeat(100)))
            .collect();
        let big = build_batched_prompt(&group);
        // 10× content + header overhead; the wrapper is bounded constant.
        assert!(big.len() > small.len() * 9);
    }

    // grouping logic (matches what the dispatcher does inline) -----------

    #[test]
    fn grouping_by_agent_id_distributes_commands_correctly() {
        let commands = vec![
            mk_cmd("c1", "A", "msg-A1"),
            mk_cmd("c2", "B", "msg-B1"),
            mk_cmd("c3", "A", "msg-A2"),
            mk_cmd("c4", "C", "msg-C1"),
            mk_cmd("c5", "A", "msg-A3"),
            mk_cmd("c6", "B", "msg-B2"),
        ];

        let mut groups: HashMap<String, Vec<AgentCommand>> = HashMap::new();
        for cmd in commands {
            groups.entry(cmd.agent_id.clone()).or_default().push(cmd);
        }

        assert_eq!(groups.len(), 3);
        assert_eq!(groups["A"].len(), 3);
        assert_eq!(groups["B"].len(), 2);
        assert_eq!(groups["C"].len(), 1);

        // Order within an agent group is preserved (matches SQL ORDER BY created_at).
        let a_prompts: Vec<&str> = groups["A"].iter().map(|c| c.prompt.as_str()).collect();
        assert_eq!(a_prompts, vec!["msg-A1", "msg-A2", "msg-A3"]);
    }
}
