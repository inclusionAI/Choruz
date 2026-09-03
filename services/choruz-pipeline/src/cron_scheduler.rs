//! Cron scheduler: checks every N seconds for due jobs and dispatches them
//! by inserting into `agent_commands` for the dispatch loop to pick up.

use choruz_session::PgSessionStore;
use choruz_store::EventStore;
use std::time::Duration;

const CRON_STALE_CLAIM_AFTER: &str = "10 minutes";

pub async fn run_cron_scheduler(
    event_store: EventStore,
    session_store: PgSessionStore,
    check_interval: Duration,
) {
    tracing::info!("cron scheduler started");
    let mut interval = tokio::time::interval(check_interval);

    loop {
        interval.tick().await;
        if let Err(e) = check_and_dispatch_due_jobs(&event_store, &session_store).await {
            tracing::error!(error = %e, "cron scheduler: check failed");
        }
    }
}

async fn check_and_dispatch_due_jobs(
    event_store: &EventStore,
    _session_store: &PgSessionStore,
) -> Result<(), String> {
    let client = event_store
        .connect()
        .await
        .map_err(|e| format!("connect: {e}"))?;

    // Atomically claim due jobs so multiple scheduler instances cannot enqueue
    // duplicate commands for the same occurrence. Stale claims are recoverable
    // because the due job's next_run_at remains the occurrence idempotency key.
    let rows = client
        .query(
            "UPDATE agent_cron_job
             SET running_at = COALESCE(running_at, NOW()), updated_at = NOW()
             WHERE id IN (
                 SELECT id
                 FROM agent_cron_job
                 WHERE enabled = true
                   AND (
                       running_at IS NULL
                       OR running_at < NOW() - $1::text::interval
                   )
                   AND next_run_at <= NOW()
                 ORDER BY next_run_at ASC
                 LIMIT 10
                 FOR UPDATE SKIP LOCKED
             )
             RETURNING id, agent_id, conversation_id, name, message, session_target,
                       delivery_mode, schedule_type, schedule_value,
                       schedule_timezone, timeout_seconds, running_at, next_run_at",
            &[&CRON_STALE_CLAIM_AFTER],
        )
        .await
        .map_err(|e| format!("claim due jobs: {e}"))?;

    if rows.is_empty() {
        return Ok(());
    }

    tracing::info!(count = rows.len(), "cron scheduler: found due jobs");

    for row in &rows {
        let job_id: String = row.get(0);
        let agent_id: String = row.get(1);
        let conversation_id: String = row.get(2);
        let name: String = row.get(3);
        let message: String = row.get(4);
        let _session_target: String = row.get(5);
        let delivery_mode: String = row.get(6);
        let schedule_type: String = row.get(7);
        let schedule_value: String = row.get(8);
        let schedule_timezone: Option<String> = row.get(9);
        let _timeout_seconds: i32 = row.get(10);
        let _claimed_at: chrono::DateTime<chrono::Utc> = row.get(11);
        let next_run_at: chrono::DateTime<chrono::Utc> = row.get(12);

        let occurrence_micros = next_run_at.timestamp_micros();
        let message_id = cron_message_id(&job_id, occurrence_micros);

        // Build prompt with cron context
        let prompt = format!(
            "[choruz-cron] job:{} schedule:{} | {}",
            name, schedule_value, message
        );

        // Determine session_key
        let session_key = format!("{}:{}", agent_id, conversation_id);

        let command_id = choruz_ids::CommandId::new().to_string();
        let turn_id = choruz_ids::TurnId::new().to_string();
        let route_id = choruz_ids::RouteId::new().to_string();
        let metadata = serde_json::json!({
            "cron_job_id": job_id,
            "cron_job_name": name,
        });

        match dispatch_cron_occurrence(
            event_store,
            CronDispatch {
                agent_id: &agent_id,
                conversation_id: &conversation_id,
                session_key: &session_key,
                message_id: &message_id,
                job_id: &job_id,
                job_name: &name,
                message: &message,
                announce: delivery_mode == "announce",
                command_id: &command_id,
                route_id: &route_id,
                turn_id: &turn_id,
                prompt: &prompt,
                metadata,
            },
        )
        .await
        {
            Ok(_) => {
                tracing::info!(
                    job_id = %job_id,
                    agent_id = %agent_id,
                    name = %name,
                    "cron: dispatched job"
                );
            }
            Err(e) => {
                tracing::error!(job_id = %job_id, error = %e, "cron: failed to dispatch occurrence");
                // Clear running_at on failure
                client
                    .execute(
                        "UPDATE agent_cron_job SET running_at = NULL WHERE id = $1",
                        &[&job_id],
                    )
                    .await
                    .ok();
                continue;
            }
        }

        // Calculate next_run_at based on schedule type
        let next_run = compute_next_run(
            &schedule_type,
            &schedule_value,
            schedule_timezone.as_deref(),
        );

        // Update job state
        if schedule_type == "at" {
            // One-shot: disable after run
            client
                .execute(
                    "UPDATE agent_cron_job SET running_at = NULL, last_run_at = NOW(),
                     enabled = false, next_run_at = NULL, updated_at = NOW() WHERE id = $1",
                    &[&job_id],
                )
                .await
                .ok();
        } else {
            // Recurring: update next_run_at
            client
                .execute(
                    "UPDATE agent_cron_job SET running_at = NULL, last_run_at = NOW(),
                     next_run_at = $2, updated_at = NOW() WHERE id = $1",
                    &[&job_id, &next_run],
                )
                .await
                .ok();
        }
    }

    Ok(())
}

