//! Integration tests for choruz-session.
//!
//! These tests run against a real PostgreSQL database. The connection string
//! is read from `CHORUZ_TEST_DATABASE_URL` or falls back to the default local
//! development database.
//!
//! Tables must be created via `V001__message_pipeline_schema.sql` before
//! running these tests.

use choruz_session::*;
use chrono::Utc;
use uuid::Uuid;

/// `list_dead_letters` returns oldest-first and is LIMITed, so on a shared
/// dev DB (which accumulates unresolved entries) freshly inserted rows get
/// pushed past the window. Look them up by source_id through a "since"
/// window anchored at test start time to keep assertions deterministic.
async fn find_dead_letter_by_source_id(
    store: &PgSessionStore,
    since: chrono::DateTime<Utc>,
    source_id: &str,
) -> Option<DeadLetter> {
    store
        .list_dead_letters_since(since, 10_000)
        .await
        .unwrap()
        .into_iter()
        .find(|dl| dl.source_id == source_id)
}

/// Get the test database URL.
fn test_database_url() -> String {
    std::env::var("CHORUZ_TEST_DATABASE_URL").unwrap_or_else(|_| {
        let user = std::env::var("USER").unwrap_or_else(|_| "postgres".to_string());
        format!("host=127.0.0.1 port=5432 user={user} dbname=choruz")
    })
}

/// Generate a unique key for test isolation.
fn unique_key(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::now_v7())
}

// =========================================================================
// D1: session_registry CRUD
// =========================================================================

#[tokio::test]
async fn test_upsert_and_get_session() {
    let store = PgSessionStore::new(&test_database_url());
    store.health_check().await.expect("DB not reachable");

    let sk = unique_key("sess");
    let agent_id = unique_key("agent");
    let conv_id = unique_key("conv");

    // First upsert creates the session
    let session = store
        .upsert_session(&sk, &agent_id, &conv_id)
        .await
        .unwrap();
    assert_eq!(session.session_key, sk);
    assert_eq!(session.agent_id, agent_id);
    assert_eq!(session.conversation_id, conv_id);
    assert_eq!(session.epoch, 0);
    assert_eq!(session.status, SessionStatus::Idle);

    // Second upsert returns the existing session (no-op)
    let session2 = store
        .upsert_session(&sk, &agent_id, &conv_id)
        .await
        .unwrap();
    assert_eq!(session2.session_key, sk);
    assert_eq!(session2.epoch, 0);

    // get_session returns Some
    let fetched = store.get_session(&sk).await.unwrap();
    assert!(fetched.is_some());
    assert_eq!(fetched.unwrap().session_key, sk);

    // get_session for non-existent returns None
    let not_found = store.get_session("non-existent-key").await.unwrap();
    assert!(not_found.is_none());
}

#[tokio::test]
async fn test_update_session() {
    let store = PgSessionStore::new(&test_database_url());
    store.health_check().await.expect("DB not reachable");

    let sk = unique_key("sess");
    let agent_id = unique_key("agent");
    let conv_id = unique_key("conv");

    store
        .upsert_session(&sk, &agent_id, &conv_id)
        .await
        .unwrap();

    // Generic session updates cannot modify the lease epoch.
    store
        .update_session(&SessionUpdate {
            session_key: sk.clone(),
            executor_node_id: Some(Some("node-1".to_string())),
            status: Some(SessionStatus::Active),
            last_heartbeat_at: Some(Some(chrono::Utc::now())),
        })
        .await
        .unwrap();

    let updated = store.get_session(&sk).await.unwrap().unwrap();
    assert_eq!(updated.epoch, 0);
    assert_eq!(updated.status, SessionStatus::Active);
    assert_eq!(updated.executor_node_id.as_deref(), Some("node-1"));
    assert!(updated.last_heartbeat_at.is_some());
}

#[tokio::test]
async fn test_update_session_heartbeat_for_current_epoch() {
    let store = PgSessionStore::new(&test_database_url());
    store.health_check().await.expect("DB not reachable");

    let sk = unique_key("sess");
    store.upsert_session(&sk, "a", "c").await.unwrap();
    let command = store
        .insert_command(&test_insert_command_input(&sk))
        .await
        .unwrap();
    store
        .assign_lease(&command.command_id, "executor-1")
        .await
        .unwrap();

    store
        .update_session_heartbeat_for_epoch(&sk, 1)
        .await
        .unwrap();

    let session = store.get_session(&sk).await.unwrap().unwrap();
    assert!(session.last_heartbeat_at.is_some());

    store
        .update_session(&SessionUpdate {
            session_key: sk.clone(),
            executor_node_id: None,
            status: Some(SessionStatus::Idle),
            last_heartbeat_at: None,
        })
        .await
        .unwrap();
    assert!(matches!(
        store.update_session_heartbeat_for_epoch(&sk, 1).await,
        Err(SessionError::SessionInactive { .. })
    ));

    // Non-existent session should error
    assert!(matches!(
        store.update_session_heartbeat_for_epoch("bogus", 1).await,
        Err(SessionError::SessionNotFound(_))
    ));
}

// =========================================================================
// D2: agent_commands state machine
// =========================================================================

fn test_insert_command_input(session_key: &str) -> InsertCommand {
    InsertCommand {
        command_id: Uuid::now_v7().to_string(),
        route_id: Uuid::now_v7().to_string(),
        session_key: session_key.to_string(),
        agent_id: "agent-1".to_string(),
        conversation_id: "conv-1".to_string(),
        message_id: Uuid::now_v7().to_string(),
        turn_id: Uuid::now_v7().to_string(),
        prompt: "test prompt".to_string(),
        max_attempts: 5,
        metadata: serde_json::json!({}),
    }
}

fn test_insert_command_input_for(
    session_key: &str,
    agent_id: &str,
    conv_id: &str,
) -> InsertCommand {
    InsertCommand {
        command_id: Uuid::now_v7().to_string(),
        route_id: Uuid::now_v7().to_string(),
        session_key: session_key.to_string(),
        agent_id: agent_id.to_string(),
        conversation_id: conv_id.to_string(),
        message_id: Uuid::now_v7().to_string(),
        turn_id: Uuid::now_v7().to_string(),
        prompt: "test prompt".to_string(),
        max_attempts: 5,
        metadata: serde_json::json!({}),
    }
}

#[tokio::test]
async fn test_insert_and_get_command() {
    let store = PgSessionStore::new(&test_database_url());
    store.health_check().await.expect("DB not reachable");

    let sk = unique_key("sess");
    store.upsert_session(&sk, "a", "c").await.unwrap();

    let input = test_insert_command_input(&sk);
    let cmd = store.insert_command(&input).await.unwrap();
    assert_eq!(cmd.command_id, input.command_id);
    assert_eq!(cmd.status, CommandStatus::Pending);
    assert_eq!(cmd.attempt_count, 0);
    assert_eq!(cmd.max_attempts, 5);

    // Get by ID
    let fetched = store.get_command(&cmd.command_id).await.unwrap();
    assert!(fetched.is_some());
    assert_eq!(fetched.unwrap().command_id, cmd.command_id);
}

