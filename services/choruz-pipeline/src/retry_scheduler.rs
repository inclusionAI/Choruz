//! Retry scheduler: sweeps stale pending commands (TTL), dead-letters
//! exhausted commands, and re-dispatches retriable ones.

use choruz_session::PgSessionStore;

/// Run the retry scheduler loop until cancelled.
pub async fn run_retry_loop(
    session_store: PgSessionStore,
    retry_interval: std::time::Duration,
    retry_batch: i64,
) {
    tracing::info!("retry scheduler started");
    let mut interval = tokio::time::interval(retry_interval);

    loop {
        interval.tick().await;

        // 0. TTL sweep: dead-letter commands pending for more than 24 hours
        // (matches executor_timeout_secs — commands can queue for a long time
        // when many agents are being created sequentially)
        match session_store
            .dead_letter_stale_pending_commands(86400, retry_batch)
            .await
        {
            Ok(n) if n > 0 => {
                tracing::warn!(
                    count = n,
                    "retry scheduler: dead-lettered stale pending commands (TTL expired)"
                );
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "retry scheduler: failed to sweep stale pending commands"
                );
            }
            _ => {}
        }

        // 1. Dead-letter exhausted commands (attempt_count >= max_attempts)
        match session_store
            .find_exhausted_retry_commands(retry_batch)
            .await
        {
            Ok(exhausted) => {
                for cmd in &exhausted {
                    tracing::warn!(
                        command_id = %cmd.command_id,
                        attempt = cmd.attempt_count,
                        max = cmd.max_attempts,
                        "retry scheduler: dead-lettering exhausted command"
                    );
                    let update = choruz_session::CommandStatusUpdate {
                        command_id: cmd.command_id.clone(),
                        status: choruz_session::CommandStatus::DeadLetter,
                        last_error: Some("max retry attempts exhausted".into()),
                        ..Default::default()
                    };
                    if let Err(e) = session_store.update_command_status(&update).await {
                        tracing::error!(
                            command_id = %cmd.command_id,
                            error = %e,
                            "retry scheduler: failed to dead-letter command"
                        );
                    } else {
                        let dl = choruz_session::InsertDeadLetter {
                            source_type: "command".to_string(),
                            source_id: cmd.command_id.clone(),
                            payload: serde_json::json!({
                                "session_key": cmd.session_key,
                                "agent_id": cmd.agent_id,
                                "attempt_count": cmd.attempt_count,
                                "last_error": cmd.last_error,
                            }),
                            error: "max retry attempts exhausted".to_string(),
                            attempt_count: cmd.attempt_count,
                        };
                        if let Err(e) = session_store.insert_dead_letter(&dl).await {
                            tracing::error!(
                                command_id = %cmd.command_id,
                                error = %e,
                                "retry scheduler: failed to insert dead letter"
                            );
                        }
                    }
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "retry scheduler: failed to find exhausted commands");
            }
        }

        // 2. Re-dispatch retriable commands (attempt_count < max_attempts)
        let now = chrono::Utc::now();
        let retriable = match session_store
            .find_retriable_commands(now, retry_batch)
            .await
        {
            Ok(cmds) => cmds,
            Err(e) => {
                tracing::error!(error = %e, "retry scheduler: failed to find retriable commands");
                continue;
            }
        };

        for cmd in &retriable {
            tracing::info!(
                command_id = %cmd.command_id,
                attempt = cmd.attempt_count,
                "retry scheduler: resetting command to pending"
            );

            // Reset the command to pending so the dispatch loop picks it up
            let update = choruz_session::CommandStatusUpdate {
                command_id: cmd.command_id.clone(),
                status: choruz_session::CommandStatus::Pending,
                ..Default::default()
            };
            if let Err(e) = session_store.update_command_status(&update).await {
                tracing::error!(
                    command_id = %cmd.command_id,
                    error = %e,
                    "retry scheduler: failed to reset command to pending"
                );
            }
        }
    }
}