struct CronDispatch<'a> {
    agent_id: &'a str,
    conversation_id: &'a str,
    session_key: &'a str,
    message_id: &'a str,
    job_id: &'a str,
    job_name: &'a str,
    message: &'a str,
    announce: bool,
    command_id: &'a str,
    route_id: &'a str,
    turn_id: &'a str,
    prompt: &'a str,
    metadata: serde_json::Value,
}

async fn dispatch_cron_occurrence(
    event_store: &EventStore,
    dispatch: CronDispatch<'_>,
) -> Result<(), String> {
    let CronDispatch {
        agent_id,
        conversation_id,
        session_key,
        message_id,
        job_id,
        job_name,
        message,
        announce,
        command_id,
        route_id,
        turn_id,
        prompt,
        metadata,
    } = dispatch;

    let mut client = event_store
        .connect()
        .await
        .map_err(|e| format!("connect: {e}"))?;
    let tx = client
        .transaction()
        .await
        .map_err(|e| format!("begin tx: {e}"))?;

    let idempotency_key = format!("{message_id}:{agent_id}");
    tx.execute(
        "SELECT pg_advisory_xact_lock(hashtext($1)::bigint)",
        &[&idempotency_key],
    )
    .await
    .map_err(|e| format!("lock cron command idempotency key: {e}"))?;

    tx.execute(
        "INSERT INTO session_registry (
            session_key, agent_id, conversation_id,
            epoch, status, created_at, updated_at
         ) VALUES ($1, $2, $3, 0, 'idle', NOW(), NOW())
         ON CONFLICT (session_key) DO UPDATE SET updated_at = NOW()",
        &[&session_key, &agent_id, &conversation_id],
    )
    .await
    .map_err(|e| format!("upsert session: {e}"))?;

    if announce {
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtext($1)::bigint)",
            &[&conversation_id],
        )
        .await
        .map_err(|e| format!("lock conversation: {e}"))?;

        let event_type = "message";
        let sender_id = "choruz-cron";
        let content_type = "text/plain";
        let content_opt: Option<&str> = Some(message);
        let event_metadata = serde_json::json!({
            "source": "cron_scheduler",
            "cron_job_id": job_id,
            "cron_job_name": job_name,
        });
        let client_msg_id: Option<&str> = None;
        let event_turn_id: Option<&str> = None;
        let reply_event_id: Option<&str> = None;

        let inserted = tx
            .query_opt(
                "INSERT INTO conversation_events
                    (conversation_id, seq, event_id, event_type, sender_id,
                     content, content_type, metadata, client_msg_id, turn_id,
                     reply_event_id, created_at)
                 VALUES (
                    $1,
                    COALESCE((SELECT MAX(seq) FROM conversation_events WHERE conversation_id = $1), 0) + 1,
                    $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW()
                 )
                 ON CONFLICT (event_id) DO NOTHING
                 RETURNING seq",
                &[
                    &conversation_id,
                    &message_id,
                    &event_type,
                    &sender_id,
                    &content_opt,
                    &content_type,
                    &event_metadata,
                    &client_msg_id,
                    &event_turn_id,
                    &reply_event_id,
                ],
            )
            .await
            .map_err(|e| format!("insert event: {e}"))?;

        if inserted.is_some() {
            tx.execute(
                "UPDATE conversation SET total_msg_count = total_msg_count + 1 WHERE id = $1",
                &[&conversation_id],
            )
            .await
            .map_err(|e| format!("update conversation count: {e}"))?;
        }
    }

    let existing = tx
        .query_opt(
            "SELECT command_id FROM agent_commands WHERE message_id = $1 AND agent_id = $2",
            &[&message_id, &agent_id],
        )
        .await
        .map_err(|e| format!("lookup existing command: {e}"))?;
    if existing.is_none() {
        tx.execute(
            "INSERT INTO agent_commands (
                command_id, route_id, session_key, agent_id,
                conversation_id, message_id, turn_id,
                status, attempt_count, max_attempts,
                prompt, metadata, created_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7,
                       'pending', 0, 3, $8, $9, NOW(), NOW())",
            &[
                &command_id,
                &route_id,
                &session_key,
                &agent_id,
                &conversation_id,
                &message_id,
                &turn_id,
                &prompt,
                &metadata,
            ],
        )
        .await
        .map_err(|e| format!("insert command: {e}"))?;
    }

    tx.commit().await.map_err(|e| format!("commit: {e}"))?;
    Ok(())
}