#[tokio::test]
async fn insert_command_is_idempotent_by_message_and_agent_under_race() {
    let store = PgSessionStore::new(&test_database_url());
    store.health_check().await.expect("DB not reachable");

    let sk = unique_key("sess");
    let agent_id = unique_key("agent");
    let conv_id = unique_key("conv");
    store
        .upsert_session(&sk, &agent_id, &conv_id)
        .await
        .unwrap();

    let message_id = Uuid::now_v7().to_string();
    let mut first = test_insert_command_input_for(&sk, &agent_id, &conv_id);
    first.message_id = message_id.clone();
    first.prompt = "first prompt wins".to_string();
    let mut duplicate = test_insert_command_input_for(&sk, &agent_id, &conv_id);
    duplicate.message_id = message_id.clone();
    duplicate.prompt = "duplicate prompt loses".to_string();

    let store2 = store.clone();
    let (a, b) = tokio::join!(
        store.insert_command(&first),
        store2.insert_command(&duplicate)
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert_eq!(a.command_id, b.command_id);
    assert_eq!(a.route_id, b.route_id);
    assert_eq!(a.turn_id, b.turn_id);

    let client = store.connect().await.unwrap();
    let row = client
        .query_one(
            "SELECT COUNT(*) FROM agent_commands WHERE message_id = $1 AND agent_id = $2",
            &[&message_id, &agent_id],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, i64>(0), 1);
}

#[tokio::test]
async fn test_find_active_command_for_session() {
    let store = PgSessionStore::new(&test_database_url());
    store.health_check().await.expect("DB not reachable");

    let sk = unique_key("sess");
    store.upsert_session(&sk, "a", "c").await.unwrap();

    // No active command initially
    let active = store.find_active_command_for_session(&sk).await.unwrap();
    assert!(active.is_none());

    // Insert a command (pending)
    let input = test_insert_command_input(&sk);
    let cmd = store.insert_command(&input).await.unwrap();

    // Still no active command (pending is not active)
    let active = store.find_active_command_for_session(&sk).await.unwrap();
    assert!(active.is_none());

    // Transition to leased, as the dispatcher does after lease assignment.
    store
        .update_command_status(&CommandStatusUpdate {
            command_id: cmd.command_id.clone(),
            status: CommandStatus::Leased,
            current_attempt_id: Some(Uuid::now_v7().to_string()),
            current_epoch: Some(1),
            attempt_count: Some(1),
            ..Default::default()
        })
        .await
        .unwrap();

    // Now there is an active command
    let active = store.find_active_command_for_session(&sk).await.unwrap();
    assert!(active.is_some());
    assert_eq!(active.unwrap().command_id, cmd.command_id);
}

#[tokio::test]
async fn test_runtime_status_for_agents() {
    let store = PgSessionStore::new(&test_database_url());
    store.health_check().await.expect("DB not reachable");

    let conv_id = unique_key("runtime-conv");
    let busy_agent = unique_key("runtime-agent-busy");
    let queued_agent = unique_key("runtime-agent-queued");
    let idle_agent = unique_key("runtime-agent-idle");
    let busy_sk = unique_key("runtime-sess-busy");
    let queued_sk = unique_key("runtime-sess-queued");
    let idle_sk = unique_key("runtime-sess-idle");

    store
        .upsert_session(&busy_sk, &busy_agent, &conv_id)
        .await
        .unwrap();
    store
        .upsert_session(&queued_sk, &queued_agent, &conv_id)
        .await
        .unwrap();
    store
        .upsert_session(&idle_sk, &idle_agent, &conv_id)
        .await
        .unwrap();

    let status_now = Utc::now();
    let active_error = "active runtime failed once";
    let queued_error = "retry waiting for runtime";

    let active_cmd = store
        .insert_command(&test_insert_command_input_for(
            &busy_sk,
            &busy_agent,
            &conv_id,
        ))
        .await
        .unwrap();
    store
        .assign_lease(&active_cmd.command_id, "runtime-node-1")
        .await
        .unwrap();
    store
        .update_command_status(&CommandStatusUpdate {
            command_id: active_cmd.command_id.clone(),
            status: CommandStatus::Leased,
            attempt_count: Some(3),
            last_error: Some(active_error.to_string()),
            ..Default::default()
        })
        .await
        .unwrap();

    let newer_active_cmd = store
        .insert_command(&test_insert_command_input_for(
            &busy_sk,
            &busy_agent,
            &conv_id,
        ))
        .await
        .unwrap();
    store
        .update_command_status(&CommandStatusUpdate {
            command_id: newer_active_cmd.command_id.clone(),
            status: CommandStatus::Started,
            attempt_count: Some(1),
            ..Default::default()
        })
        .await
        .unwrap();

    let active_created_at = status_now - chrono::TimeDelta::seconds(120);
    let active_updated_at = status_now - chrono::TimeDelta::seconds(42);
    let newer_created_at = status_now - chrono::TimeDelta::seconds(60);
    let client = store.connect().await.unwrap();
    client
        .execute(
            "UPDATE agent_commands
             SET created_at = $2, updated_at = $3
             WHERE command_id = $1",
            &[
                &active_cmd.command_id,
                &active_created_at,
                &active_updated_at,
            ],
        )
        .await
        .unwrap();
    client
        .execute(
            "UPDATE agent_commands
             SET created_at = $2, updated_at = $2
             WHERE command_id = $1",
            &[&newer_active_cmd.command_id, &newer_created_at],
        )
        .await
        .unwrap();

    let retry_cmd = store
        .insert_command(&test_insert_command_input_for(
            &queued_sk,
            &queued_agent,
            &conv_id,
        ))
        .await
        .unwrap();
    store
        .update_command_status(&CommandStatusUpdate {
            command_id: retry_cmd.command_id.clone(),
            status: CommandStatus::RetryScheduled,
            attempt_count: Some(2),
            next_retry_at: Some(Some(status_now + chrono::TimeDelta::seconds(60))),
            last_error: Some(queued_error.to_string()),
            ..Default::default()
        })
        .await
        .unwrap();
    store
        .insert_command(&test_insert_command_input_for(
            &queued_sk,
            &queued_agent,
            &conv_id,
        ))
        .await
        .unwrap();

    let active = store
        .find_active_command_for_agent(&conv_id, &busy_agent)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(active.command_id, active_cmd.command_id);

    assert_eq!(
        store
            .count_queued_commands_for_agent(&conv_id, &queued_agent)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        store
            .count_queued_commands_for_agent(&conv_id, &idle_agent)
            .await
            .unwrap(),
        0
    );

    let agent_ids = vec![busy_agent.clone(), queued_agent.clone(), idle_agent.clone()];
    let statuses = store
        .list_runtime_status_for_agents(&conv_id, &agent_ids, status_now)
        .await
        .unwrap();

    assert_eq!(statuses.len(), 3);
    assert_eq!(statuses[0].agent_id, busy_agent);
    assert_eq!(statuses[1].agent_id, queued_agent);
    assert_eq!(statuses[2].agent_id, idle_agent);

    let busy = &statuses[0];
    assert_eq!(busy.conversation_id, conv_id);
    assert_eq!(busy.status, "busy");
    assert_eq!(busy.queued_count, 0);
    assert_eq!(busy.last_error.as_deref(), Some(active_error));
    let busy_command = busy.active_command.as_ref().unwrap();
    assert_eq!(busy_command.command_id, active_cmd.command_id);
    assert_eq!(busy_command.message_id, active_cmd.message_id);
    assert_eq!(busy_command.turn_id, active_cmd.turn_id);
    assert_eq!(busy_command.status, "leased");
    assert_eq!(busy_command.attempt_count, 3);
    assert_eq!(busy_command.lease_age_seconds, 42);
    assert_eq!(busy_command.last_error.as_deref(), Some(active_error));

    let queued = &statuses[1];
    assert_eq!(queued.status, "queued");
    assert!(queued.active_command.is_none());
    assert_eq!(queued.queued_count, 2);
    assert_eq!(queued.last_error.as_deref(), Some(queued_error));

    let idle = &statuses[2];
    assert_eq!(idle.status, "idle");
    assert!(idle.active_command.is_none());
    assert_eq!(idle.queued_count, 0);
    assert!(idle.last_error.is_none());
}

#[tokio::test]
async fn test_find_pending_commands() {
    let store = PgSessionStore::new(&test_database_url());
    store.health_check().await.expect("DB not reachable");

    let sk = unique_key("sess");
    let agent_id = unique_key("agent");
    let conv_id = unique_key("conv");
    store
        .upsert_session(&sk, &agent_id, &conv_id)
        .await
        .unwrap();

    // Insert two pending commands
    let cmd1 = store
        .insert_command(&test_insert_command_input_for(&sk, &agent_id, &conv_id))
        .await
        .unwrap();
    let cmd2 = store
        .insert_command(&test_insert_command_input_for(&sk, &agent_id, &conv_id))
        .await
        .unwrap();

    let pending = store.find_pending_commands(1000).await.unwrap();
    assert!(pending.iter().any(|c| c.command_id == cmd1.command_id));
    assert!(pending.iter().any(|c| c.command_id == cmd2.command_id));
}

#[tokio::test]
async fn test_find_pending_commands_gives_idle_agents_a_fair_first_slot() {
    let store = PgSessionStore::new(&test_database_url());
    store.health_check().await.expect("DB not reachable");

    let hot_agent = unique_key("fair-hot-agent");
    let idle_agent = unique_key("fair-idle-agent");
    let conversation = unique_key("fair-conv");
    let hot_session = format!("{hot_agent}:{conversation}");
    let idle_session = format!("{idle_agent}:{conversation}");
    store
        .upsert_session(&hot_session, &hot_agent, &conversation)
        .await
        .unwrap();
    store
        .upsert_session(&idle_session, &idle_agent, &conversation)
        .await
        .unwrap();

    let hot_first = store
        .insert_command(&test_insert_command_input_for(
            &hot_session,
            &hot_agent,
            &conversation,
        ))
        .await
        .unwrap();
    let hot_second = store
        .insert_command(&test_insert_command_input_for(
            &hot_session,
            &hot_agent,
            &conversation,
        ))
        .await
        .unwrap();
    let idle_first = store
        .insert_command(&test_insert_command_input_for(
            &idle_session,
            &idle_agent,
            &conversation,
        ))
        .await
        .unwrap();

    let pending = store.find_pending_commands(1000).await.unwrap();
    let position = |command_id: &str| {
        pending
            .iter()
            .position(|command| command.command_id == command_id)
            .expect("test command should be dispatchable")
    };

    assert!(position(&hot_first.command_id) < position(&hot_second.command_id));
    assert!(
        position(&idle_first.command_id) < position(&hot_second.command_id),
        "every idle agent's first command must be offered before a hot agent's second"
    );
}

/// Coalescer: `find_pending_commands` must skip pending commands when the
/// same agent_id already has a command in any non-terminal active or
/// retry-queued state — regardless of which conversation. Bounds OS process
/// count at 1 per agent so 13-agent group chats don't accumulate hundreds
/// of stuck `claude --print` children, AND ensures the agent's single
/// `external_session_id` (per-workspace, migration 0018) isn't raced by two
/// concurrent spawns that would clobber session history.
#[tokio::test]
async fn test_find_pending_commands_coalesces_per_agent() {
    let store = PgSessionStore::new(&test_database_url());
    store.health_check().await.expect("DB not reachable");

    // Per-test pair so we don't collide with other concurrent tests reusing
    // the shared dev DB. unique_key() embeds a v7 UUID -> globally distinct.
    let agent_id = unique_key("coal-agent");
    let conv_id = unique_key("coal-conv");

    fn input(agent: &str, conv: &str, sk: &str) -> InsertCommand {
        InsertCommand {
            command_id: Uuid::now_v7().to_string(),
            route_id: Uuid::now_v7().to_string(),
            session_key: sk.to_string(),
            agent_id: agent.to_string(),
            conversation_id: conv.to_string(),
            message_id: Uuid::now_v7().to_string(),
            turn_id: Uuid::now_v7().to_string(),
            prompt: "p".to_string(),
            max_attempts: 5,
            metadata: serde_json::json!({}),
        }
    }

    let sk = format!("{}:{}", agent_id, conv_id);
    store
        .upsert_session(&sk, &agent_id, &conv_id)
        .await
        .unwrap();

    // Snapshot pending IDs we know about, so other tests' rows don't pollute
    // assertions.
    let mine = |all: Vec<AgentCommand>, ids: &[&str]| -> Vec<String> {
        all.into_iter()
            .filter(|c| ids.iter().any(|wanted| c.command_id == *wanted))
            .map(|c| c.command_id)
            .collect()
    };

    // Insert cmd1 (pending). It should be visible.
    let cmd1 = store
        .insert_command(&input(&agent_id, &conv_id, &sk))
        .await
        .unwrap();
    let pending = store.find_pending_commands(1000).await.unwrap();
    assert!(
        pending.iter().any(|c| c.command_id == cmd1.command_id),
        "cmd1 should be returned when nothing else exists for this pair"
    );

    // Move cmd1 to leased (active). Insert cmd2 (pending) for SAME pair.
    // cmd2 must NOT be returned — coalescer blocks while cmd1 holds the slot.
    store
        .update_command_status(&CommandStatusUpdate {
            command_id: cmd1.command_id.clone(),
            status: CommandStatus::Leased,
            current_attempt_id: Some(Uuid::now_v7().to_string()),
            current_epoch: Some(1),
            attempt_count: Some(1),
            ..Default::default()
        })
        .await
        .unwrap();
    let cmd2 = store
        .insert_command(&input(&agent_id, &conv_id, &sk))
        .await
        .unwrap();
    let pending = store.find_pending_commands(1000).await.unwrap();
    let visible = mine(pending, &[&cmd2.command_id]);
    assert!(
        visible.is_empty(),
        "cmd2 must be hidden while cmd1 (leased) holds the agent's slot, got {visible:?}"
    );

    // Same agent, DIFFERENT conversation → also blocked. Per-agent coalescing
    // matches the per-agent runtime binding scope: agent A has ONE
    // external_session_id shared across all groups, so a parallel spawn in a
    // different group would race the same session.
    let other_conv = unique_key("coal-conv-other");
    let other_sk = format!("{}:{}", agent_id, other_conv);
    store
        .upsert_session(&other_sk, &agent_id, &other_conv)
        .await
        .unwrap();
    let cmd_cross_conv = store
        .insert_command(&input(&agent_id, &other_conv, &other_sk))
        .await
        .unwrap();
    let pending = store.find_pending_commands(1000).await.unwrap();
    let visible = mine(pending, &[&cmd_cross_conv.command_id]);
    assert!(
        visible.is_empty(),
        "cmd in a different conversation for the SAME agent must also be coalesced — agent's session is workspace-scoped, not per-conv, got {visible:?}"
    );

    // DIFFERENT agent → NOT blocked. The coalescer guards a single agent's
    // session; other agents' work proceeds independently.
    let other_agent = unique_key("coal-agent-other");
    let other_agent_sk = format!("{}:{}", other_agent, conv_id);
    store
        .upsert_session(&other_agent_sk, &other_agent, &conv_id)
        .await
        .unwrap();
    let cmd_other_agent = store
        .insert_command(&input(&other_agent, &conv_id, &other_agent_sk))
        .await
        .unwrap();
    let pending = store.find_pending_commands(1000).await.unwrap();
    assert!(
        pending
            .iter()
            .any(|c| c.command_id == cmd_other_agent.command_id),
        "cmd for a different agent must NOT be coalesced; the slot is per-agent"
    );

    // Move cmd1 to retry_scheduled (queued for retry, not actively running).
    // cmd2 still must not be returned — retry_scheduled is included in the
    // blocking set so messages stay in send-order on a flaky pair.
    let future = chrono::Utc::now() + chrono::TimeDelta::seconds(60);
    store
        .update_command_status(&CommandStatusUpdate {
            command_id: cmd1.command_id.clone(),
            status: CommandStatus::RetryScheduled,
            next_retry_at: Some(Some(future)),
            ..Default::default()
        })
        .await
        .unwrap();
    let pending = store.find_pending_commands(1000).await.unwrap();
    let visible = mine(pending, &[&cmd2.command_id, &cmd_cross_conv.command_id]);
    assert!(
        visible.is_empty(),
        "cmd2 AND cmd_cross_conv must stay hidden while cmd1 sits in retry_scheduled — preserves send-order across all of agent's convs"
    );

    // Move cmd1 to a terminal status (committed). cmd2 must now appear:
    // terminal states are not blocking.
    store
        .update_command_status(&CommandStatusUpdate {
            command_id: cmd1.command_id.clone(),
            status: CommandStatus::Committed,
            ..Default::default()
        })
        .await
        .unwrap();
    let pending = store.find_pending_commands(1000).await.unwrap();
    assert!(
        pending.iter().any(|c| c.command_id == cmd2.command_id),
        "cmd2 must become visible after cmd1 reaches a terminal status"
    );
}

#[tokio::test]
async fn test_find_retriable_commands() {
    let store = PgSessionStore::new(&test_database_url());
    store.health_check().await.expect("DB not reachable");

    let sk = unique_key("sess");
    store.upsert_session(&sk, "a", "c").await.unwrap();

    let input = test_insert_command_input(&sk);
    let cmd = store.insert_command(&input).await.unwrap();

    // Schedule retry in the past
    let past = chrono::Utc::now() - chrono::TimeDelta::seconds(10);
    store
        .update_command_status(&CommandStatusUpdate {
            command_id: cmd.command_id.clone(),
            status: CommandStatus::RetryScheduled,
            next_retry_at: Some(Some(past)),
            ..Default::default()
        })
        .await
        .unwrap();

    let retriable = store
        .find_retriable_commands(chrono::Utc::now(), 1000)
        .await
        .unwrap();
    assert!(retriable.iter().any(|c| c.command_id == cmd.command_id));
}

// =========================================================================
// D3: Lease assignment + epoch bump
// =========================================================================

#[tokio::test]
async fn test_assign_and_validate_lease() {
    let store = PgSessionStore::new(&test_database_url());
    store.health_check().await.expect("DB not reachable");

    let sk = unique_key("sess");
    store.upsert_session(&sk, "a", "c").await.unwrap();

    let input = test_insert_command_input(&sk);
    let cmd = store.insert_command(&input).await.unwrap();

    // Assign lease
    let lease = store
        .assign_lease(&cmd.command_id, "executor-1")
        .await
        .unwrap();
    assert_eq!(lease.epoch, 1); // epoch bumped from 0 to 1
    assert!(!lease.attempt_id.is_empty());
    assert_eq!(lease.attempt_count, 1);

    // Validate epoch
    let valid = store.validate_epoch(&sk, lease.epoch).await.unwrap();
    assert!(valid);

    let invalid = store.validate_epoch(&sk, 999).await.unwrap();
    assert!(!invalid);

    // Check command was updated
    let updated_cmd = store.get_command(&cmd.command_id).await.unwrap().unwrap();
    assert_eq!(updated_cmd.status, CommandStatus::Leased);
    assert_eq!(updated_cmd.current_epoch, Some(1));
    assert_eq!(updated_cmd.attempt_count, 1);
    assert!(updated_cmd.current_attempt_id.is_some());

    // Check session was updated
    let updated_sess = store.get_session(&sk).await.unwrap().unwrap();
    assert_eq!(updated_sess.epoch, 1);
    assert_eq!(updated_sess.status, SessionStatus::Active);
    assert_eq!(updated_sess.executor_node_id.as_deref(), Some("executor-1"));
}

#[tokio::test]
async fn assign_lease_rejects_duplicate_lease_attempt() {
    let store = PgSessionStore::new(&test_database_url());
    store.health_check().await.expect("DB not reachable");

    let sk = unique_key("sess");
    store.upsert_session(&sk, "a", "c").await.unwrap();

    let input = test_insert_command_input(&sk);
    let cmd = store.insert_command(&input).await.unwrap();

    store
        .assign_lease(&cmd.command_id, "executor-1")
        .await
        .unwrap();
    let err = store
        .assign_lease(&cmd.command_id, "executor-2")
        .await
        .unwrap_err();
    assert!(
        matches!(err, SessionError::InvalidStateTransition { .. }),
        "expected duplicate lease to be rejected, got {err:?}"
    );

    let updated = store.get_command(&cmd.command_id).await.unwrap().unwrap();
    assert_eq!(updated.status, CommandStatus::Leased);
    assert_eq!(updated.attempt_count, 1);
}

#[tokio::test]
async fn assign_batch_leases_rolls_back_when_any_member_is_not_pending() {
    let store = PgSessionStore::new(&test_database_url());
    store.health_check().await.expect("DB not reachable");

    let sk = unique_key("sess");
    let agent_id = unique_key("agent");
    let conv_id = unique_key("conv");
    store
        .upsert_session(&sk, &agent_id, &conv_id)
        .await
        .unwrap();

    let cmd1 = store
        .insert_command(&test_insert_command_input_for(&sk, &agent_id, &conv_id))
        .await
        .unwrap();
    let cmd2 = store
        .insert_command(&test_insert_command_input_for(&sk, &agent_id, &conv_id))
        .await
        .unwrap();
    store
        .assign_lease(&cmd2.command_id, "other-node")
        .await
        .unwrap();

    let err = store
        .assign_batch_leases(
            &[cmd1.command_id.clone(), cmd2.command_id.clone()],
            "batch-node",
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, SessionError::InvalidStateTransition { .. }),
        "expected non-pending batch member to reject the whole batch, got {err:?}"
    );

    let cmd1 = store.get_command(&cmd1.command_id).await.unwrap().unwrap();
    let cmd2 = store.get_command(&cmd2.command_id).await.unwrap().unwrap();
    assert_eq!(cmd1.status, CommandStatus::Pending);
    assert_eq!(cmd1.attempt_count, 0);
    assert_eq!(cmd2.status, CommandStatus::Leased);
    assert_eq!(cmd2.attempt_count, 1);
}

#[tokio::test]
async fn assign_batch_leases_same_session_uses_one_epoch() {
    let store = PgSessionStore::new(&test_database_url());
    store.health_check().await.expect("DB not reachable");

    let sk = unique_key("sess");
    let agent_id = unique_key("agent");
    let conv_id = unique_key("conv");
    store
        .upsert_session(&sk, &agent_id, &conv_id)
        .await
        .unwrap();

    let cmd1 = store
        .insert_command(&test_insert_command_input_for(&sk, &agent_id, &conv_id))
        .await
        .unwrap();
    let cmd2 = store
        .insert_command(&test_insert_command_input_for(&sk, &agent_id, &conv_id))
        .await
        .unwrap();

    let leases = store
        .assign_batch_leases(
            &[cmd1.command_id.clone(), cmd2.command_id.clone()],
            "batch-node",
        )
        .await
        .unwrap();
    assert_eq!(leases.len(), 2);
    assert_eq!(leases.get(&cmd1.command_id).unwrap().epoch, 1);
    assert_eq!(leases.get(&cmd2.command_id).unwrap().epoch, 1);

    let cmd1 = store.get_command(&cmd1.command_id).await.unwrap().unwrap();
    let cmd2 = store.get_command(&cmd2.command_id).await.unwrap().unwrap();
    assert_eq!(cmd1.status, CommandStatus::Leased);
    assert_eq!(cmd2.status, CommandStatus::Leased);
    assert_eq!(cmd1.current_epoch, Some(1));
    assert_eq!(cmd2.current_epoch, Some(1));
    assert_eq!(cmd1.attempt_count, 1);
    assert_eq!(cmd2.attempt_count, 1);

    let session = store.get_session(&sk).await.unwrap().unwrap();
    assert_eq!(session.epoch, 1);
}