fn compute_next_run(
    schedule_type: &str,
    schedule_value: &str,
    _timezone: Option<&str>,
) -> Option<chrono::DateTime<chrono::Utc>> {
    let now = chrono::Utc::now();
    match schedule_type {
        "every" => {
            // Parse interval like "30m", "1h", "24h", "7d"
            let duration = parse_interval(schedule_value)?;
            Some(now + duration)
        }
        "cron" => {
            // For cron expressions, use a simple next-match calculation.
            // TODO: Add croner/cron-parser crate for proper cron expression support.
            // For now, approximate: daily jobs -> 24h, hourly -> 1h, minute -> 1m
            let parts: Vec<&str> = schedule_value.split_whitespace().collect();
            if parts.len() >= 5 {
                // Heuristic: if minute and hour are specific but day/month/dow are *,
                // it's a daily job. Otherwise fall back to 1h.
                let is_daily =
                    parts[0] != "*" && parts[1] != "*" && parts[2] == "*" && parts[3] == "*";
                if is_daily {
                    Some(now + chrono::Duration::hours(24))
                } else {
                    Some(now + chrono::Duration::hours(1))
                }
            } else {
                Some(now + chrono::Duration::hours(24))
            }
        }
        "at" => None, // One-shot, no next run
        _ => None,
    }
}

fn parse_interval(s: &str) -> Option<chrono::Duration> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (num_str, unit) = s.split_at(s.len() - 1);
    let num: i64 = num_str.parse().ok()?;
    match unit {
        "s" => Some(chrono::Duration::seconds(num)),
        "m" => Some(chrono::Duration::minutes(num)),
        "h" => Some(chrono::Duration::hours(num)),
        "d" => Some(chrono::Duration::days(num)),
        _ => {
            // Try parsing the whole string as minutes
            let num: i64 = s.parse().ok()?;
            Some(chrono::Duration::minutes(num))
        }
    }
}