#[tokio::test]
async fn test_release_lease() {
    let store = PgSessionStore::new(&test_database_url());
    store.health_check().await.expect("DB not reachable");

    let sk = unique_key("sess");
    store.upsert_session(&sk, "a", "c").await.unwrap();

    let input = test_insert_command_input(&sk);
    let cmd = store.insert_command(&input).await.unwrap();

    // Assign then release
    store
        .assign_lease(&cmd.command_id, "executor-1")
        .await
        .unwrap();
    store.release_lease(&cmd.command_id).await.unwrap();

    // Command should be succeeded
    let updated = store.get_command(&cmd.command_id).await.unwrap().unwrap();
    assert_eq!(updated.status, CommandStatus::Succeeded);

    // Session should be idle
    let session = store.get_session(&sk).await.unwrap().unwrap();
    assert_eq!(session.status, SessionStatus::Idle);
    assert!(session.executor_node_id.is_none());
}

// =========================================================================
// D4: Heartbeat monitoring + lease expiry
// =========================================================================

#[tokio::test]
async fn test_check_expired_leases() {
    let store = PgSessionStore::new(&test_database_url());
    store.health_check().await.expect("DB not reachable");

    let sk = unique_key("sess");
    store.upsert_session(&sk, "a", "c").await.unwrap();

    let input = test_insert_command_input(&sk);
    let cmd = store.insert_command(&input).await.unwrap();

    // Assign lease (this sets heartbeat to now)
    store
        .assign_lease(&cmd.command_id, "executor-1")
        .await
        .unwrap();

    // No expired leases right now (heartbeat was just set)
    let expired = store
        .check_expired_leases(chrono::Utc::now(), 60)
        .await
        .unwrap();
    assert!(!expired.iter().any(|e| e.command_id == cmd.command_id));

    // Simulate time passing: check with timeout_secs = 0 (everything is expired)
    let expired = store
        .check_expired_leases(chrono::Utc::now() + chrono::TimeDelta::seconds(120), 60)
        .await
        .unwrap();
    assert!(expired.iter().any(|e| e.command_id == cmd.command_id));
}

#[tokio::test]
async fn test_handle_lease_expiry_retry() {
    let store = PgSessionStore::new(&test_database_url());
    store.health_check().await.expect("DB not reachable");

    let sk = unique_key("sess");
    store.upsert_session(&sk, "a", "c").await.unwrap();

    let input = test_insert_command_input(&sk);
    let cmd = store.insert_command(&input).await.unwrap();
    let lease = store
        .assign_lease(&cmd.command_id, "executor-1")
        .await
        .unwrap();

    // Handle expiry (attempt_count=1, max=5 => should retry)
    let expired = ExpiredLease {
        session_key: sk.clone(),
        command_id: cmd.command_id.clone(),
        epoch: 1,
        attempt_count: 1,
        max_attempts: 5,
    };
    store.handle_lease_expiry(&expired).await.unwrap();

    // Command should be retry_scheduled
    let updated = store.get_command(&cmd.command_id).await.unwrap().unwrap();
    assert_eq!(updated.status, CommandStatus::RetryScheduled);
    assert!(updated.next_retry_at.is_some());
    assert_eq!(updated.last_error.as_deref(), Some("lease expired"));
    assert!(updated.current_attempt_id.is_none());
    assert!(matches!(
        store
            .mark_command_succeeded_for_attempt(&cmd.command_id, &lease.attempt_id)
            .await,
        Err(SessionError::StaleAttempt { .. })
    ));

    // Session epoch should have been bumped
    let session = store.get_session(&sk).await.unwrap().unwrap();
    assert_eq!(session.epoch, 2); // was 1, bumped to 2
    assert_eq!(session.status, SessionStatus::Idle);
}