fn cron_message_id(job_id: &str, occurrence_micros: i64) -> String {
    format!("cron-{job_id}-{occurrence_micros}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_postgres::NoTls;

    #[test]
    fn parse_interval_minutes() {
        let d = parse_interval("30m").unwrap();
        assert_eq!(d, chrono::Duration::minutes(30));
    }

    #[test]
    fn parse_interval_hours() {
        let d = parse_interval("2h").unwrap();
        assert_eq!(d, chrono::Duration::hours(2));
    }

    #[test]
    fn parse_interval_days() {
        let d = parse_interval("7d").unwrap();
        assert_eq!(d, chrono::Duration::days(7));
    }

    #[test]
    fn parse_interval_seconds() {
        let d = parse_interval("90s").unwrap();
        assert_eq!(d, chrono::Duration::seconds(90));
    }

    #[test]
    fn parse_interval_empty() {
        assert!(parse_interval("").is_none());
    }

    #[test]
    fn compute_next_every() {
        let next = compute_next_run("every", "1h", None);
        assert!(next.is_some());
        let delta = next.unwrap() - chrono::Utc::now();
        // Should be roughly 1 hour (within a few seconds)
        assert!(delta.num_minutes() >= 59 && delta.num_minutes() <= 61);
    }

    #[test]
    fn compute_next_at() {
        let next = compute_next_run("at", "2026-01-01T00:00:00Z", None);
        assert!(next.is_none());
    }

    #[test]
    fn compute_next_cron_daily() {
        let next = compute_next_run("cron", "0 10 * * *", None);
        assert!(next.is_some());
        let delta = next.unwrap() - chrono::Utc::now();
        // Should be ~24h for daily cron
        assert!(delta.num_hours() >= 23 && delta.num_hours() <= 25);
    }

    #[test]
    fn cron_message_id_is_unique_per_dispatch_command() {
        assert_ne!(cron_message_id("job-1", 100), cron_message_id("job-1", 200));
    }

    #[tokio::test]
    async fn concurrent_schedulers_claim_due_job_once() {
        let Ok(db_url) = std::env::var("CHORUZ_DATABASE_URL") else {
            return;
        };

        let job_id = choruz_common::new_id();
        let agent_id = choruz_common::new_id();
        let conversation_id = choruz_common::new_id();
        let (client, connection) = tokio_postgres::connect(&db_url, NoTls)
            .await
            .expect("connect for cron claim test");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        client
            .execute(
                "INSERT INTO agent_cron_job
                    (id, agent_id, conversation_id, name, schedule_type,
                     schedule_value, message, next_run_at)
                 VALUES ($1, $2, $3, 'claim-once', 'every', '1h', 'run once', NOW())",
                &[&job_id, &agent_id, &conversation_id],
            )
            .await
            .expect("seed due cron job");

        let event_store_a = EventStore::new(&db_url);
        let event_store_b = EventStore::new(&db_url);
        let session_store_a = PgSessionStore::new(&db_url);
        let session_store_b = PgSessionStore::new(&db_url);
        let (a, b) = tokio::join!(
            check_and_dispatch_due_jobs(&event_store_a, &session_store_a),
            check_and_dispatch_due_jobs(&event_store_b, &session_store_b),
        );
        a.expect("first scheduler");
        b.expect("second scheduler");

        let row = client
            .query_one(
                "SELECT COUNT(*)::BIGINT
                 FROM agent_commands
                 WHERE metadata->>'cron_job_id' = $1",
                &[&job_id],
            )
            .await
            .expect("count commands");
        assert_eq!(row.get::<_, i64>(0), 1);
    }

    #[tokio::test]
    async fn due_cron_job_inserts_message_and_agent_command() {
        let Ok(db_url) = std::env::var("CHORUZ_DATABASE_URL") else {
            return;
        };

        let workspace_id = choruz_common::new_id();
        let agent_id = choruz_common::new_id();
        let human_id = choruz_common::new_id();
        let human_name = format!("Cron Viewer {human_id}");
        let conversation_id = choruz_common::new_id();
        let job_id = choruz_common::new_id();
        let cron_message = format!("cron message {}", choruz_common::new_id());
        let (client, connection) = tokio_postgres::connect(&db_url, NoTls)
            .await
            .expect("connect for due cron integration test");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        client
            .execute(
                "INSERT INTO principal (id, workspace_id, type, name, disabled, created_at, updated_at)
                 VALUES ($1, $2, 'agent', 'Cron Target', FALSE, NOW(), NOW()),
                        ($3, $2, 'human', $4, FALSE, NOW(), NOW())",
                &[&agent_id, &workspace_id, &human_id, &human_name],
            )
            .await
            .expect("seed cron principals");
        client
            .execute(
                "INSERT INTO conversation (id, workspace_id, type, name, creator_id, created_at, updated_at)
                 VALUES ($1, $2, 'group', 'Cron Group', $3, NOW(), NOW())",
                &[&conversation_id, &workspace_id, &human_id],
            )
            .await
            .expect("seed cron conversation");
        client
            .execute(
                "INSERT INTO conversation_member (conv_id, principal_id, joined_at)
                 VALUES ($1, $2, NOW()),
                        ($1, $3, NOW())",
                &[&conversation_id, &agent_id, &human_id],
            )
            .await
            .expect("seed cron memberships");
        client
            .execute(
                "INSERT INTO agent_cron_job
                    (id, agent_id, conversation_id, name, schedule_type,
                     schedule_value, message, delivery_mode, next_run_at)
                 VALUES ($1, $2, $3, 'due-cron', 'every', '1h', $4, 'announce', NOW())",
                &[&job_id, &agent_id, &conversation_id, &cron_message],
            )
            .await
            .expect("seed due cron job");

        let event_store = EventStore::new(&db_url);
        let session_store = PgSessionStore::new(&db_url);
        check_and_dispatch_due_jobs(&event_store, &session_store)
            .await
            .expect("dispatch due cron job");

        let event = client
            .query_one(
                "SELECT event_id, sender_id, content, content_type, metadata->>'source' AS source,
                        metadata->>'cron_job_id' AS cron_job_id
                 FROM conversation_events
                 WHERE conversation_id = $1 AND content = $2",
                &[&conversation_id, &Some(cron_message.as_str())],
            )
            .await
            .expect("cron message event exists");
        let message_id: String = event.get("event_id");
        assert_eq!(event.get::<_, String>("sender_id"), "choruz-cron");
        assert_eq!(
            event.get::<_, Option<String>>("content").as_deref(),
            Some(cron_message.as_str())
        );
        assert_eq!(event.get::<_, String>("content_type"), "text/plain");
        assert_eq!(
            event.get::<_, Option<String>>("source").as_deref(),
            Some("cron_scheduler")
        );
        assert_eq!(
            event.get::<_, Option<String>>("cron_job_id").as_deref(),
            Some(job_id.as_str())
        );

        let command = client
            .query_one(
                "SELECT message_id, agent_id, conversation_id, status, prompt
                 FROM agent_commands
                 WHERE metadata->>'cron_job_id' = $1",
                &[&job_id],
            )
            .await
            .expect("cron command exists");
        assert_eq!(command.get::<_, String>("message_id"), message_id);
        assert_eq!(command.get::<_, String>("agent_id"), agent_id);
        assert_eq!(command.get::<_, String>("conversation_id"), conversation_id);
        assert_eq!(command.get::<_, String>("status"), "pending");
        assert!(
            command
                .get::<_, String>("prompt")
                .contains(cron_message.as_str())
        );

        let count = client
            .query_one(
                "SELECT total_msg_count FROM conversation WHERE id = $1",
                &[&conversation_id],
            )
            .await
            .expect("conversation count exists");
        assert_eq!(count.get::<_, i64>("total_msg_count"), 1);
    }

    #[tokio::test]
    async fn cron_dispatch_rolls_back_message_when_command_insert_fails() {
        let Ok(db_url) = std::env::var("CHORUZ_DATABASE_URL") else {
            return;
        };

        let workspace_id = choruz_common::new_id();
        let agent_id = choruz_common::new_id();
        let human_id = choruz_common::new_id();
        let human_name = format!("Cron Viewer {human_id}");
        let conversation_id = choruz_common::new_id();
        let session_key = format!("{agent_id}:{conversation_id}");
        let message_id = cron_message_id(
            &choruz_common::new_id(),
            chrono::Utc::now().timestamp_micros(),
        );
        let route_id = choruz_common::new_id();
        let cron_message = format!("rollback cron message {}", choruz_common::new_id());
        let (client, connection) = tokio_postgres::connect(&db_url, NoTls)
            .await
            .expect("connect for cron rollback test");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        client
            .execute(
                "INSERT INTO principal (id, workspace_id, type, name, disabled, created_at, updated_at)
                 VALUES ($1, $2, 'agent', 'Cron Target', FALSE, NOW(), NOW()),
                        ($3, $2, 'human', $4, FALSE, NOW(), NOW())",
                &[&agent_id, &workspace_id, &human_id, &human_name],
            )
            .await
            .expect("seed cron principals");
        client
            .execute(
                "INSERT INTO conversation (id, workspace_id, type, name, creator_id, created_at, updated_at)
                 VALUES ($1, $2, 'group', 'Cron Group', $3, NOW(), NOW())",
                &[&conversation_id, &workspace_id, &human_id],
            )
            .await
            .expect("seed cron conversation");
        client
            .execute(
                "INSERT INTO session_registry (session_key, agent_id, conversation_id, created_at, updated_at)
                 VALUES ($1, $2, $3, NOW(), NOW())",
                &[&format!("existing-{session_key}"), &agent_id, &conversation_id],
            )
            .await
            .expect("seed existing session");
        client
            .execute(
                "INSERT INTO agent_commands (
                    command_id, route_id, session_key, agent_id, conversation_id,
                    message_id, turn_id, prompt, metadata, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, 'existing', '{}', NOW(), NOW())",
                &[
                    &choruz_common::new_id(),
                    &route_id,
                    &format!("existing-{session_key}"),
                    &agent_id,
                    &conversation_id,
                    &format!("other-message-{}", choruz_common::new_id()),
                    &choruz_common::new_id(),
                ],
            )
            .await
            .expect("seed route_id collision");

        let event_store = EventStore::new(&db_url);
        let rollback_job_id = choruz_common::new_id();
        let rollback_command_id = choruz_common::new_id();
        let rollback_turn_id = choruz_common::new_id();
        let err = dispatch_cron_occurrence(
            &event_store,
            CronDispatch {
                agent_id: &agent_id,
                conversation_id: &conversation_id,
                session_key: &session_key,
                message_id: &message_id,
                job_id: &rollback_job_id,
                job_name: "rollback-test",
                message: &cron_message,
                announce: true,
                command_id: &rollback_command_id,
                route_id: &route_id,
                turn_id: &rollback_turn_id,
                prompt: "prompt",
                metadata: serde_json::json!({"cron_job_id": "rollback-test"}),
            },
        )
        .await
        .expect_err("route_id collision should fail command insert");
        assert!(err.contains("insert command"));

        let event_count = client
            .query_one(
                "SELECT COUNT(*)::BIGINT
                 FROM conversation_events
                 WHERE conversation_id = $1 AND event_id = $2",
                &[&conversation_id, &message_id],
            )
            .await
            .expect("count rolled back cron messages");
        assert_eq!(event_count.get::<_, i64>(0), 0);

        let total = client
            .query_one(
                "SELECT total_msg_count FROM conversation WHERE id = $1",
                &[&conversation_id],
            )
            .await
            .expect("conversation count exists");
        assert_eq!(total.get::<_, i64>("total_msg_count"), 0);
    }

    #[tokio::test]
    async fn cron_dispatch_creates_session_and_recovers_stale_claim_once() {
        let Ok(db_url) = std::env::var("CHORUZ_DATABASE_URL") else {
            return;
        };

        let job_id = choruz_common::new_id();
        let agent_id = choruz_common::new_id();
        let conversation_id = choruz_common::new_id();
        let due_at = chrono::Utc::now() - chrono::Duration::seconds(1);
        let stale_running_at = chrono::Utc::now() - chrono::Duration::minutes(30);
        let (client, connection) = tokio_postgres::connect(&db_url, NoTls)
            .await
            .expect("connect for stale cron claim test");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        client
            .execute(
                "INSERT INTO agent_cron_job
                    (id, agent_id, conversation_id, name, schedule_type,
                     schedule_value, message, next_run_at, running_at)
                 VALUES ($1, $2, $3, 'stale-claim', 'every', '1h',
                         'recover once', $4, $5)",
                &[
                    &job_id,
                    &agent_id,
                    &conversation_id,
                    &due_at,
                    &stale_running_at,
                ],
            )
            .await
            .expect("seed stale claimed cron job");

        let event_store = EventStore::new(&db_url);
        let session_store = PgSessionStore::new(&db_url);
        check_and_dispatch_due_jobs(&event_store, &session_store)
            .await
            .expect("recover stale cron claim");
        // Simulate the final job-state update failing after command insert by
        // putting the same stale occurrence back into claimable state.
        client
            .execute(
                "UPDATE agent_cron_job
                 SET next_run_at = $2, running_at = $3
                 WHERE id = $1",
                &[&job_id, &due_at, &stale_running_at],
            )
            .await
            .expect("reset stale claim");
        check_and_dispatch_due_jobs(&event_store, &session_store)
            .await
            .expect("recover same stale cron claim again");

        let row = client
            .query_one(
                "SELECT COUNT(*)::BIGINT
                 FROM agent_commands
                 WHERE metadata->>'cron_job_id' = $1",
                &[&job_id],
            )
            .await
            .expect("count commands");
        assert_eq!(row.get::<_, i64>(0), 1);

        let session_key = format!("{agent_id}:{conversation_id}");
        let row = client
            .query_one(
                "SELECT agent_id, conversation_id
                 FROM session_registry
                 WHERE session_key = $1",
                &[&session_key],
            )
            .await
            .expect("cron session exists");
        assert_eq!(row.get::<_, String>("agent_id"), agent_id);
        assert_eq!(row.get::<_, String>("conversation_id"), conversation_id);
    }
}