#[tokio::test]
async fn test_handle_lease_expiry_dead_letter() {
    let store = PgSessionStore::new(&test_database_url());
    store.health_check().await.expect("DB not reachable");

    let since = Utc::now();
    let sk = unique_key("sess");
    store.upsert_session(&sk, "a", "c").await.unwrap();

    let input = test_insert_command_input(&sk);
    let cmd = store.insert_command(&input).await.unwrap();
    let lease = store
        .assign_lease(&cmd.command_id, "executor-1")
        .await
        .unwrap();
    store
        .update_command_status(&CommandStatusUpdate {
            command_id: cmd.command_id.clone(),
            status: CommandStatus::Leased,
            attempt_count: Some(5),
            ..Default::default()
        })
        .await
        .unwrap();

    // Handle expiry (attempt_count=5, max=5 => should dead-letter)
    let expired = ExpiredLease {
        session_key: sk.clone(),
        command_id: cmd.command_id.clone(),
        epoch: 1,
        attempt_count: 5,
        max_attempts: 5,
    };
    store.handle_lease_expiry(&expired).await.unwrap();

    // Command should be dead_letter
    let updated = store.get_command(&cmd.command_id).await.unwrap().unwrap();
    assert_eq!(updated.status, CommandStatus::DeadLetter);
    assert!(updated.current_attempt_id.is_none());
    assert!(matches!(
        store
            .mark_command_succeeded_for_attempt(&cmd.command_id, &lease.attempt_id)
            .await,
        Err(SessionError::StaleAttempt { .. })
    ));

    // A dead letter record should have been created.
    assert!(
        find_dead_letter_by_source_id(&store, since, &cmd.command_id)
            .await
            .is_some(),
        "expected a dead_letter entry for source_id={}",
        cmd.command_id,
    );
    assert!(matches!(
        store.handle_lease_expiry(&expired).await,
        Err(SessionError::InvalidStateTransition { .. })
    ));
}

#[tokio::test]
async fn expired_batch_members_share_one_epoch_fence_and_all_retry() {
    let store = PgSessionStore::new(&test_database_url());
    store.health_check().await.expect("DB not reachable");

    let sk = unique_key("expired-batch-session");
    let agent_id = unique_key("expired-batch-agent");
    let conv_id = unique_key("expired-batch-conv");
    store
        .upsert_session(&sk, &agent_id, &conv_id)
        .await
        .unwrap();
    let first = store
        .insert_command(&test_insert_command_input_for(&sk, &agent_id, &conv_id))
        .await
        .unwrap();
    let second = store
        .insert_command(&test_insert_command_input_for(&sk, &agent_id, &conv_id))
        .await
        .unwrap();
    let leases = store
        .assign_batch_leases(
            &[first.command_id.clone(), second.command_id.clone()],
            "batch-executor",
        )
        .await
        .unwrap();

    let second_lease = &leases[&second.command_id];
    assert!(
        store
            .command_attempt_is_current(&second.command_id, &second_lease.attempt_id)
            .await
            .unwrap()
    );
    store
        .update_command_status_for_attempt(
            &CommandStatusUpdate {
                command_id: second.command_id.clone(),
                status: CommandStatus::Heartbeating,
                ..Default::default()
            },
            &second_lease.attempt_id,
        )
        .await
        .unwrap();

    let first_lease = &leases[&first.command_id];
    store
        .handle_lease_expiry(&ExpiredLease {
            session_key: sk.clone(),
            command_id: first.command_id.clone(),
            epoch: first_lease.epoch,
            attempt_count: first_lease.attempt_count,
            max_attempts: first.max_attempts,
        })
        .await
        .unwrap();

    let second_after_epoch_bump = store
        .get_command(&second.command_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        second_after_epoch_bump.current_attempt_id.as_deref(),
        Some(second_lease.attempt_id.as_str()),
        "the command attempt stays unchanged so this isolates epoch fencing"
    );
    assert!(
        !store
            .command_attempt_is_current(&second.command_id, &second_lease.attempt_id)
            .await
            .unwrap()
    );
    let stale_update = CommandStatusUpdate {
        command_id: second.command_id.clone(),
        status: CommandStatus::RetryScheduled,
        last_error: Some("old epoch update".to_string()),
        ..Default::default()
    };
    assert!(matches!(
        store
            .update_command_status_for_attempt(&stale_update, &second_lease.attempt_id)
            .await,
        Err(SessionError::StaleAttempt { .. })
    ));
    assert!(matches!(
        store
            .mark_command_succeeded_for_attempt(&second.command_id, &second_lease.attempt_id)
            .await,
        Err(SessionError::StaleAttempt { .. })
    ));
    assert!(matches!(
        store
            .mark_command_committed_for_attempt(&second.command_id, &second_lease.attempt_id)
            .await,
        Err(SessionError::StaleAttempt { .. })
    ));
    assert!(matches!(
        store
            .dead_letter_command_for_attempt(
                &InsertDeadLetter {
                    source_type: "command".to_string(),
                    source_id: second.command_id.clone(),
                    payload: serde_json::json!({"reason": "old epoch"}),
                    error: "old epoch dead letter".to_string(),
                    attempt_count: second_lease.attempt_count,
                },
                &second_lease.attempt_id,
            )
            .await,
        Err(SessionError::StaleAttempt { .. })
    ));
    store
        .handle_lease_expiry(&ExpiredLease {
            session_key: sk.clone(),
            command_id: second.command_id.clone(),
            epoch: second_lease.epoch,
            attempt_count: second_lease.attempt_count,
            max_attempts: second.max_attempts,
        })
        .await
        .unwrap();

    let first = store.get_command(&first.command_id).await.unwrap().unwrap();
    let second = store
        .get_command(&second.command_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.status, CommandStatus::RetryScheduled);
    assert_eq!(second.status, CommandStatus::RetryScheduled);
    let session = store.get_session(&sk).await.unwrap().unwrap();
    assert_eq!(
        session.epoch, 2,
        "the shared lease epoch must be fenced once"
    );
    assert_eq!(session.status, SessionStatus::Idle);
}

#[tokio::test]
async fn stale_attempt_cannot_overwrite_reassigned_command_or_heartbeat() {
    let store = PgSessionStore::new(&test_database_url());
    store.health_check().await.expect("DB not reachable");

    let since = Utc::now();
    let sk = unique_key("stale-attempt-session");
    store
        .upsert_session(&sk, "agent-stale", "conv-stale")
        .await
        .unwrap();
    let cmd = store
        .insert_command(&test_insert_command_input(&sk))
        .await
        .unwrap();

    let first = store
        .assign_lease(&cmd.command_id, "executor-old")
        .await
        .unwrap();
    store
        .handle_lease_expiry(&ExpiredLease {
            session_key: sk.clone(),
            command_id: cmd.command_id.clone(),
            epoch: first.epoch,
            attempt_count: first.attempt_count,
            max_attempts: 5,
        })
        .await
        .unwrap();
    // Simulate run_retry_loop moving a due retry back to pending.
    store
        .update_command_status(&CommandStatusUpdate {
            command_id: cmd.command_id.clone(),
            status: CommandStatus::Pending,
            ..Default::default()
        })
        .await
        .unwrap();
    let second = store
        .assign_lease(&cmd.command_id, "executor-new")
        .await
        .unwrap();
    assert_ne!(first.attempt_id, second.attempt_id);
    assert!(second.epoch > first.epoch);
    assert!(matches!(
        store
            .handle_lease_expiry(&ExpiredLease {
                session_key: sk.clone(),
                command_id: cmd.command_id.clone(),
                epoch: first.epoch,
                attempt_count: first.attempt_count,
                max_attempts: 5,
            })
            .await,
        Err(SessionError::EpochMismatch { .. })
    ));

    let stale_update = CommandStatusUpdate {
        command_id: cmd.command_id.clone(),
        status: CommandStatus::RetryScheduled,
        last_error: Some("late old failure".into()),
        ..Default::default()
    };
    assert!(matches!(
        store
            .update_command_status_for_attempt(&stale_update, &first.attempt_id)
            .await,
        Err(SessionError::StaleAttempt { .. })
    ));
    assert!(matches!(
        store
            .mark_command_succeeded_for_attempt(&cmd.command_id, &first.attempt_id)
            .await,
        Err(SessionError::StaleAttempt { .. })
    ));
    assert!(matches!(
        store
            .mark_command_committed_for_attempt(&cmd.command_id, &first.attempt_id)
            .await,
        Err(SessionError::StaleAttempt { .. })
    ));
    assert!(matches!(
        store
            .update_session_heartbeat_for_epoch(&sk, first.epoch)
            .await,
        Err(SessionError::EpochMismatch { .. })
    ));
    let stale_dead_letter = InsertDeadLetter {
        source_type: "command".into(),
        source_id: cmd.command_id.clone(),
        payload: serde_json::json!({"attempt_id": first.attempt_id.clone()}),
        error: "late old failure".into(),
        attempt_count: first.attempt_count,
    };
    assert!(matches!(
        store
            .dead_letter_command_for_attempt(&stale_dead_letter, &first.attempt_id)
            .await,
        Err(SessionError::StaleAttempt { .. })
    ));

    let current = store.get_command(&cmd.command_id).await.unwrap().unwrap();
    assert_eq!(current.status, CommandStatus::Leased);
    assert_eq!(
        current.current_attempt_id.as_deref(),
        Some(second.attempt_id.as_str())
    );
    assert_eq!(current.current_epoch, Some(second.epoch));
    assert_eq!(current.attempt_count, second.attempt_count);
    let session = store.get_session(&sk).await.unwrap().unwrap();
    assert_eq!(session.status, SessionStatus::Active);
    assert_eq!(session.epoch, second.epoch);
    assert_eq!(session.executor_node_id.as_deref(), Some("executor-new"));
    assert!(
        find_dead_letter_by_source_id(&store, since, &cmd.command_id)
            .await
            .is_none()
    );

    store
        .mark_command_succeeded_for_attempt(&cmd.command_id, &second.attempt_id)
        .await
        .expect("the current attempt must retain ownership");
    store
        .mark_command_committed_for_attempt(&cmd.command_id, &second.attempt_id)
        .await
        .expect("the current attempt must commit and release its session");
    let committed = store.get_command(&cmd.command_id).await.unwrap().unwrap();
    assert_eq!(committed.status, CommandStatus::Committed);
    assert!(committed.current_attempt_id.is_none());
    let released = store.get_session(&sk).await.unwrap().unwrap();
    assert_eq!(released.status, SessionStatus::Idle);
    assert!(released.executor_node_id.is_none());
}

// =========================================================================
// D6: Dead letter operations
// =========================================================================

#[tokio::test]
async fn test_insert_and_list_dead_letters() {
    let store = PgSessionStore::new(&test_database_url());
    store.health_check().await.expect("DB not reachable");

    let since = Utc::now();
    let source_id = unique_key("cmd");
    let dl = store
        .insert_dead_letter(&InsertDeadLetter {
            source_type: "command".to_string(),
            source_id: source_id.clone(),
            payload: serde_json::json!({"test": true}),
            error: "test error".to_string(),
            attempt_count: 5,
        })
        .await
        .unwrap();

    assert_eq!(dl.source_type, "command");
    assert_eq!(dl.source_id, source_id);
    assert!(dl.resolved_at.is_none());

    assert!(
        find_dead_letter_by_source_id(&store, since, &source_id)
            .await
            .is_some(),
        "expected dead_letter entry for source_id={source_id}",
    );
}

#[tokio::test]
async fn dead_letter_command_updates_status_and_record_together() {
    let store = PgSessionStore::new(&test_database_url());
    store.health_check().await.expect("DB not reachable");

    let since = Utc::now();
    let sk = unique_key("sess");
    store.upsert_session(&sk, "a", "c").await.unwrap();
    let command = store
        .insert_command(&test_insert_command_input(&sk))
        .await
        .unwrap();
    let second_command = store
        .insert_command(&test_insert_command_input(&sk))
        .await
        .unwrap();
    let leases = store
        .assign_batch_leases(
            &[
                command.command_id.clone(),
                second_command.command_id.clone(),
            ],
            "executor-1",
        )
        .await
        .unwrap();
    let lease = &leases[&command.command_id];

    store
        .dead_letter_command_for_attempt(
            &InsertDeadLetter {
                source_type: "command".to_string(),
                source_id: command.command_id.clone(),
                payload: serde_json::json!({"attempt_count": lease.attempt_count}),
                error: "configuration failure".to_string(),
                attempt_count: lease.attempt_count,
            },
            &lease.attempt_id,
        )
        .await
        .unwrap();

    let updated = store
        .get_command(&command.command_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.status, CommandStatus::DeadLetter);
    assert_eq!(updated.last_error.as_deref(), Some("configuration failure"));
    assert!(updated.current_attempt_id.is_none());
    let session = store.get_session(&sk).await.unwrap().unwrap();
    assert_eq!(session.status, SessionStatus::Active);

    let second_lease = &leases[&second_command.command_id];
    store
        .dead_letter_command_for_attempt(
            &InsertDeadLetter {
                source_type: "command".to_string(),
                source_id: second_command.command_id.clone(),
                payload: serde_json::json!({"attempt_count": second_lease.attempt_count}),
                error: "configuration failure".to_string(),
                attempt_count: second_lease.attempt_count,
            },
            &second_lease.attempt_id,
        )
        .await
        .unwrap();
    let session = store.get_session(&sk).await.unwrap().unwrap();
    assert_eq!(session.status, SessionStatus::Idle);
    assert!(session.executor_node_id.is_none());
    assert!(session.last_heartbeat_at.is_none());
    let dead_letter = find_dead_letter_by_source_id(&store, since, &command.command_id)
        .await
        .expect("dead-letter record");
    assert_eq!(dead_letter.attempt_count, 1);
}

#[tokio::test]
async fn batch_commit_releases_session_only_after_last_active_command() {
    let store = PgSessionStore::new(&test_database_url());
    store.health_check().await.expect("DB not reachable");

    let sk = unique_key("batch-commit-session");
    store.upsert_session(&sk, "a", "c").await.unwrap();
    let first = store
        .insert_command(&test_insert_command_input(&sk))
        .await
        .unwrap();
    let second = store
        .insert_command(&test_insert_command_input(&sk))
        .await
        .unwrap();
    let leases = store
        .assign_batch_leases(
            &[first.command_id.clone(), second.command_id.clone()],
            "executor-1",
        )
        .await
        .unwrap();

    store
        .mark_command_committed_for_attempt(
            &first.command_id,
            &leases[&first.command_id].attempt_id,
        )
        .await
        .unwrap();
    let active = store.get_session(&sk).await.unwrap().unwrap();
    assert_eq!(active.status, SessionStatus::Active);
    assert_eq!(active.executor_node_id.as_deref(), Some("executor-1"));
    assert!(active.last_heartbeat_at.is_some());

    store
        .mark_command_committed_for_attempt(
            &second.command_id,
            &leases[&second.command_id].attempt_id,
        )
        .await
        .unwrap();
    let idle = store.get_session(&sk).await.unwrap().unwrap();
    assert_eq!(idle.status, SessionStatus::Idle);
    assert!(idle.executor_node_id.is_none());
    assert!(idle.last_heartbeat_at.is_none());
}

// =========================================================================
// D7: Executor registry
// =========================================================================

#[tokio::test]
async fn test_executor_registry() {
    let store = PgSessionStore::new(&test_database_url());

    // Register an executor
    let exec = store
        .register_executor("node-1", serde_json::json!({"gpu": true}))
        .await
        .unwrap();
    assert_eq!(exec.node_id, "node-1");
    assert_eq!(exec.status, ExecutorStatus::Available);

    // Find available executor
    let found = store.find_available_executor("any-agent").await.unwrap();
    assert_eq!(found.node_id, "node-1");

    // Update heartbeat
    store.update_executor_heartbeat("node-1").await.unwrap();

    // Deregister
    store.deregister_executor("node-1").await.unwrap();

    // No more available executors
    let result = store.find_available_executor("any-agent").await;
    assert!(result.is_err());
}

// =========================================================================
// D8: Full lifecycle integration tests
// =========================================================================

/// Happy path: pending -> leased -> completed
#[tokio::test]
async fn test_lifecycle_happy_path() {
    let store = PgSessionStore::new(&test_database_url());
    store.health_check().await.expect("DB not reachable");

    let sk = unique_key("sess");
    store
        .upsert_session(&sk, "agent-1", "conv-1")
        .await
        .unwrap();

    // 1. Insert command (pending)
    let input = test_insert_command_input(&sk);
    let cmd = store.insert_command(&input).await.unwrap();
    assert_eq!(cmd.status, CommandStatus::Pending);

    // 2. Assign lease (pending -> leased)
    let lease = store
        .assign_lease(&cmd.command_id, "executor-1")
        .await
        .unwrap();
    let cmd = store.get_command(&cmd.command_id).await.unwrap().unwrap();
    assert_eq!(cmd.status, CommandStatus::Leased);
    assert_eq!(cmd.current_epoch, Some(lease.epoch));
    assert_eq!(cmd.attempt_count, 1);

    // 3. Validate epoch
    assert!(store.validate_epoch(&sk, lease.epoch).await.unwrap());

    // 4. Release lease (leased -> succeeded)
    store.release_lease(&cmd.command_id).await.unwrap();
    let cmd = store.get_command(&cmd.command_id).await.unwrap().unwrap();
    assert_eq!(cmd.status, CommandStatus::Succeeded);

    // Session back to idle
    let session = store.get_session(&sk).await.unwrap().unwrap();
    assert_eq!(session.status, SessionStatus::Idle);
}

/// Retry path: leased -> expired -> retry_scheduled -> pending (re-leased) -> completed
#[tokio::test]
async fn test_lifecycle_retry() {
    let store = PgSessionStore::new(&test_database_url());
    store.health_check().await.expect("DB not reachable");

    let sk = unique_key("sess");
    store
        .upsert_session(&sk, "agent-1", "conv-1")
        .await
        .unwrap();

    // 1. Insert and lease
    let input = test_insert_command_input(&sk);
    let cmd = store.insert_command(&input).await.unwrap();
    store
        .assign_lease(&cmd.command_id, "executor-1")
        .await
        .unwrap();

    // 2. Simulate lease expiry
    let expired = ExpiredLease {
        session_key: sk.clone(),
        command_id: cmd.command_id.clone(),
        epoch: 1,
        attempt_count: 1,
        max_attempts: 5,
    };
    store.handle_lease_expiry(&expired).await.unwrap();

    // 3. Verify retry_scheduled
    let cmd = store.get_command(&cmd.command_id).await.unwrap().unwrap();
    assert_eq!(cmd.status, CommandStatus::RetryScheduled);
    assert!(cmd.next_retry_at.is_some());

    // 4. Simulate retry scheduler picking it up: set status back to pending
    store
        .update_command_status(&CommandStatusUpdate {
            command_id: cmd.command_id.clone(),
            status: CommandStatus::Pending,
            next_retry_at: Some(None),
            ..Default::default()
        })
        .await
        .unwrap();

    // 5. Re-lease (second attempt)
    let lease2 = store
        .assign_lease(&cmd.command_id, "executor-2")
        .await
        .unwrap();
    assert_eq!(lease2.epoch, 3); // epoch: 0 -> 1 (first lease) -> 2 (expiry bump) -> 3 (second lease)

    let cmd = store.get_command(&cmd.command_id).await.unwrap().unwrap();
    assert_eq!(cmd.status, CommandStatus::Leased);
    assert_eq!(cmd.attempt_count, 2);

    // 6. Complete
    store.release_lease(&cmd.command_id).await.unwrap();
    let cmd = store.get_command(&cmd.command_id).await.unwrap().unwrap();
    assert_eq!(cmd.status, CommandStatus::Succeeded);
}

/// Dead letter path: 5 failures -> dead_letter
#[tokio::test]
async fn test_lifecycle_dead_letter() {
    let store = PgSessionStore::new(&test_database_url());
    store.health_check().await.expect("DB not reachable");

    let since = Utc::now();
    let sk = unique_key("sess");
    store
        .upsert_session(&sk, "agent-1", "conv-1")
        .await
        .unwrap();

    let input = test_insert_command_input(&sk);
    let cmd = store.insert_command(&input).await.unwrap();

    // Assign first lease
    store
        .assign_lease(&cmd.command_id, "executor-1")
        .await
        .unwrap();
    store
        .update_command_status(&CommandStatusUpdate {
            command_id: cmd.command_id.clone(),
            status: CommandStatus::Leased,
            attempt_count: Some(5),
            ..Default::default()
        })
        .await
        .unwrap();

    // Simulate exhausted retries
    let expired = ExpiredLease {
        session_key: sk.clone(),
        command_id: cmd.command_id.clone(),
        epoch: 1,
        attempt_count: 5,
        max_attempts: 5,
    };
    store.handle_lease_expiry(&expired).await.unwrap();

    // Command should be dead_letter
    let cmd = store.get_command(&cmd.command_id).await.unwrap().unwrap();
    assert_eq!(cmd.status, CommandStatus::DeadLetter);

    // Dead letter record exists
    let dl = find_dead_letter_by_source_id(&store, since, &cmd.command_id).await;
    assert!(dl.is_some(), "expected dead_letter for {}", cmd.command_id);
    let dl = dl.unwrap();
    assert_eq!(dl.source_type, "command");
    assert!(dl.error.contains("max attempts exceeded"));
}
