use super::{
    is_principal_id_like, metadata_for_group_send_command, parse_dev_env_value,
    process_single_outbox_command, resolve_names_to_ids, send_to_group,
};
use choruz_store::EventStore;
use std::{
    sync::{Mutex, MutexGuard},
    time::{Duration, SystemTime},
};
use tempfile::tempdir;
use tokio_postgres::NoTls;

static CHANNEL_TASK_ENV_LOCK: Mutex<()> = Mutex::new(());

struct ChannelTaskEnvGuard {
    _guard: MutexGuard<'static, ()>,
    saved: Option<String>,
    saved_agent_tokens_file: Option<String>,
}

impl ChannelTaskEnvGuard {
    fn enabled() -> Self {
        let guard = CHANNEL_TASK_ENV_LOCK.lock().expect("channel task env lock");
        let saved = std::env::var("CHORUZ_PLUGINS").ok();
        let saved_agent_tokens_file = std::env::var("CHORUZ_AGENT_TOKENS_FILE").ok();
        // SAFETY: this test guard serializes mutations of these env vars
        // inside the choruz-pipeline test process.
        unsafe {
            std::env::set_var("CHORUZ_PLUGINS", "kanban,pixel-world");
        }
        Self {
            _guard: guard,
            saved,
            saved_agent_tokens_file,
        }
    }

    fn disabled() -> Self {
        let guard = CHANNEL_TASK_ENV_LOCK.lock().expect("channel task env lock");
        let saved = std::env::var("CHORUZ_PLUGINS").ok();
        let saved_agent_tokens_file = std::env::var("CHORUZ_AGENT_TOKENS_FILE").ok();
        // SAFETY: this test guard serializes mutations of this env var inside
        // the choruz-pipeline test process.
        unsafe {
            std::env::set_var("CHORUZ_PLUGINS", "pixel-world");
        }
        Self {
            _guard: guard,
            saved,
            saved_agent_tokens_file,
        }
    }

    fn with_agent_tokens_file(self, path: &std::path::Path) -> Self {
        // SAFETY: this guard holds CHANNEL_TASK_ENV_LOCK while mutating the env var.
        unsafe {
            std::env::set_var("CHORUZ_AGENT_TOKENS_FILE", path);
        }
        self
    }
}

impl Drop for ChannelTaskEnvGuard {
    fn drop(&mut self) {
        // SAFETY: this guard still holds CHANNEL_TASK_ENV_LOCK while restoring.
        unsafe {
            match &self.saved {
                Some(value) => std::env::set_var("CHORUZ_PLUGINS", value),
                None => std::env::remove_var("CHORUZ_PLUGINS"),
            }
            match &self.saved_agent_tokens_file {
                Some(value) => std::env::set_var("CHORUZ_AGENT_TOKENS_FILE", value),
                None => std::env::remove_var("CHORUZ_AGENT_TOKENS_FILE"),
            }
        }
    }
}

/// Assert that every key in `expected` matches the corresponding field in
/// `envelope`. Extra keys on `envelope` (e.g. the timestamped `emitted_at`,
/// or `idempotency_key` when the test does not pin it) are tolerated.
/// Tests that *do* want to pin those extra fields should assert them
/// directly on the envelope.
fn assert_envelope_matches(envelope: &serde_json::Value, expected: serde_json::Value) {
    let object = envelope
        .as_object()
        .expect("envelope must be a JSON object");
    let expected_obj = expected
        .as_object()
        .expect("expected payload must be a JSON object literal");
    for (key, expected_value) in expected_obj {
        let actual = object
            .get(key)
            .unwrap_or_else(|| panic!("envelope missing expected key `{key}`: {envelope}"));
        assert_eq!(
            actual, expected_value,
            "envelope key `{key}` mismatch (envelope = {envelope})"
        );
    }
    // emitted_at is non-deterministic but must always be present and parse
    // as RFC3339 — without it the envelope cannot satisfy the agent
    // contract.
    let emitted_at = object
        .get("emitted_at")
        .and_then(|value| value.as_str())
        .expect("envelope must carry emitted_at");
    chrono::DateTime::parse_from_rfc3339(emitted_at)
        .unwrap_or_else(|_| panic!("emitted_at `{emitted_at}` must be RFC3339"));
}

#[test]
fn parses_exported_dev_env_value() {
    let contents = r#"
export CHORUZ_SESSION_SECRET=abc
export CHORUZ_OPERATOR_PASSWORD=local-secret
"#;

    assert_eq!(
        parse_dev_env_value(contents, "CHORUZ_OPERATOR_PASSWORD").as_deref(),
        Some("local-secret")
    );
}

#[test]
fn parses_quoted_dev_env_value() {
    let contents = r#"export CHORUZ_OPERATOR_PASSWORD="local-secret""#;

    assert_eq!(
        parse_dev_env_value(contents, "CHORUZ_OPERATOR_PASSWORD").as_deref(),
        Some("local-secret")
    );
}

#[test]
fn detects_principal_ids_without_treating_long_names_as_ids() {
    assert!(is_principal_id_like("019e1a12-7d40-73f0-9147-676d33ab0c4b"));
    assert!(!is_principal_id_like("backend-dev-1778553877992-s1y4"));
    assert!(!is_principal_id_like("frontend-dev"));
}

#[tokio::test]
async fn uuid_shaped_member_name_resolves_before_raw_id_fallback() {
    let Ok(db_url) = std::env::var("CHORUZ_DATABASE_URL") else {
        return;
    };

    let workspace_id = choruz_common::new_id();
    let principal_id = choruz_common::new_id();
    let uuid_shaped_name = choruz_ids::MessageId::new().to_string();
    let raw_member_id = choruz_ids::MessageId::new().to_string();

    let (client, connection) = tokio_postgres::connect(&db_url, NoTls)
        .await
        .expect("connect for uuid-shaped member name test");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "INSERT INTO principal (id, workspace_id, type, name, disabled, created_at, updated_at)
                 VALUES ($1, $2, 'agent', $3, FALSE, NOW(), NOW())",
            &[&principal_id, &workspace_id, &uuid_shaped_name],
        )
        .await
        .expect("seed uuid-shaped principal name");

    let store = EventStore::new(&db_url);
    let resolved = resolve_names_to_ids(
        Some(&store),
        &workspace_id,
        &[uuid_shaped_name.clone(), raw_member_id.clone()],
    )
    .await;

    assert_eq!(resolved, vec![principal_id, raw_member_id]);
}

#[tokio::test]
async fn send_to_missing_group_returns_visible_error() {
    let Ok(db_url) = std::env::var("CHORUZ_DATABASE_URL") else {
        return;
    };

    let workspace_id = choruz_common::new_id();
    let agent_id = choruz_common::new_id();
    let (client, connection) = tokio_postgres::connect(&db_url, NoTls)
        .await
        .expect("connect for missing group test");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "INSERT INTO principal (id, workspace_id, type, name, disabled, created_at, updated_at)
                 VALUES ($1, $2, 'agent', 'Sender Agent', FALSE, NOW(), NOW())",
            &[&agent_id, &workspace_id],
        )
        .await
        .expect("seed sender agent");

    let store = EventStore::new(&db_url);
    let tmp = tempdir().expect("temp workspace");
    let reply = process_single_outbox_command(
        "watcher:test",
        &agent_id,
        tmp.path(),
        "http://127.0.0.1:3000",
        Some(&store),
        &serde_json::json!({
            "type": "send",
            "group": choruz_ids::MessageId::new().to_string(),
            "content": "hello",
        }),
    )
    .await
    .expect("send command should produce visible reply");

    assert!(reply.contains("Failed to send to group"));
    assert!(reply.contains("was not found in this workspace"));
}

#[tokio::test]
async fn process_outbox_commands_claims_maildir_files_once() {
    let tmp = tempdir().expect("temp workspace");
    let maildir_new = tmp.path().join(".choruz-outbox").join("new");
    std::fs::create_dir_all(&maildir_new).expect("maildir new");
    std::fs::write(
        maildir_new.join("cmd.json"),
        r#"{"type":"send","content":"hello once"}"#,
    )
    .expect("write command");

    let (a, b) = tokio::join!(
        super::process_outbox_commands(
            "watcher:test",
            "agent-1",
            tmp.path(),
            "http://127.0.0.1:3000",
            None,
        ),
        super::process_outbox_commands(
            "watcher:test",
            "agent-1",
            tmp.path(),
            "http://127.0.0.1:3000",
            None,
        ),
    );

    let replies = [a, b];
    assert_eq!(
        replies
            .iter()
            .filter(|reply| reply.as_str() == "hello once")
            .count(),
        1
    );
    assert_eq!(
        replies
            .iter()
            .filter(|reply| reply.trim().is_empty())
            .count(),
        1
    );
}

#[tokio::test]
async fn process_outbox_commands_recovers_stale_processing_files() {
    let tmp = tempdir().expect("temp workspace");
    let maildir_new = tmp.path().join(".choruz-outbox").join("new");
    std::fs::create_dir_all(&maildir_new).expect("maildir new");
    let processing_path = maildir_new.join("cmd.json.old.processing");
    std::fs::write(
        &processing_path,
        r#"{"type":"send","content":"after crash"}"#,
    )
    .expect("write processing command");
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(&processing_path)
        .expect("open processing command");
    file.set_times(
        std::fs::FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(120)),
    )
    .expect("age processing command");

    let reply = super::process_outbox_commands(
        "watcher:test",
        "agent-1",
        tmp.path(),
        "http://127.0.0.1:3000",
        None,
    )
    .await;

    assert_eq!(reply, "after crash");
    assert!(
        std::fs::read_dir(&maildir_new)
            .expect("maildir readable")
            .next()
            .is_none(),
        "stale processing file should be removed after recovery"
    );
}

#[tokio::test]
async fn process_outbox_commands_refreshes_claim_mtime_for_old_backlog() {
    let tmp = tempdir().expect("temp workspace");
    let maildir_new = tmp.path().join(".choruz-outbox").join("new");
    std::fs::create_dir_all(&maildir_new).expect("maildir new");
    let path = maildir_new.join("old.json");
    std::fs::write(&path, r#"{"type":"send","content":"old backlog"}"#).expect("write old command");
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("open old command");
    file.set_times(
        std::fs::FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(120)),
    )
    .expect("age old command");

    let claimed = super::claim_outbox_file(&path)
        .await
        .expect("claim old file");

    assert!(!super::is_stale_processing_file(&claimed));
    std::fs::remove_file(claimed).expect("cleanup claimed file");
}

#[tokio::test]
async fn process_outbox_commands_returns_all_visible_replies() {
    let tmp = tempdir().expect("temp workspace");
    let maildir_new = tmp.path().join(".choruz-outbox").join("new");
    std::fs::create_dir_all(&maildir_new).expect("maildir new");
    std::fs::write(
        maildir_new.join("001.json"),
        r#"{"type":"send","content":"one"}"#,
    )
    .expect("write first command");
    std::fs::write(
        maildir_new.join("002.json"),
        r#"{"type":"send","content":"two"}"#,
    )
    .expect("write second command");

    let reply = super::process_outbox_commands(
        "watcher:test",
        "agent-1",
        tmp.path(),
        "http://127.0.0.1:3000",
        None,
    )
    .await;

    assert_eq!(reply, "one\n\ntwo");
}

#[tokio::test]
async fn group_send_with_malformed_metadata_returns_visible_error() {
    let tmp = tempdir().expect("temp workspace");
    let reply = process_single_outbox_command(
        "watcher:test",
        "agent-1",
        tmp.path(),
        "http://127.0.0.1:3000",
        None,
        &serde_json::json!({
            "type": "send",
            "group": "team",
            "content": "hello",
            "metadata": ["not", "an", "object"],
        }),
    )
    .await
    .expect("send command should produce visible reply");

    assert_eq!(
        reply,
        "Failed to send to group 'team': metadata must be a JSON object."
    );
}

#[tokio::test]
async fn direct_send_with_metadata_still_returns_content() {
    let tmp = tempdir().expect("temp workspace");
    let reply = process_single_outbox_command(
        "watcher:test",
        "agent-1",
        tmp.path(),
        "http://127.0.0.1:3000",
        None,
        &serde_json::json!({
            "type": "send",
            "content": "hello direct",
            "metadata": {
                "workflow": {
                    "kind": "task.feedback",
                    "task_key": "DOC-P0-03",
                },
            },
        }),
    )
    .await
    .expect("direct send should return content for writer insertion");

    assert_eq!(reply, "hello direct");
}

#[tokio::test]
async fn task_commands_are_rejected_as_non_chat_failures_when_gate_is_off() {
    let _env = ChannelTaskEnvGuard::disabled();
    for command_type in ["task_create", "task_update", "task_transfer"] {
        let tmp = tempdir().expect("temp workspace");
        let maildir_new = tmp.path().join(".choruz-outbox").join("new");
        std::fs::create_dir_all(&maildir_new).expect("maildir new");
        let command = serde_json::json!({
            "type": command_type,
            "group": "team",
            "task_key": "CHAN-1",
            "title": "Draft API contract",
            "idempotency_key": "turn-1:task"
        });
        std::fs::write(maildir_new.join("task-command.json"), command.to_string())
            .expect("write task command");

        let result = super::process_outbox_commands_with_stats(
            "watcher:test",
            "agent-1",
            tmp.path(),
            "http://127.0.0.1:3000",
            None,
        )
        .await;

        assert_eq!(result.reply, "");
        assert_eq!(result.processed_count, 1);
        assert_eq!(result.command_results.len(), 1);
        assert_envelope_matches(
            &result.command_results[0],
            serde_json::json!({
                "command_type": command_type,
                "ok": false,
                "error_code": "channel_tasks_disabled",
                "message": choruz_common::plugins::KANBAN_PLUGIN_DISABLED_DETAIL,
                "task_key": "CHAN-1",
                "task_id": null,
                "idempotency_key": "turn-1:task",
            }),
        );

        let result_files = std::fs::read_dir(tmp.path().join(".choruz-outbox/results"))
            .expect("command result directory exists")
            .collect::<Result<Vec<_>, _>>()
            .expect("command result files are readable");
        assert_eq!(result_files.len(), 1);
        let persisted: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(result_files[0].path()).unwrap())
                .unwrap();
        assert_eq!(persisted, result.command_results[0]);
    }
}

#[tokio::test]
#[allow(clippy::type_complexity)]
async fn task_create_command_posts_to_gateway_as_non_chat_result() {
    let tmp = tempdir().expect("temp workspace");
    let token_path = tmp.path().join("agent_tokens.json");
    std::fs::write(&token_path, r#"{"agent-1":"agent-token-1"}"#).expect("write token file");
    let _env = ChannelTaskEnvGuard::enabled().with_agent_tokens_file(&token_path);
    let captured = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let captured_for_route = captured.clone();
    let app = axum::Router::new()
        .route(
            "/v1/conversations/{conversation_id}/tasks",
            axum::routing::post(
                |axum::extract::Path(conversation_id): axum::extract::Path<String>,
                 headers: axum::http::HeaderMap,
                 axum::extract::State(captured): axum::extract::State<
                    std::sync::Arc<
                        tokio::sync::Mutex<Vec<(String, Option<String>, serde_json::Value)>>,
                    >,
                >,
                 axum::Json(payload): axum::Json<serde_json::Value>| async move {
                    let auth = headers
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_string);
                    captured.lock().await.push((conversation_id, auth, payload));
                    axum::Json(serde_json::json!({
                        "task_id": "task-created-1",
                        "task_key": "BE-12"
                    }))
                },
            ),
        )
        .with_state(captured_for_route);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock gateway");
    let gateway_base_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    let maildir_new = tmp.path().join(".choruz-outbox").join("new");
    std::fs::create_dir_all(&maildir_new).expect("maildir new");
    let command = serde_json::json!({
        "type": "task_create",
        "conversation_id": "conv-1",
        "task_key": "BE-12",
        "title": "Design backend model and APIs",
        "status": "todo",
        "context_label": "Channel Kanban MVP",
        "idempotency_key": "turn-abc:BE-12"
    });
    std::fs::write(maildir_new.join("task-create.json"), command.to_string())
        .expect("write task_create command");

    let result = super::process_outbox_commands_with_stats(
        "watcher:test",
        "agent-1",
        tmp.path(),
        &gateway_base_url,
        None,
    )
    .await;

    assert_eq!(result.reply, "");
    assert_eq!(result.processed_count, 1);
    assert_eq!(result.command_results.len(), 1);
    assert_envelope_matches(
        &result.command_results[0],
        serde_json::json!({
            "command_type": "task_create",
            "ok": true,
            "task_key": "BE-12",
            "task_id": "task-created-1",
            "idempotency_key": "turn-abc:BE-12",
        }),
    );
    let captured = captured.lock().await;
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].0, "conv-1");
    assert_eq!(captured[0].1.as_deref(), Some("Bearer agent-token-1"));
    assert_eq!(
        captured[0].2,
        serde_json::json!({
            "task_key": "BE-12",
            "title": "Design backend model and APIs",
            "status": "todo",
            "context_label": "Channel Kanban MVP",
            "idempotency_key": "turn-abc:BE-12"
        })
    );
    let result_files = std::fs::read_dir(tmp.path().join(".choruz-outbox/results"))
        .expect("command result directory exists")
        .collect::<Result<Vec<_>, _>>()
        .expect("command result files are readable");
    assert_eq!(result_files.len(), 1);
    let persisted: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(result_files[0].path()).unwrap()).unwrap();
    assert_eq!(persisted, result.command_results[0]);
    assert!(
        !maildir_new.join("task-create.json").exists(),
        "task_create command should be removed after processing"
    );
}

#[tokio::test]
#[allow(clippy::type_complexity)]
async fn task_create_command_omits_task_key_when_agent_does_not_supply_one() {
    let tmp = tempdir().expect("temp workspace");
    let token_path = tmp.path().join("agent_tokens.json");
    std::fs::write(&token_path, r#"{"agent-1":"agent-token-1"}"#).expect("write token file");
    let _env = ChannelTaskEnvGuard::enabled().with_agent_tokens_file(&token_path);
    let captured = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let captured_for_route = captured.clone();
    let app = axum::Router::new()
        .route(
            "/v1/conversations/{conversation_id}/tasks",
            axum::routing::post(
                |axum::extract::Path(conversation_id): axum::extract::Path<String>,
                 _headers: axum::http::HeaderMap,
                 axum::extract::State(captured): axum::extract::State<
                    std::sync::Arc<tokio::sync::Mutex<Vec<(String, serde_json::Value)>>>,
                >,
                 axum::Json(payload): axum::Json<serde_json::Value>| async move {
                    captured.lock().await.push((conversation_id, payload));
                    axum::Json(serde_json::json!({
                        "task_id": "task-created-auto",
                        "task_key": "TASK-1"
                    }))
                },
            ),
        )
        .with_state(captured_for_route);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock gateway");
    let gateway_base_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    let maildir_new = tmp.path().join(".choruz-outbox").join("new");
    std::fs::create_dir_all(&maildir_new).expect("maildir new");
    let command = serde_json::json!({
        "type": "task_create",
        "conversation_id": "conv-auto",
        "title": "Design backend model and APIs",
        "idempotency_key": "turn-auto:1"
    });
    std::fs::write(maildir_new.join("task-create.json"), command.to_string())
        .expect("write task_create command");

    let result = super::process_outbox_commands_with_stats(
        "watcher:test",
        "agent-1",
        tmp.path(),
        &gateway_base_url,
        None,
    )
    .await;

    assert_eq!(result.processed_count, 1);
    assert_eq!(result.command_results.len(), 1);
    let captured = captured.lock().await;
    assert_eq!(captured.len(), 1);
    let (_conv, payload) = &captured[0];
    assert!(
        payload.get("task_key").is_none(),
        "pipeline must omit task_key when the agent did not supply one, got {payload}"
    );
    assert_eq!(payload["title"], "Design backend model and APIs");
    assert_eq!(payload["idempotency_key"], "turn-auto:1");
}

#[tokio::test]
#[allow(clippy::type_complexity)]
async fn task_create_command_resolves_group_and_assignee_name_without_chat_event() {
    let Ok(db_url) = std::env::var("CHORUZ_DATABASE_URL") else {
        return;
    };

    let tmp = tempdir().expect("temp workspace");
    let workspace_id = choruz_common::new_id();
    let agent_id = choruz_common::new_id();
    let assignee_id = choruz_common::new_id();
    let human_id = choruz_common::new_id();
    let removed_agent_id = choruz_common::new_id();
    let internal_agent_id = choruz_common::new_id();
    let disabled_agent_id = choruz_common::new_id();
    let deleted_agent_id = choruz_common::new_id();
    let cross_workspace_agent_id = choruz_common::new_id();
    let conversation_id = choruz_common::new_id();
    let group_name = format!("task-command-group-{}", choruz_common::new_id());
    let token_path = tmp.path().join("agent_tokens.json");
    std::fs::write(
        &token_path,
        serde_json::json!({ agent_id.clone(): "agent-token-group" }).to_string(),
    )
    .expect("write token file");
    let _env = ChannelTaskEnvGuard::enabled().with_agent_tokens_file(&token_path);

    let (client, connection) = tokio_postgres::connect(&db_url, NoTls)
        .await
        .expect("connect for task_create group test");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
            .execute(
                "INSERT INTO principal (id, workspace_id, type, name, disabled, channel_visibility, created_at, updated_at)
                 VALUES
                   ($1, $2, 'agent', 'Sender Agent', FALSE, 'visible', NOW(), NOW()),
                   ($3, $2, 'agent', 'Reviewer Agent', FALSE, 'visible', NOW(), NOW()),
                   ($4, $2, 'human', 'Human Planner', FALSE, 'visible', NOW(), NOW()),
                   ($5, $2, 'agent', 'Removed Agent', FALSE, 'visible', NOW(), NOW()),
                   ($6, $2, 'agent', 'Internal Helper', FALSE, 'internal', NOW(), NOW()),
                   ($7, $2, 'agent', 'Disabled Agent', TRUE, 'visible', NOW(), NOW()),
                   ($8, $2, 'agent', 'Deleted Agent', FALSE, 'visible', NOW(), NOW()),
                   ($9, 'cross-workspace', 'agent', 'Cross Workspace Agent', FALSE, 'visible', NOW(), NOW())",
                &[
                    &agent_id,
                    &workspace_id,
                    &assignee_id,
                    &human_id,
                    &removed_agent_id,
                    &internal_agent_id,
                    &disabled_agent_id,
                    &deleted_agent_id,
                    &cross_workspace_agent_id,
                ],
            )
            .await
            .expect("seed principals");
    client
        .execute(
            "UPDATE principal SET deleted_at = NOW() WHERE id = $1",
            &[&deleted_agent_id],
        )
        .await
        .expect("soft-delete deleted assignee fixture");
    client
            .execute(
                "INSERT INTO conversation (id, workspace_id, type, name, creator_id, created_at, updated_at)
                 VALUES ($1, $2, 'group', $3, $4, NOW(), NOW())",
                &[&conversation_id, &workspace_id, &group_name, &agent_id],
            )
            .await
            .expect("seed group conversation");
    client
        .execute(
            "INSERT INTO conversation_member (conv_id, principal_id, joined_at, removed_at)
                 VALUES ($1, $2, NOW(), NULL),
                        ($1, $3, NOW(), NULL),
                        ($1, $4, NOW(), NULL),
                        ($1, $5, NOW(), NOW()),
                        ($1, $6, NOW(), NULL),
                        ($1, $7, NOW(), NULL),
                        ($1, $8, NOW(), NULL),
                        ($1, $9, NOW(), NULL)",
            &[
                &conversation_id,
                &agent_id,
                &assignee_id,
                &human_id,
                &removed_agent_id,
                &internal_agent_id,
                &disabled_agent_id,
                &deleted_agent_id,
                &cross_workspace_agent_id,
            ],
        )
        .await
        .expect("seed group members");

    let captured = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let captured_for_route = captured.clone();
    let app = axum::Router::new()
        .route(
            "/v1/conversations/{conversation_id}/tasks",
            axum::routing::post(
                |axum::extract::Path(conversation_id): axum::extract::Path<String>,
                 headers: axum::http::HeaderMap,
                 axum::extract::State(captured): axum::extract::State<
                    std::sync::Arc<
                        tokio::sync::Mutex<Vec<(String, Option<String>, serde_json::Value)>>,
                    >,
                >,
                 axum::Json(payload): axum::Json<serde_json::Value>| async move {
                    let auth = headers
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_string);
                    captured.lock().await.push((conversation_id, auth, payload));
                    axum::Json(serde_json::json!({
                        "task_id": "task-created-group",
                        "task_key": "GRP-1"
                    }))
                },
            ),
        )
        .with_state(captured_for_route);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock gateway");
    let gateway_base_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    let store = EventStore::new(&db_url);
    let maildir_new = tmp.path().join(".choruz-outbox").join("new");
    std::fs::create_dir_all(&maildir_new).expect("maildir new");
    let commands = [
        serde_json::json!({
            "type": "task_create",
            "group": group_name,
            "task_key": "GRP-1",
            "title": "Review group-addressed task create",
            "assignee": "Reviewer Agent",
            "idempotency_key": "turn-group:GRP-1"
        }),
        serde_json::json!({
            "type": "task_create",
            "group": group_name,
            "task_key": "BAD-TEMPLATE",
            "title": "Template-only role must not be assignable",
            "assignee": "Frontend Engineer",
            "idempotency_key": "turn-group:bad-template"
        }),
        serde_json::json!({
            "type": "task_create",
            "group": group_name,
            "task_key": "BAD-HUMAN",
            "title": "Human must not be assignable by agents",
            "assignee": "Human Planner",
            "idempotency_key": "turn-group:bad-human"
        }),
        serde_json::json!({
            "type": "task_create",
            "group": group_name,
            "task_key": "BAD-REMOVED",
            "title": "Removed member must not be assignable",
            "assignee": "Removed Agent",
            "idempotency_key": "turn-group:bad-removed"
        }),
        serde_json::json!({
            "type": "task_create",
            "group": group_name,
            "task_key": "BAD-INTERNAL",
            "title": "Internal agent must not be assignable",
            "assignee": "Internal Helper",
            "idempotency_key": "turn-group:bad-internal"
        }),
        serde_json::json!({
            "type": "task_create",
            "group": group_name,
            "task_key": "BAD-DISABLED",
            "title": "Disabled agent must not be assignable",
            "assignee": "Disabled Agent",
            "idempotency_key": "turn-group:bad-disabled"
        }),
        serde_json::json!({
            "type": "task_create",
            "group": group_name,
            "task_key": "BAD-DELETED",
            "title": "Deleted agent must not be assignable",
            "assignee": "Deleted Agent",
            "idempotency_key": "turn-group:bad-deleted"
        }),
        serde_json::json!({
            "type": "task_create",
            "group": group_name,
            "task_key": "BAD-CROSS-WORKSPACE",
            "title": "Cross-workspace agent must not be assignable",
            "assignee": "Cross Workspace Agent",
            "idempotency_key": "turn-group:bad-cross-workspace"
        }),
        serde_json::json!({
            "type": "task_create",
            "group": group_name,
            "task_key": "GRP-ID",
            "title": "Review group-addressed task create by id",
            "assignee_principal_id": assignee_id,
            "idempotency_key": "turn-group:GRP-ID"
        }),
        serde_json::json!({
            "type": "task_create",
            "group": group_name,
            "task_key": "BAD-CASE-PRINCIPAL-ID",
            "title": "Case-variant principal id must not be assignable",
            "assignee_principal_id": assignee_id.to_ascii_uppercase(),
            "idempotency_key": "turn-group:bad-case-principal-id"
        }),
        serde_json::json!({
            "type": "task_create",
            "group": group_name,
            "task_key": "BAD-HUMAN-ID",
            "title": "Raw human principal id must not be assignable",
            "assignee": human_id,
            "idempotency_key": "turn-group:bad-human-id"
        }),
        serde_json::json!({
            "type": "task_create",
            "group": group_name,
            "task_key": "BAD-INTERNAL-ID",
            "title": "Raw internal principal id must not be assignable",
            "assignee": internal_agent_id,
            "idempotency_key": "turn-group:bad-internal-id"
        }),
        serde_json::json!({
            "type": "task_create",
            "group": group_name,
            "task_key": "BAD-HUMAN-PRINCIPAL-ID",
            "title": "Explicit human principal id must not be assignable",
            "assignee_principal_id": human_id,
            "idempotency_key": "turn-group:bad-human-principal-id"
        }),
        serde_json::json!({
            "type": "task_create",
            "group": group_name,
            "task_key": "BAD-INTERNAL-PRINCIPAL-ID",
            "title": "Explicit internal principal id must not be assignable",
            "assignee_principal_id": internal_agent_id,
            "idempotency_key": "turn-group:bad-internal-principal-id"
        }),
        serde_json::json!({
            "type": "task_create",
            "group": group_name,
            "task_key": "BAD-REMOVED-PRINCIPAL-ID",
            "title": "Explicit removed principal id must not be assignable",
            "assignee_principal_id": removed_agent_id,
            "idempotency_key": "turn-group:bad-removed-principal-id"
        }),
        serde_json::json!({
            "type": "task_create",
            "group": group_name,
            "task_key": "BAD-DISABLED-PRINCIPAL-ID",
            "title": "Explicit disabled principal id must not be assignable",
            "assignee_principal_id": disabled_agent_id,
            "idempotency_key": "turn-group:bad-disabled-principal-id"
        }),
        serde_json::json!({
            "type": "task_create",
            "group": group_name,
            "task_key": "BAD-DELETED-PRINCIPAL-ID",
            "title": "Explicit deleted principal id must not be assignable",
            "assignee_principal_id": deleted_agent_id,
            "idempotency_key": "turn-group:bad-deleted-principal-id"
        }),
        serde_json::json!({
            "type": "task_create",
            "group": group_name,
            "task_key": "BAD-CROSS-WORKSPACE-PRINCIPAL-ID",
            "title": "Explicit cross-workspace principal id must not be assignable",
            "assignee_principal_id": cross_workspace_agent_id,
            "idempotency_key": "turn-group:bad-cross-workspace-principal-id"
        }),
    ];
    for (index, command) in commands.into_iter().enumerate() {
        std::fs::write(
            maildir_new.join(format!("task-create-group-{index}.json")),
            command.to_string(),
        )
        .expect("write group task_create command");
    }

    let result = super::process_outbox_commands_with_stats(
        "watcher:test",
        &agent_id,
        tmp.path(),
        &gateway_base_url,
        Some(&store),
    )
    .await;

    assert_eq!(result.reply, "");
    assert_eq!(result.processed_count, 18);
    let success = result
        .command_results
        .iter()
        .find(|item| item["idempotency_key"] == "turn-group:GRP-1")
        .expect("visible roster assignee command result");
    assert_eq!(success["ok"], true);
    assert_eq!(success["task_id"], "task-created-group");
    for idempotency_key in [
        "turn-group:bad-template",
        "turn-group:bad-human",
        "turn-group:bad-removed",
        "turn-group:bad-internal",
        "turn-group:bad-disabled",
        "turn-group:bad-deleted",
        "turn-group:bad-cross-workspace",
        "turn-group:bad-case-principal-id",
        "turn-group:bad-human-id",
        "turn-group:bad-internal-id",
        "turn-group:bad-human-principal-id",
        "turn-group:bad-internal-principal-id",
        "turn-group:bad-removed-principal-id",
        "turn-group:bad-disabled-principal-id",
        "turn-group:bad-deleted-principal-id",
        "turn-group:bad-cross-workspace-principal-id",
    ] {
        let envelope = result
            .command_results
            .iter()
            .find(|item| item["idempotency_key"] == idempotency_key)
            .unwrap_or_else(|| panic!("missing command result for {idempotency_key}"));
        assert_eq!(envelope["command_type"], "task_create");
        assert_eq!(envelope["ok"], false);
        assert_eq!(envelope["error_code"], "invalid_assignee");
        assert!(envelope["task_id"].is_null());
    }
    let captured = captured.lock().await;
    assert_eq!(captured.len(), 2);
    for (captured_conversation_id, auth, _) in captured.iter() {
        assert_eq!(captured_conversation_id, &conversation_id);
        assert_eq!(auth.as_deref(), Some("Bearer agent-token-group"));
    }
    let payload_for = |idempotency_key: &str| {
        captured
            .iter()
            .map(|(_, _, payload)| payload)
            .find(|payload| payload["idempotency_key"] == idempotency_key)
            .unwrap_or_else(|| panic!("missing gateway payload for {idempotency_key}"))
    };
    assert_eq!(
        payload_for("turn-group:GRP-1"),
        &serde_json::json!({
            "task_key": "GRP-1",
            "title": "Review group-addressed task create",
            "assignee_principal_id": assignee_id,
            "idempotency_key": "turn-group:GRP-1"
        })
    );
    assert_eq!(
        payload_for("turn-group:GRP-ID"),
        &serde_json::json!({
            "task_key": "GRP-ID",
            "title": "Review group-addressed task create by id",
            "assignee_principal_id": assignee_id,
            "idempotency_key": "turn-group:GRP-ID"
        })
    );
    let chat_event_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
                 FROM conversation_events
                 WHERE conversation_id = $1",
            &[&conversation_id],
        )
        .await
        .expect("count chat events")
        .get::<_, i64>("count");
    assert_eq!(chat_event_count, 0);
}

#[tokio::test]
async fn task_update_command_patches_gateway_as_non_chat_result() {
    let tmp = tempdir().expect("temp workspace");
    let token_path = tmp.path().join("agent_tokens.json");
    std::fs::write(&token_path, r#"{"agent-1":"agent-token-1"}"#).expect("write token file");
    let _env = ChannelTaskEnvGuard::enabled().with_agent_tokens_file(&token_path);
    let captured = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let captured_for_route = captured.clone();
    let app = axum::Router::new()
        .route(
            "/v1/conversations/{conversation_id}/tasks",
            axum::routing::get(
                |axum::extract::Path(conversation_id): axum::extract::Path<String>,
                 headers: axum::http::HeaderMap,
                 axum::extract::State(captured): axum::extract::State<
                    std::sync::Arc<tokio::sync::Mutex<Vec<serde_json::Value>>>,
                >| async move {
                    let auth = headers
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_string);
                    captured.lock().await.push(serde_json::json!({
                        "method": "GET",
                        "conversation_id": conversation_id,
                        "auth": auth,
                    }));
                    axum::Json(serde_json::json!([
                        {
                            "task_id": "task-123",
                            "task_key": "BE-12"
                        }
                    ]))
                },
            ),
        )
        .route(
            "/v1/tasks/{task_id}",
            axum::routing::patch(
                |axum::extract::Path(task_id): axum::extract::Path<String>,
                 headers: axum::http::HeaderMap,
                 axum::extract::State(captured): axum::extract::State<
                    std::sync::Arc<tokio::sync::Mutex<Vec<serde_json::Value>>>,
                >,
                 axum::Json(payload): axum::Json<serde_json::Value>| async move {
                    let auth = headers
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_string);
                    captured.lock().await.push(serde_json::json!({
                        "method": "PATCH",
                        "task_id": task_id,
                        "auth": auth,
                        "payload": payload,
                    }));
                    axum::Json(serde_json::json!({
                        "task_id": "task-123",
                        "task_key": "BE-12"
                    }))
                },
            ),
        )
        .with_state(captured_for_route);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock gateway");
    let gateway_base_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    let maildir_new = tmp.path().join(".choruz-outbox").join("new");
    std::fs::create_dir_all(&maildir_new).expect("maildir new");
    let command = serde_json::json!({
        "type": "task_update",
        "conversation_id": "conv-1",
        "task_key": "BE-12",
        "status": "blocked",
        "blocked_reason": "Need product decision",
        "context_label": null,
        "idempotency_key": "turn-up:BE-12"
    });
    std::fs::write(maildir_new.join("task-update.json"), command.to_string())
        .expect("write task_update command");

    let result = super::process_outbox_commands_with_stats(
        "watcher:test",
        "agent-1",
        tmp.path(),
        &gateway_base_url,
        None,
    )
    .await;

    assert_eq!(result.reply, "");
    assert_eq!(result.processed_count, 1);
    assert_eq!(result.command_results.len(), 1);
    assert_envelope_matches(
        &result.command_results[0],
        serde_json::json!({
            "command_type": "task_update",
            "ok": true,
            "task_key": "BE-12",
            "task_id": "task-123",
            "idempotency_key": "turn-up:BE-12",
        }),
    );
    let captured = captured.lock().await;
    assert_eq!(
        *captured,
        vec![
            serde_json::json!({
                "method": "GET",
                "conversation_id": "conv-1",
                "auth": "Bearer agent-token-1",
            }),
            serde_json::json!({
                "method": "PATCH",
                "task_id": "task-123",
                "auth": "Bearer agent-token-1",
                "payload": {
                    "status": "blocked",
                    "blocked_reason": "Need product decision",
                    "context_label": null
                }
            })
        ]
    );
    let result_files = std::fs::read_dir(tmp.path().join(".choruz-outbox/results"))
        .expect("command result directory exists")
        .collect::<Result<Vec<_>, _>>()
        .expect("command result files are readable");
    assert_eq!(result_files.len(), 1);
    let persisted: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(result_files[0].path()).unwrap()).unwrap();
    assert_eq!(persisted, result.command_results[0]);
}

#[tokio::test]
async fn task_update_command_returns_structured_failure_when_task_key_is_missing() {
    let tmp = tempdir().expect("temp workspace");
    let token_path = tmp.path().join("agent_tokens.json");
    std::fs::write(&token_path, r#"{"agent-1":"agent-token-1"}"#).expect("write token file");
    let _env = ChannelTaskEnvGuard::enabled().with_agent_tokens_file(&token_path);
    let captured = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let captured_for_route = captured.clone();
    let app = axum::Router::new()
        .route(
            "/v1/conversations/{conversation_id}/tasks",
            axum::routing::get(
                |axum::extract::Path(conversation_id): axum::extract::Path<String>,
                 headers: axum::http::HeaderMap,
                 axum::extract::State(captured): axum::extract::State<
                    std::sync::Arc<tokio::sync::Mutex<Vec<serde_json::Value>>>,
                >| async move {
                    let auth = headers
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_string);
                    captured.lock().await.push(serde_json::json!({
                        "method": "GET",
                        "conversation_id": conversation_id,
                        "auth": auth,
                    }));
                    axum::Json(serde_json::json!([
                        {
                            "task_id": "task-other",
                            "task_key": "OTHER-1"
                        }
                    ]))
                },
            ),
        )
        .with_state(captured_for_route);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock gateway");
    let gateway_base_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    let maildir_new = tmp.path().join(".choruz-outbox").join("new");
    std::fs::create_dir_all(&maildir_new).expect("maildir new");
    let command = serde_json::json!({
        "type": "task_update",
        "conversation_id": "conv-1",
        "task_key": "BE-12",
        "status": "done",
        "idempotency_key": "turn-up:missing-key"
    });
    std::fs::write(maildir_new.join("task-update.json"), command.to_string())
        .expect("write task_update command");

    let result = super::process_outbox_commands_with_stats(
        "watcher:test",
        "agent-1",
        tmp.path(),
        &gateway_base_url,
        None,
    )
    .await;

    assert_eq!(result.reply, "");
    assert_eq!(result.processed_count, 1);
    assert_eq!(result.command_results.len(), 1);
    assert_envelope_matches(
        &result.command_results[0],
        serde_json::json!({
            "command_type": "task_update",
            "ok": false,
            "error_code": "task_not_found",
            "message": "No task with key BE-12 exists in this conversation.",
            "task_key": "BE-12",
            "task_id": null,
            "idempotency_key": "turn-up:missing-key",
        }),
    );
    let captured = captured.lock().await;
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0]["method"], "GET");
    let result_files = std::fs::read_dir(tmp.path().join(".choruz-outbox/results"))
        .expect("command result directory exists")
        .collect::<Result<Vec<_>, _>>()
        .expect("command result files are readable");
    assert_eq!(result_files.len(), 1);
    let persisted: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(result_files[0].path()).unwrap()).unwrap();
    assert_eq!(persisted, result.command_results[0]);
}

#[tokio::test]
async fn task_update_command_validates_task_id_against_targeted_conversation() {
    let tmp = tempdir().expect("temp workspace");
    let token_path = tmp.path().join("agent_tokens.json");
    std::fs::write(&token_path, r#"{"agent-1":"agent-token-1"}"#).expect("write token file");
    let _env = ChannelTaskEnvGuard::enabled().with_agent_tokens_file(&token_path);
    let captured = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let captured_for_route = captured.clone();
    let app = axum::Router::new()
        .route(
            "/v1/conversations/{conversation_id}/tasks",
            axum::routing::get(
                |axum::extract::Path(conversation_id): axum::extract::Path<String>,
                 headers: axum::http::HeaderMap,
                 axum::extract::State(captured): axum::extract::State<
                    std::sync::Arc<tokio::sync::Mutex<Vec<serde_json::Value>>>,
                >| async move {
                    let auth = headers
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_string);
                    captured.lock().await.push(serde_json::json!({
                        "method": "GET",
                        "conversation_id": conversation_id,
                        "auth": auth,
                    }));
                    axum::Json(serde_json::json!([
                        {
                            "task_id": "task-in-target-conversation",
                            "task_key": "TARGET-1"
                        }
                    ]))
                },
            ),
        )
        .route(
            "/v1/tasks/{task_id}",
            axum::routing::patch(|_path: axum::extract::Path<String>| async move {
                axum::http::StatusCode::INTERNAL_SERVER_ERROR
            }),
        )
        .with_state(captured_for_route);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock gateway");
    let gateway_base_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    let maildir_new = tmp.path().join(".choruz-outbox").join("new");
    std::fs::create_dir_all(&maildir_new).expect("maildir new");
    let command = serde_json::json!({
        "type": "task_update",
        "conversation_id": "conv-1",
        "task_id": "task-in-another-conversation",
        "status": "done",
        "idempotency_key": "turn-up:wrong-conv"
    });
    std::fs::write(maildir_new.join("task-update.json"), command.to_string())
        .expect("write task_update command");

    let result = super::process_outbox_commands_with_stats(
        "watcher:test",
        "agent-1",
        tmp.path(),
        &gateway_base_url,
        None,
    )
    .await;

    assert_eq!(result.reply, "");
    assert_eq!(result.processed_count, 1);
    assert_eq!(result.command_results.len(), 1);
    assert_envelope_matches(
        &result.command_results[0],
        serde_json::json!({
            "command_type": "task_update",
            "ok": false,
            "error_code": "task_not_found",
            "message": "No task with id task-in-another-conversation exists in this conversation.",
            "task_key": null,
            "task_id": "task-in-another-conversation",
            "idempotency_key": "turn-up:wrong-conv",
        }),
    );
    let captured = captured.lock().await;
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0]["method"], "GET");
}

#[tokio::test]
async fn task_update_gateway_failure_includes_resolved_task_id() {
    let tmp = tempdir().expect("temp workspace");
    let token_path = tmp.path().join("agent_tokens.json");
    std::fs::write(&token_path, r#"{"agent-1":"agent-token-1"}"#).expect("write token file");
    let _env = ChannelTaskEnvGuard::enabled().with_agent_tokens_file(&token_path);
    let app = axum::Router::new()
            .route(
                "/v1/conversations/{conversation_id}/tasks",
                axum::routing::get(
                    |_path: axum::extract::Path<String>| async move {
                        axum::Json(serde_json::json!([
                            {
                                "task_id": "task-123",
                                "task_key": "BE-12"
                            }
                        ]))
                    },
                ),
            )
            .route(
                "/v1/tasks/{task_id}",
                axum::routing::patch(
                    |_path: axum::extract::Path<String>| async move {
                        (
                            axum::http::StatusCode::FORBIDDEN,
                            axum::Json(serde_json::json!({
                                "error": {
                                    "detail": "agent can update only owned group tasks or tasks in a coordinated group"
                                }
                            })),
                        )
                    },
                ),
            );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock gateway");
    let gateway_base_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    let maildir_new = tmp.path().join(".choruz-outbox").join("new");
    std::fs::create_dir_all(&maildir_new).expect("maildir new");
    let command = serde_json::json!({
        "type": "task_update",
        "conversation_id": "conv-1",
        "task_key": "BE-12",
        "status": "done",
        "idempotency_key": "turn-up:forbidden"
    });
    std::fs::write(maildir_new.join("task-update.json"), command.to_string())
        .expect("write task_update command");

    let result = super::process_outbox_commands_with_stats(
        "watcher:test",
        "agent-1",
        tmp.path(),
        &gateway_base_url,
        None,
    )
    .await;

    assert_eq!(result.reply, "");
    assert_eq!(result.processed_count, 1);
    assert_eq!(result.command_results.len(), 1);
    assert_envelope_matches(
        &result.command_results[0],
        serde_json::json!({
            "command_type": "task_update",
            "ok": false,
            "error_code": "forbidden",
            "message": "agent can update only owned group tasks or tasks in a coordinated group",
            "task_key": "BE-12",
            "task_id": "task-123",
            "idempotency_key": "turn-up:forbidden",
        }),
    );
}

/// Spin up a mock choruz-api-gateway that always responds with the given status and
/// body to `POST /v1/conversations/{conversation_id}/tasks`, then run one
/// `task_create` command through the outbox. Returns the command results.
///
/// These helpers exercise the production outbox path (token load, payload
/// build, HTTP call, status-to-error-code mapping, result persistence) and
/// assert the structured non-chat command failure envelope the pipeline
/// returns when the gateway rejects a payload. They do NOT verify the
/// gateway-side service validation rules themselves — those are covered by
/// the choruz-api-gateway integration tests for the production service path.
async fn run_task_create_against_gateway_returning(
    status: axum::http::StatusCode,
    body: serde_json::Value,
    command: serde_json::Value,
) -> Vec<serde_json::Value> {
    let tmp = tempdir().expect("temp workspace");
    let token_path = tmp.path().join("agent_tokens.json");
    std::fs::write(&token_path, r#"{"agent-1":"agent-token-1"}"#).expect("write token file");
    let _env = ChannelTaskEnvGuard::enabled().with_agent_tokens_file(&token_path);

    let response: std::sync::Arc<(axum::http::StatusCode, serde_json::Value)> =
        std::sync::Arc::new((status, body));
    let response_for_route = response.clone();
    let app = axum::Router::new()
        .route(
            "/v1/conversations/{conversation_id}/tasks",
            axum::routing::post(
                |axum::extract::State(response): axum::extract::State<
                    std::sync::Arc<(axum::http::StatusCode, serde_json::Value)>,
                >,
                 _path: axum::extract::Path<String>,
                 _headers: axum::http::HeaderMap,
                 axum::Json(_payload): axum::Json<serde_json::Value>| async move {
                    (response.0, axum::Json(response.1.clone()))
                },
            ),
        )
        .with_state(response_for_route);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock gateway");
    let gateway_base_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    let maildir_new = tmp.path().join(".choruz-outbox").join("new");
    std::fs::create_dir_all(&maildir_new).expect("maildir new");
    std::fs::write(maildir_new.join("task-create.json"), command.to_string())
        .expect("write task_create command");

    let result = super::process_outbox_commands_with_stats(
        "watcher:test",
        "agent-1",
        tmp.path(),
        &gateway_base_url,
        None,
    )
    .await;

    assert_eq!(result.reply, "");
    assert_eq!(result.processed_count, 1);
    result.command_results
}

#[tokio::test]
async fn task_create_command_returns_validation_failed_envelope_when_gateway_rejects_required_fields()
 {
    for (label, payload_override, gateway_detail) in [
        (
            "missing_title",
            serde_json::json!({
                "type": "task_create",
                "conversation_id": "conv-1",
                "task_key": "VAL-1",
                "title": " ",
                "idempotency_key": "turn-1:missing-title"
            }),
            "title is required",
        ),
        (
            "meaningless_title",
            serde_json::json!({
                "type": "task_create",
                "conversation_id": "conv-1",
                "task_key": "VAL-2",
                "title": "...",
                "idempotency_key": "turn-1:meaningless-title"
            }),
            "title must include at least one letter or number",
        ),
        (
            "missing_idempotency_key",
            serde_json::json!({
                "type": "task_create",
                "conversation_id": "conv-1",
                "task_key": "VAL-3",
                "title": "Missing idempotency"
            }),
            "idempotency_key is required",
        ),
    ] {
        let command_results = run_task_create_against_gateway_returning(
            axum::http::StatusCode::BAD_REQUEST,
            serde_json::json!({
                "error": {
                    "status": 400,
                    "detail": gateway_detail
                }
            }),
            payload_override.clone(),
        )
        .await;

        assert_eq!(command_results.len(), 1, "{label}");
        let envelope = &command_results[0];
        assert_eq!(envelope["command_type"], "task_create", "{label}");
        assert_eq!(envelope["ok"], false, "{label}");
        assert_eq!(envelope["error_code"], "validation_failed", "{label}");
        assert_eq!(envelope["message"], gateway_detail, "{label}");
        assert_eq!(
            envelope["task_key"],
            payload_override
                .get("task_key")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            "{label}"
        );
        assert!(envelope["task_id"].is_null(), "{label}");
    }
}

#[tokio::test]
async fn task_create_command_returns_idempotency_conflict_envelope_when_gateway_returns_conflict() {
    let command = serde_json::json!({
        "type": "task_create",
        "conversation_id": "conv-1",
        "task_key": "DUP-1",
        "title": "Duplicate idempotency",
        "idempotency_key": "turn-duplicate"
    });
    let command_results = run_task_create_against_gateway_returning(
        axum::http::StatusCode::CONFLICT,
        serde_json::json!({
            "error": {
                "status": 409,
                "detail": "idempotency_key was already used for a different channel task payload"
            }
        }),
        command,
    )
    .await;

    assert_eq!(command_results.len(), 1);
    assert_envelope_matches(
        &command_results[0],
        serde_json::json!({
            "command_type": "task_create",
            "ok": false,
            "error_code": "idempotency_conflict",
            "message": "idempotency_key was already used for a different channel task payload",
            "task_key": "DUP-1",
            "task_id": null,
            "idempotency_key": "turn-duplicate",
        }),
    );
}

#[tokio::test]
async fn task_create_command_rejects_principal_id_assignee_without_runtime_roster() {
    let command = serde_json::json!({
        "type": "task_create",
        "conversation_id": "conv-1",
        "task_key": "VIS-1",
        "title": "Reject unvalidated assignee",
        "assignee_principal_id": "internal-agent-id",
        "idempotency_key": "turn-internal-assignee"
    });
    let command_results = run_task_create_against_gateway_returning(
        axum::http::StatusCode::OK,
        serde_json::json!({
            "task_id": "should-not-reach-gateway",
            "task_key": "VIS-1"
        }),
        command,
    )
    .await;

    assert_eq!(command_results.len(), 1);
    assert_envelope_matches(
        &command_results[0],
        serde_json::json!({
            "command_type": "task_create",
            "ok": false,
            "error_code": "invalid_assignee",
            "message": "Could not resolve task assignee in the agent workspace.",
            "task_key": "VIS-1",
            "task_id": null,
            "idempotency_key": "turn-internal-assignee",
        }),
    );
}

#[tokio::test]
async fn task_create_command_returns_forbidden_envelope_when_gateway_denies_actor() {
    let command = serde_json::json!({
        "type": "task_create",
        "conversation_id": "conv-1",
        "task_key": "AUTHZ-1",
        "title": "Unauthorized agent cannot create",
        "idempotency_key": "turn-unauthorized"
    });
    let command_results = run_task_create_against_gateway_returning(
        axum::http::StatusCode::FORBIDDEN,
        serde_json::json!({
            "error": {
                "status": 403,
                "detail": "actor is not a visible group agent member of this conversation"
            }
        }),
        command,
    )
    .await;

    assert_eq!(command_results.len(), 1);
    assert_envelope_matches(
        &command_results[0],
        serde_json::json!({
            "command_type": "task_create",
            "ok": false,
            "error_code": "forbidden",
            "message": "actor is not a visible group agent member of this conversation",
            "task_key": "AUTHZ-1",
            "task_id": null,
            "idempotency_key": "turn-unauthorized",
        }),
    );
}

#[tokio::test]
async fn task_create_command_returns_invalid_assignee_envelope_when_name_cannot_be_resolved() {
    let Ok(db_url) = std::env::var("CHORUZ_DATABASE_URL") else {
        return;
    };

    let tmp = tempdir().expect("temp workspace");
    let workspace_id = choruz_common::new_id();
    let agent_id = choruz_common::new_id();
    let conversation_id = choruz_common::new_id();
    let group_name = format!("invalid-assignee-group-{}", choruz_common::new_id());
    let token_path = tmp.path().join("agent_tokens.json");
    std::fs::write(
        &token_path,
        serde_json::json!({ agent_id.clone(): "agent-token-invalid-assignee" }).to_string(),
    )
    .expect("write token file");
    let _env = ChannelTaskEnvGuard::enabled().with_agent_tokens_file(&token_path);

    let (client, connection) = tokio_postgres::connect(&db_url, NoTls)
        .await
        .expect("connect for invalid assignee test");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "INSERT INTO principal (id, workspace_id, type, name, disabled, created_at, updated_at)
                 VALUES ($1, $2, 'agent', 'Sender Agent', FALSE, NOW(), NOW())",
            &[&agent_id, &workspace_id],
        )
        .await
        .expect("seed sender agent");
    client
            .execute(
                "INSERT INTO conversation (id, workspace_id, type, name, creator_id, created_at, updated_at)
                 VALUES ($1, $2, 'group', $3, $4, NOW(), NOW())",
                &[&conversation_id, &workspace_id, &group_name, &agent_id],
            )
            .await
            .expect("seed conversation");
    client
        .execute(
            "INSERT INTO conversation_member (conv_id, principal_id, joined_at)
                 VALUES ($1, $2, NOW())",
            &[&conversation_id, &agent_id],
        )
        .await
        .expect("seed membership");

    let store = EventStore::new(&db_url);
    let maildir_new = tmp.path().join(".choruz-outbox").join("new");
    std::fs::create_dir_all(&maildir_new).expect("maildir new");
    let command = serde_json::json!({
        "type": "task_create",
        "group": group_name,
        "task_key": "ASN-1",
        "title": "Cannot resolve assignee name",
        "assignee": format!("ghost-agent-{}", choruz_common::new_id()),
        "idempotency_key": "turn-ghost-assignee"
    });
    std::fs::write(
        maildir_new.join("task-create-invalid-assignee.json"),
        command.to_string(),
    )
    .expect("write task_create command");

    let result = super::process_outbox_commands_with_stats(
        "watcher:test",
        &agent_id,
        tmp.path(),
        "http://127.0.0.1:1",
        Some(&store),
    )
    .await;

    assert_eq!(result.reply, "");
    assert_eq!(result.processed_count, 1);
    let envelope = &result.command_results[0];
    assert_eq!(envelope["command_type"], "task_create");
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["error_code"], "invalid_assignee");
    assert_eq!(envelope["task_key"], "ASN-1");
    assert!(envelope["task_id"].is_null());

    let chat_event_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
                 FROM conversation_events
                 WHERE conversation_id = $1",
            &[&conversation_id],
        )
        .await
        .expect("count chat events")
        .get::<_, i64>("count");
    assert_eq!(
        chat_event_count, 0,
        "failed task command must not produce any chat events"
    );
}

#[tokio::test]
async fn task_create_command_does_not_probe_roster_for_direct_conversation_non_member() {
    let Ok(db_url) = std::env::var("CHORUZ_DATABASE_URL") else {
        return;
    };

    let tmp = tempdir().expect("temp workspace");
    let sender_workspace_id = choruz_common::new_id();
    let target_workspace_id = choruz_common::new_id();
    let agent_id = choruz_common::new_id();
    let target_agent_id = choruz_common::new_id();
    let conversation_id = choruz_common::new_id();
    let token_path = tmp.path().join("agent_tokens.json");
    std::fs::write(
        &token_path,
        serde_json::json!({ agent_id.clone(): "agent-token-direct-probe" }).to_string(),
    )
    .expect("write token file");
    let _env = ChannelTaskEnvGuard::enabled().with_agent_tokens_file(&token_path);

    let (client, connection) = tokio_postgres::connect(&db_url, NoTls)
        .await
        .expect("connect for direct conversation roster probe test");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
            .execute(
                "INSERT INTO principal (id, workspace_id, type, name, disabled, channel_visibility, created_at, updated_at)
                 VALUES
                   ($1, $2, 'agent', 'Sender Agent', FALSE, 'visible', NOW(), NOW()),
                   ($3, $4, 'agent', 'Other Workspace Agent', FALSE, 'visible', NOW(), NOW())",
                &[&agent_id, &sender_workspace_id, &target_agent_id, &target_workspace_id],
            )
            .await
            .expect("seed principals");
    client
            .execute(
                "INSERT INTO conversation (id, workspace_id, type, name, creator_id, created_at, updated_at)
                 VALUES ($1, $2, 'group', 'foreign-group', $3, NOW(), NOW())",
                &[&conversation_id, &target_workspace_id, &target_agent_id],
            )
            .await
            .expect("seed target conversation");
    client
        .execute(
            "INSERT INTO conversation_member (conv_id, principal_id, joined_at)
                 VALUES ($1, $2, NOW())",
            &[&conversation_id, &target_agent_id],
        )
        .await
        .expect("seed target member");

    let store = EventStore::new(&db_url);
    let maildir_new = tmp.path().join(".choruz-outbox").join("new");
    std::fs::create_dir_all(&maildir_new).expect("maildir new");
    let command = serde_json::json!({
        "type": "task_create",
        "conversation_id": conversation_id,
        "task_key": "PROBE-1",
        "title": "Do not probe another conversation roster",
        "assignee": "Other Workspace Agent",
        "idempotency_key": "turn-direct-probe"
    });
    std::fs::write(
        maildir_new.join("task-create-direct-probe.json"),
        command.to_string(),
    )
    .expect("write task_create command");

    let result = super::process_outbox_commands_with_stats(
        "watcher:test",
        &agent_id,
        tmp.path(),
        "http://127.0.0.1:1",
        Some(&store),
    )
    .await;

    assert_eq!(result.reply, "");
    assert_eq!(result.processed_count, 1);
    let envelope = &result.command_results[0];
    assert_eq!(envelope["command_type"], "task_create");
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["error_code"], "invalid_assignee");
    assert_eq!(envelope["task_key"], "PROBE-1");
    assert!(envelope["task_id"].is_null());
}

#[tokio::test]
async fn task_create_command_missing_target_returns_structured_failure_without_db() {
    let tmp = tempdir().expect("temp workspace");
    let _env = ChannelTaskEnvGuard::enabled();
    let maildir_new = tmp.path().join(".choruz-outbox").join("new");
    std::fs::create_dir_all(&maildir_new).expect("maildir new");
    let command = serde_json::json!({
        "type": "task_create",
        "task_key": "NO-TARGET-1",
        "title": "Missing conversation target",
        "idempotency_key": "turn-no-target"
    });
    std::fs::write(maildir_new.join("missing-target.json"), command.to_string())
        .expect("write task_create command");

    let result = super::process_outbox_commands_with_stats(
        "watcher:test",
        "agent-1",
        tmp.path(),
        "http://127.0.0.1:1",
        None,
    )
    .await;

    assert_eq!(result.reply, "");
    assert_eq!(result.processed_count, 1);
    assert_envelope_matches(
        &result.command_results[0],
        serde_json::json!({
            "command_type": "task_create",
            "ok": false,
            "error_code": "missing_target",
            "message": "Channel task commands require group or conversation_id.",
            "task_key": "NO-TARGET-1",
            "task_id": null,
            "idempotency_key": "turn-no-target",
        }),
    );
}

#[tokio::test]
async fn task_update_and_transfer_return_missing_task_envelope_when_identifier_omitted() {
    struct Case {
        command_type: &'static str,
        variant: &'static str,
        command: serde_json::Value,
        expected_task_id: serde_json::Value,
        expected_task_key: serde_json::Value,
        expected_idempotency_key: serde_json::Value,
    }
    let cases = [
        Case {
            command_type: "task_update",
            variant: "omitted",
            command: serde_json::json!({
                "type": "task_update",
                "conversation_id": "conv-1",
                "idempotency_key": "turn-mt:update-omitted"
            }),
            expected_task_id: serde_json::Value::Null,
            expected_task_key: serde_json::Value::Null,
            expected_idempotency_key: serde_json::Value::String("turn-mt:update-omitted".into()),
        },
        Case {
            command_type: "task_transfer",
            variant: "omitted",
            command: serde_json::json!({
                "type": "task_transfer",
                "conversation_id": "conv-1",
                "idempotency_key": "turn-mt:transfer-omitted"
            }),
            expected_task_id: serde_json::Value::Null,
            expected_task_key: serde_json::Value::Null,
            expected_idempotency_key: serde_json::Value::String("turn-mt:transfer-omitted".into()),
        },
        // Blank-string identifiers must take the same `missing_task` branch as
        // omitted ones (`resolve_channel_task_ref` trims+filters), even though the
        // envelope reflects the operator-supplied raw strings.
        Case {
            command_type: "task_update",
            variant: "blank-strings",
            command: serde_json::json!({
                "type": "task_update",
                "conversation_id": "conv-1",
                "task_id": "",
                "task_key": "   ",
                "idempotency_key": "turn-mt:update-blank"
            }),
            expected_task_id: serde_json::Value::String(String::new()),
            expected_task_key: serde_json::Value::String("   ".into()),
            expected_idempotency_key: serde_json::Value::String("turn-mt:update-blank".into()),
        },
        Case {
            command_type: "task_transfer",
            variant: "blank-strings",
            command: serde_json::json!({
                "type": "task_transfer",
                "conversation_id": "conv-1",
                "task_id": "",
                "task_key": "   ",
                "idempotency_key": "turn-mt:transfer-blank"
            }),
            expected_task_id: serde_json::Value::String(String::new()),
            expected_task_key: serde_json::Value::String("   ".into()),
            expected_idempotency_key: serde_json::Value::String("turn-mt:transfer-blank".into()),
        },
    ];

    for case in &cases {
        let tmp = tempdir().expect("temp workspace");
        let token_path = tmp.path().join("agent_tokens.json");
        std::fs::write(&token_path, r#"{"agent-1":"agent-token-1"}"#).expect("write token file");
        let _env = ChannelTaskEnvGuard::enabled().with_agent_tokens_file(&token_path);

        let app = axum::Router::new()
            .route(
                "/v1/conversations/{conversation_id}/tasks",
                axum::routing::get(|_path: axum::extract::Path<String>| async move {
                    panic!("missing task identifier must fail before issuing a task lookup");
                    #[allow(unreachable_code)]
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR
                }),
            )
            .route(
                "/v1/tasks/{task_id}",
                axum::routing::patch(|_path: axum::extract::Path<String>| async move {
                    panic!("missing task identifier must fail before issuing a PATCH mutation");
                    #[allow(unreachable_code)]
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock gateway");
        let gateway_base_url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });

        let maildir_new = tmp.path().join(".choruz-outbox").join("new");
        std::fs::create_dir_all(&maildir_new).expect("maildir new");
        std::fs::write(
            maildir_new.join(format!("{}-{}.json", case.command_type, case.variant)),
            case.command.to_string(),
        )
        .expect("write command without identifier");

        let result = super::process_outbox_commands_with_stats(
            "watcher:test",
            "agent-1",
            tmp.path(),
            &gateway_base_url,
            None,
        )
        .await;

        assert_eq!(result.reply, "");
        assert_eq!(result.processed_count, 1);
        assert_envelope_matches(
            &result.command_results[0],
            serde_json::json!({
                "command_type": case.command_type,
                "ok": false,
                "error_code": "missing_task",
                "message": "task_update and task_transfer require task_id or task_key.",
                "task_key": case.expected_task_key,
                "task_id": case.expected_task_id,
                "idempotency_key": case.expected_idempotency_key,
            }),
        );
    }
}

#[tokio::test]
async fn task_transfer_command_returns_missing_assignee_envelope_when_assignee_omitted() {
    let tmp = tempdir().expect("temp workspace");
    let token_path = tmp.path().join("agent_tokens.json");
    std::fs::write(&token_path, r#"{"agent-1":"agent-token-1"}"#).expect("write token file");
    let _env = ChannelTaskEnvGuard::enabled().with_agent_tokens_file(&token_path);
    let app = axum::Router::new()
        .route(
            "/v1/conversations/{conversation_id}/tasks",
            axum::routing::get(|_path: axum::extract::Path<String>| async move {
                axum::Json(serde_json::json!([
                    {
                        "task_id": "task-transfer-noassignee",
                        "task_key": "TR-1"
                    }
                ]))
            }),
        )
        .route(
            "/v1/tasks/{task_id}",
            axum::routing::patch(|_path: axum::extract::Path<String>| async move {
                panic!(
                    "task_transfer without an assignee must fail before issuing a PATCH mutation"
                );
                #[allow(unreachable_code)]
                axum::http::StatusCode::INTERNAL_SERVER_ERROR
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock gateway");
    let gateway_base_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    let maildir_new = tmp.path().join(".choruz-outbox").join("new");
    std::fs::create_dir_all(&maildir_new).expect("maildir new");
    let command = serde_json::json!({
        "type": "task_transfer",
        "conversation_id": "conv-1",
        "task_key": "TR-1",
        "idempotency_key": "turn-ma:transfer"
    });
    std::fs::write(maildir_new.join("task-transfer.json"), command.to_string())
        .expect("write task_transfer command");

    let result = super::process_outbox_commands_with_stats(
        "watcher:test",
        "agent-1",
        tmp.path(),
        &gateway_base_url,
        None,
    )
    .await;

    assert_eq!(result.reply, "");
    assert_eq!(result.processed_count, 1);
    assert_envelope_matches(
        &result.command_results[0],
        serde_json::json!({
            "command_type": "task_transfer",
            "ok": false,
            "error_code": "missing_assignee",
            "message": "task_transfer requires assignee or assignee_principal_id.",
            "task_key": "TR-1",
            "task_id": "task-transfer-noassignee",
            "idempotency_key": "turn-ma:transfer",
        }),
    );
}

#[tokio::test]
async fn task_transfer_command_resolves_group_and_assignee_name_without_chat_event() {
    let Ok(db_url) = std::env::var("CHORUZ_DATABASE_URL") else {
        return;
    };

    let tmp = tempdir().expect("temp workspace");
    let workspace_id = choruz_common::new_id();
    let agent_id = choruz_common::new_id();
    let assignee_id = choruz_common::new_id();
    let conversation_id = choruz_common::new_id();
    let group_name = format!("task-transfer-group-{}", choruz_common::new_id());
    let assignee_name = format!("Transfer Agent {}", choruz_common::new_id());
    let token_path = tmp.path().join("agent_tokens.json");
    std::fs::write(
        &token_path,
        serde_json::json!({ agent_id.clone(): "agent-token-transfer" }).to_string(),
    )
    .expect("write token file");
    let _env = ChannelTaskEnvGuard::enabled().with_agent_tokens_file(&token_path);

    let (client, connection) = tokio_postgres::connect(&db_url, NoTls)
        .await
        .expect("connect for task_transfer group test");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "INSERT INTO principal (id, workspace_id, type, name, disabled, created_at, updated_at)
                 VALUES
                   ($1, $2, 'agent', 'Sender Agent', FALSE, NOW(), NOW()),
                   ($3, $2, 'agent', $4, FALSE, NOW(), NOW())",
            &[&agent_id, &workspace_id, &assignee_id, &assignee_name],
        )
        .await
        .expect("seed principals");
    client
            .execute(
                "INSERT INTO conversation (id, workspace_id, type, name, creator_id, created_at, updated_at)
                 VALUES ($1, $2, 'group', $3, $4, NOW(), NOW())",
                &[&conversation_id, &workspace_id, &group_name, &agent_id],
            )
            .await
            .expect("seed group conversation");
    client
        .execute(
            "INSERT INTO conversation_member (conv_id, principal_id, joined_at)
                 VALUES ($1, $2, NOW()), ($1, $3, NOW())",
            &[&conversation_id, &agent_id, &assignee_id],
        )
        .await
        .expect("seed group members");

    let captured = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let captured_for_route = captured.clone();
    let app = axum::Router::new()
        .route(
            "/v1/conversations/{conversation_id}/tasks",
            axum::routing::get(
                |axum::extract::Path(conversation_id): axum::extract::Path<String>,
                 headers: axum::http::HeaderMap,
                 axum::extract::State(captured): axum::extract::State<
                    std::sync::Arc<tokio::sync::Mutex<Vec<serde_json::Value>>>,
                >| async move {
                    let auth = headers
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_string);
                    captured.lock().await.push(serde_json::json!({
                        "method": "GET",
                        "conversation_id": conversation_id,
                        "auth": auth,
                    }));
                    axum::Json(serde_json::json!([
                        {
                            "task_id": "task-transfer-1",
                            "task_key": "GRP-2"
                        }
                    ]))
                },
            ),
        )
        .route(
            "/v1/tasks/{task_id}",
            axum::routing::patch(
                |axum::extract::Path(task_id): axum::extract::Path<String>,
                 headers: axum::http::HeaderMap,
                 axum::extract::State(captured): axum::extract::State<
                    std::sync::Arc<tokio::sync::Mutex<Vec<serde_json::Value>>>,
                >,
                 axum::Json(payload): axum::Json<serde_json::Value>| async move {
                    let auth = headers
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_string);
                    captured.lock().await.push(serde_json::json!({
                        "method": "PATCH",
                        "task_id": task_id,
                        "auth": auth,
                        "payload": payload,
                    }));
                    axum::Json(serde_json::json!({
                        "task_id": "task-transfer-1",
                        "task_key": "GRP-2"
                    }))
                },
            ),
        )
        .with_state(captured_for_route);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock gateway");
    let gateway_base_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    let store = EventStore::new(&db_url);
    let maildir_new = tmp.path().join(".choruz-outbox").join("new");
    std::fs::create_dir_all(&maildir_new).expect("maildir new");
    let command = serde_json::json!({
        "type": "task_transfer",
        "group": group_name,
        "task_key": "GRP-2",
        "assignee": assignee_name
    });
    std::fs::write(
        maildir_new.join("task-transfer-group.json"),
        command.to_string(),
    )
    .expect("write group task_transfer command");

    let result = super::process_outbox_commands_with_stats(
        "watcher:test",
        &agent_id,
        tmp.path(),
        &gateway_base_url,
        Some(&store),
    )
    .await;

    assert_eq!(result.reply, "");
    assert_eq!(result.processed_count, 1);
    assert_eq!(result.command_results.len(), 1);
    assert_envelope_matches(
        &result.command_results[0],
        serde_json::json!({
            "command_type": "task_transfer",
            "ok": true,
            "task_key": "GRP-2",
            "task_id": "task-transfer-1",
            "idempotency_key": null,
        }),
    );
    let captured = captured.lock().await;
    assert_eq!(captured.len(), 2);
    assert_eq!(captured[0]["conversation_id"], conversation_id);
    assert_eq!(captured[0]["auth"], "Bearer agent-token-transfer");
    assert_eq!(
        captured[1],
        serde_json::json!({
            "method": "PATCH",
            "task_id": "task-transfer-1",
            "auth": "Bearer agent-token-transfer",
            "payload": {
                "assignee_principal_id": assignee_id
            }
        })
    );
    let chat_event_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
                 FROM conversation_events
                 WHERE conversation_id = $1",
            &[&conversation_id],
        )
        .await
        .expect("count chat events")
        .get::<_, i64>("count");
    assert_eq!(chat_event_count, 0);
}

#[test]
fn provision_agent_payload_defaults_durable_teammates_to_visible() {
    let payload = super::provision_agent_payload(
        "Research Lead",
        "codex_terminal",
        "Lead the shared research methodology.",
        Some("workspace-1"),
        false,
        Some("gpt-5.6-codex"),
    );

    assert_eq!(payload["name"].as_str(), Some("Research Lead"));
    assert_eq!(payload["driver_type"].as_str(), Some("codex_terminal"));
    assert_eq!(payload["workspace_id"].as_str(), Some("workspace-1"));
    assert_eq!(payload["model"].as_str(), Some("gpt-5.6-codex"));
    assert!(payload.get("channel_visibility").is_none());
}

#[test]
fn provision_agent_payload_can_request_private_internal_helper() {
    let payload = super::provision_agent_payload(
        "Private Helper",
        "codex_terminal",
        "Help with private local planning.",
        Some("workspace-1"),
        true,
        None,
    );

    assert_eq!(payload["channel_visibility"].as_str(), Some("internal"));
}

#[test]
fn provision_agent_visibility_accepts_only_absent_or_known_strings() {
    assert_eq!(
        super::provision_agent_channel_visibility(&serde_json::json!({})),
        Ok(None)
    );
    for visibility in ["visible", "internal"] {
        let command = serde_json::json!({"channel_visibility": visibility});
        assert_eq!(
            super::provision_agent_channel_visibility(&command),
            Ok(Some(visibility))
        );
    }
    for invalid in [serde_json::json!(true), serde_json::json!(7)] {
        let command = serde_json::json!({"channel_visibility": invalid});
        assert_eq!(
            super::provision_agent_channel_visibility(&command),
            Err("channel_visibility must be 'visible' or 'internal'.")
        );
    }
}

#[test]
fn internal_provision_token_requires_dedicated_env() {
    let saved = std::env::var("CHORUZ_INTERNAL_PROVISION_TOKEN").ok();
    unsafe {
        std::env::remove_var("CHORUZ_INTERNAL_PROVISION_TOKEN");
    }
    assert_eq!(super::internal_provision_token(), "");
    unsafe {
        match saved {
            Some(value) => std::env::set_var("CHORUZ_INTERNAL_PROVISION_TOKEN", value),
            None => std::env::remove_var("CHORUZ_INTERNAL_PROVISION_TOKEN"),
        }
    }
}

#[tokio::test]
async fn process_outbox_commands_delivers_group_send_to_named_group() {
    let Ok(db_url) = std::env::var("CHORUZ_DATABASE_URL") else {
        return;
    };

    let workspace_id = choruz_common::new_id();
    let agent_id = choruz_common::new_id();
    let human_id = choruz_common::new_id();
    let conv_id = choruz_common::new_id();
    let group_name = format!("release-outbox-group-{}", choruz_common::new_id());
    let content = format!("release outbox group send {}", choruz_common::new_id());

    let (client, connection) = tokio_postgres::connect(&db_url, NoTls)
        .await
        .expect("connect for group outbox send test");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "INSERT INTO principal (id, workspace_id, type, name, disabled, created_at, updated_at)
                 VALUES ($1, $2, 'agent', 'Outbox Sender', FALSE, NOW(), NOW()),
                        ($3, $2, 'human', 'Outbox Viewer', FALSE, NOW(), NOW())",
            &[&agent_id, &workspace_id, &human_id],
        )
        .await
        .expect("seed outbox principals");
    client
            .execute(
                "INSERT INTO conversation (id, workspace_id, type, name, creator_id, created_at, updated_at)
                 VALUES ($1, $2, 'group', $3, $4, NOW(), NOW())",
                &[&conv_id, &workspace_id, &group_name, &human_id],
            )
            .await
            .expect("seed outbox group");
    client
        .execute(
            "INSERT INTO conversation_member (conv_id, principal_id, joined_at)
                 VALUES ($1, $2, NOW()),
                        ($1, $3, NOW())",
            &[&conv_id, &agent_id, &human_id],
        )
        .await
        .expect("seed outbox memberships");

    let tmp = tempdir().expect("temp workspace");
    let maildir_new = tmp.path().join(".choruz-outbox").join("new");
    std::fs::create_dir_all(&maildir_new).expect("maildir new");
    let command = serde_json::json!({
        "type": "send",
        "group": group_name,
        "content": content,
    });
    std::fs::write(maildir_new.join("cmd.json"), command.to_string())
        .expect("write group send command");

    let store = EventStore::new(&db_url);
    let reply = super::process_outbox_commands(
        "watcher:test",
        &agent_id,
        tmp.path(),
        "http://127.0.0.1:3000",
        Some(&store),
    )
    .await;

    assert_eq!(reply, "");
    assert!(
        std::fs::read_dir(&maildir_new)
            .expect("maildir readable")
            .next()
            .is_none(),
        "group send command should be removed after delivery"
    );

    let event = client
        .query_one(
            "SELECT event_id, sender_id, content, content_type, metadata
                 FROM conversation_events
                 WHERE conversation_id = $1 AND content = $2",
            &[&conv_id, &Some(content.as_str())],
        )
        .await
        .expect("group message exists");
    let message_id: String = event.get("event_id");
    assert_eq!(event.get::<_, String>("sender_id"), agent_id);
    assert_eq!(
        event.get::<_, Option<String>>("content").as_deref(),
        Some(content.as_str())
    );
    assert_eq!(event.get::<_, String>("content_type"), "text/plain");
    assert_eq!(
        event.get::<_, serde_json::Value>("metadata"),
        serde_json::json!({})
    );

    let outbox = client
        .query_one(
            "SELECT payload->>'message_id' AS message_id,
                        payload->'metadata' AS metadata,
                        published
                 FROM event_outbox
                 WHERE aggregate_id = $1 AND payload->>'content' = $2",
            &[&conv_id, &content],
        )
        .await
        .expect("event outbox row exists");
    assert_eq!(
        outbox.get::<_, Option<String>>("message_id").as_deref(),
        Some(message_id.as_str())
    );
    assert_eq!(
        outbox.get::<_, serde_json::Value>("metadata"),
        serde_json::json!({})
    );
    assert!(!outbox.get::<_, bool>("published"));

    let visible_member = client
        .query_one(
            "SELECT COUNT(*)::BIGINT
                 FROM conversation_member
                 WHERE conv_id = $1 AND principal_id = $2 AND removed_at IS NULL",
            &[&conv_id, &human_id],
        )
        .await
        .expect("count human membership");
    assert_eq!(visible_member.get::<_, i64>(0), 1);
}

#[tokio::test]
async fn process_outbox_commands_preserves_group_send_metadata() {
    let Ok(db_url) = std::env::var("CHORUZ_DATABASE_URL") else {
        return;
    };

    let workspace_id = choruz_common::new_id();
    let agent_id = choruz_common::new_id();
    let conv_id = choruz_common::new_id();
    let task_id = choruz_common::new_id();
    let group_name = format!("workflow-outbox-group-{}", choruz_common::new_id());
    let content = format!("workflow outbox group send {}", choruz_common::new_id());
    let workflow_metadata = serde_json::json!({
        "workflow": {
            "kind": "task.feedback",
            "task_key": "DOC-P0-03",
            "task_id": task_id.clone(),
        },
        "trace": {
            "source": "outbox-test",
            "sequence": 1,
        },
    });

    let (client, connection) = tokio_postgres::connect(&db_url, NoTls)
        .await
        .expect("connect for group outbox metadata test");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "INSERT INTO principal (id, workspace_id, type, name, disabled, created_at, updated_at)
                 VALUES ($1, $2, 'agent', 'Workflow Outbox Sender', FALSE, NOW(), NOW())",
            &[&agent_id, &workspace_id],
        )
        .await
        .expect("seed outbox sender");
    client
            .execute(
                "INSERT INTO conversation (id, workspace_id, type, name, creator_id, created_at, updated_at)
                 VALUES ($1, $2, 'group', $3, $4, NOW(), NOW())",
                &[&conv_id, &workspace_id, &group_name, &agent_id],
            )
            .await
            .expect("seed outbox group");
    client
        .execute(
            "INSERT INTO conversation_member (conv_id, principal_id, joined_at)
                 VALUES ($1, $2, NOW())",
            &[&conv_id, &agent_id],
        )
        .await
        .expect("seed outbox membership");
    client
        .execute(
            "INSERT INTO group_workflow_task
                    (id, conversation_id, task_key, title, status, assignee_principal_id,
                     source_kind, created_by, version)
                 VALUES
                    ($1, $2, 'DOC-P0-03', 'Draft workflow doc', 'todo', $3,
                     'agent', $3, 1)",
            &[&task_id, &conv_id, &agent_id],
        )
        .await
        .expect("seed workflow task");

    let tmp = tempdir().expect("temp workspace");
    let maildir_new = tmp.path().join(".choruz-outbox").join("new");
    std::fs::create_dir_all(&maildir_new).expect("maildir new");
    let command = serde_json::json!({
        "type": "send",
        "group": group_name,
        "content": content,
        "metadata": workflow_metadata.clone(),
    });
    std::fs::write(maildir_new.join("cmd.json"), command.to_string())
        .expect("write group send command");

    let store = EventStore::new(&db_url);
    let reply = super::process_outbox_commands(
        "watcher:test",
        &agent_id,
        tmp.path(),
        "http://127.0.0.1:3000",
        Some(&store),
    )
    .await;

    assert_eq!(reply, "");

    let event = client
        .query_one(
            "SELECT event_id, metadata
                 FROM conversation_events
                 WHERE conversation_id = $1 AND content = $2",
            &[&conv_id, &Some(content.as_str())],
        )
        .await
        .expect("group message exists");
    let message_id: String = event.get("event_id");
    assert_eq!(
        event.get::<_, serde_json::Value>("metadata"),
        workflow_metadata
    );

    let outbox = client
        .query_one(
            "SELECT payload->'metadata' AS metadata
                 FROM event_outbox
                 WHERE aggregate_id = $1 AND payload->>'message_id' = $2",
            &[&conv_id, &message_id],
        )
        .await
        .expect("event outbox row exists");
    assert_eq!(
        outbox.get::<_, serde_json::Value>("metadata"),
        workflow_metadata
    );

    let task = client
        .query_one(
            "SELECT status, version
                 FROM group_workflow_task
                 WHERE id = $1",
            &[&task_id],
        )
        .await
        .expect("workflow task updated");
    assert_eq!(task.get::<_, String>("status"), "in_progress");
    assert_eq!(task.get::<_, i64>("version"), 2);

    let workflow_event = client
        .query_one(
            "SELECT task_id, source_message_id, actor_principal_id, kind, resulting_version
                 FROM group_workflow_event
                 WHERE task_id = $1",
            &[&task_id],
        )
        .await
        .expect("workflow event appended");
    assert_eq!(
        workflow_event
            .get::<_, Option<String>>("task_id")
            .as_deref(),
        Some(task_id.as_str())
    );
    assert_eq!(
        workflow_event
            .get::<_, Option<String>>("source_message_id")
            .as_deref(),
        Some(message_id.as_str())
    );
    assert_eq!(
        workflow_event
            .get::<_, Option<String>>("actor_principal_id")
            .as_deref(),
        Some(agent_id.as_str())
    );
    assert_eq!(workflow_event.get::<_, String>("kind"), "task.feedback");
    assert_eq!(
        workflow_event.get::<_, Option<i64>>("resulting_version"),
        Some(2)
    );

    let passed_content = format!("workflow outbox passed check {}", choruz_common::new_id());
    let passed_metadata = serde_json::json!({
        "workflow": {
            "kind": "external_check.passed",
            "task_key": "DOC-P0-03",
            "task_id": task_id.clone(),
        },
        "trace": {
            "source": "outbox-test",
            "sequence": 2,
        },
    });
    let passed_command = serde_json::json!({
        "type": "send",
        "group": group_name,
        "content": passed_content,
        "metadata": passed_metadata,
    });
    std::fs::write(
        maildir_new.join("cmd-passed.json"),
        passed_command.to_string(),
    )
    .expect("write passed check command");

    let passed_reply = super::process_outbox_commands(
        "watcher:test",
        &agent_id,
        tmp.path(),
        "http://127.0.0.1:3000",
        Some(&store),
    )
    .await;

    assert_eq!(passed_reply, "");
    let passed_message = client
        .query_one(
            "SELECT event_id
                 FROM conversation_events
                 WHERE conversation_id = $1 AND content = $2",
            &[&conv_id, &Some(passed_content.as_str())],
        )
        .await
        .expect("passed check group message exists");
    let passed_message_id: String = passed_message.get("event_id");

    let task_after_passed = client
        .query_one(
            "SELECT status, version
                 FROM group_workflow_task
                 WHERE id = $1",
            &[&task_id],
        )
        .await
        .expect("workflow task unchanged after passed check");
    assert_eq!(task_after_passed.get::<_, String>("status"), "in_progress");
    assert_eq!(task_after_passed.get::<_, i64>("version"), 2);

    let passed_workflow_event = client
        .query_one(
            "SELECT source_message_id, actor_principal_id, kind, payload, resulting_version
                 FROM group_workflow_event
                 WHERE task_id = $1 AND kind = 'external_check.passed'",
            &[&task_id],
        )
        .await
        .expect("passed check workflow event appended");
    assert_eq!(
        passed_workflow_event
            .get::<_, Option<String>>("source_message_id")
            .as_deref(),
        Some(passed_message_id.as_str())
    );
    assert_eq!(
        passed_workflow_event
            .get::<_, Option<String>>("actor_principal_id")
            .as_deref(),
        Some(agent_id.as_str())
    );
    assert_eq!(
        passed_workflow_event.get::<_, Option<i64>>("resulting_version"),
        None
    );
    assert_eq!(
        passed_workflow_event.get::<_, serde_json::Value>("payload")["workflow_diagnostic"]["reason_code"],
        "workflow_status_noop"
    );
}

#[tokio::test]
async fn send_to_group_resolves_name_within_agent_workspace() {
    let Ok(db_url) = std::env::var("CHORUZ_DATABASE_URL") else {
        return;
    };

    let workspace_a = choruz_common::new_id();
    let workspace_b = choruz_common::new_id();
    let agent_a = choruz_common::new_id();
    let agent_b = choruz_common::new_id();
    let conv_a = choruz_common::new_id();
    let conv_b = choruz_common::new_id();
    let group_name = "shared-team-name";
    let content = format!("workspace scoped send {}", choruz_common::new_id());

    let (client, connection) = tokio_postgres::connect(&db_url, NoTls)
        .await
        .expect("connect for workspace group send test");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "INSERT INTO principal (id, workspace_id, type, name, disabled, created_at, updated_at)
                 VALUES ($1, $2, 'agent', 'Agent A', FALSE, NOW(), NOW()),
                        ($3, $4, 'agent', 'Agent B', FALSE, NOW(), NOW())",
            &[&agent_a, &workspace_a, &agent_b, &workspace_b],
        )
        .await
        .expect("seed agents");
    client
            .execute(
                "INSERT INTO conversation (id, workspace_id, type, name, creator_id, created_at, updated_at)
                 VALUES ($1, $2, 'group', $3, $4, NOW(), NOW()),
                        ($5, $6, 'group', $3, $7, NOW(), NOW())",
                &[
                    &conv_a,
                    &workspace_a,
                    &group_name,
                    &agent_a,
                    &conv_b,
                    &workspace_b,
                    &agent_b,
                ],
            )
            .await
            .expect("seed same-name groups");
    client
        .execute(
            "INSERT INTO conversation_member (conv_id, principal_id, joined_at)
                 VALUES ($1, $2, NOW()),
                        ($3, $4, NOW())",
            &[&conv_a, &agent_a, &conv_b, &agent_b],
        )
        .await
        .expect("seed memberships");

    let store = EventStore::new(&db_url);
    send_to_group(
        "watcher:test",
        &agent_a,
        &store,
        group_name,
        &content,
        "text/plain",
        serde_json::json!({}),
    )
    .await
    .expect("send should succeed");

    let row = client
        .query_one(
            "SELECT conversation_id FROM conversation_events WHERE content = $1",
            &[&Some(content.as_str())],
        )
        .await
        .expect("sent message exists");
    assert_eq!(row.get::<_, String>("conversation_id"), conv_a);
}

#[tokio::test]
async fn send_to_group_rejects_non_member_agent() {
    let Ok(db_url) = std::env::var("CHORUZ_DATABASE_URL") else {
        return;
    };

    let workspace_id = choruz_common::new_id();
    let agent_id = choruz_common::new_id();
    let conv_id = choruz_common::new_id();
    let group_name = "private-team";

    let (client, connection) = tokio_postgres::connect(&db_url, NoTls)
        .await
        .expect("connect for non-member group send test");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "INSERT INTO principal (id, workspace_id, type, name, disabled, created_at, updated_at)
                 VALUES ($1, $2, 'agent', 'Agent A', FALSE, NOW(), NOW())",
            &[&agent_id, &workspace_id],
        )
        .await
        .expect("seed agent");
    client
            .execute(
                "INSERT INTO conversation (id, workspace_id, type, name, creator_id, created_at, updated_at)
                 VALUES ($1, $2, 'group', $3, $4, NOW(), NOW())",
                &[&conv_id, &workspace_id, &group_name, &agent_id],
            )
            .await
            .expect("seed group without agent membership");

    let store = EventStore::new(&db_url);
    let err = send_to_group(
        "watcher:test",
        &agent_id,
        &store,
        group_name,
        "hello",
        "text/plain",
        serde_json::json!({}),
    )
    .await
    .expect_err("non-member send should fail");

    assert!(err.contains("not found in this workspace"));
    assert_eq!(
        client
            .query_one(
                "SELECT COUNT(*)::BIGINT FROM conversation_events WHERE conversation_id = $1",
                &[&conv_id],
            )
            .await
            .expect("count events")
            .get::<_, i64>(0),
        0
    );
}

#[tokio::test]
async fn send_to_group_rejects_ambiguous_same_workspace_name() {
    let Ok(db_url) = std::env::var("CHORUZ_DATABASE_URL") else {
        return;
    };

    let workspace_id = choruz_common::new_id();
    let agent_id = choruz_common::new_id();
    let conv_a = choruz_common::new_id();
    let conv_b = choruz_common::new_id();
    let group_name = "duplicated-team";

    let (client, connection) = tokio_postgres::connect(&db_url, NoTls)
        .await
        .expect("connect for ambiguous group send test");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "INSERT INTO principal (id, workspace_id, type, name, disabled, created_at, updated_at)
                 VALUES ($1, $2, 'agent', 'Agent A', FALSE, NOW(), NOW())",
            &[&agent_id, &workspace_id],
        )
        .await
        .expect("seed agent");
    client
            .execute(
                "INSERT INTO conversation (id, workspace_id, type, name, creator_id, created_at, updated_at)
                 VALUES ($1, $2, 'group', $3, $4, NOW(), NOW()),
                        ($5, $2, 'group', $3, $4, NOW(), NOW())",
                &[&conv_a, &workspace_id, &group_name, &agent_id, &conv_b],
            )
            .await
            .expect("seed duplicate groups");
    client
        .execute(
            "INSERT INTO conversation_member (conv_id, principal_id, joined_at)
                 VALUES ($1, $2, NOW()),
                        ($3, $2, NOW())",
            &[&conv_a, &agent_id, &conv_b],
        )
        .await
        .expect("seed memberships");

    let store = EventStore::new(&db_url);
    let err = send_to_group(
        "watcher:test",
        &agent_id,
        &store,
        group_name,
        "hello",
        "text/plain",
        serde_json::json!({}),
    )
    .await
    .expect_err("ambiguous group send should fail");

    assert!(err.contains("ambiguous"));
}

#[tokio::test]
async fn watcher_session_key_set_cron_uses_binding_conversation_id() {
    let Ok(db_url) = std::env::var("CHORUZ_DATABASE_URL") else {
        return;
    };

    let workspace_id = choruz_common::new_id();
    let agent_id = choruz_common::new_id();
    let conversation_id = choruz_common::new_id();
    let binding_id = choruz_common::new_id();

    let (client, connection) = tokio_postgres::connect(&db_url, NoTls)
        .await
        .expect("connect for watcher set_cron test");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "INSERT INTO principal (id, workspace_id, type, name, disabled, created_at, updated_at)
                 VALUES ($1, $2, 'agent', 'Cron Agent', FALSE, NOW(), NOW())",
            &[&agent_id, &workspace_id],
        )
        .await
        .expect("seed agent");
    client
            .execute(
                "INSERT INTO conversation (id, workspace_id, type, name, creator_id, created_at, updated_at)
                 VALUES ($1, $2, 'direct', NULL, $3, NOW(), NOW())",
                &[&conversation_id, &workspace_id, &agent_id],
            )
            .await
            .expect("seed bound conversation");
    client
        .execute(
            "INSERT INTO agent_runtime_bindings
                    (id, conversation_id, agent_principal_id, driver_type, workspace_path, state)
                 VALUES ($1, $2, $3, 'codex_terminal', '/tmp/choruz-cron-test', 'idle')",
            &[&binding_id, &conversation_id, &agent_id],
        )
        .await
        .expect("seed runtime binding");

    let store = EventStore::new(&db_url);
    let tmp = tempdir().expect("temp workspace");
    let reply = process_single_outbox_command(
        &format!("watcher:{binding_id}"),
        &agent_id,
        tmp.path(),
        "http://127.0.0.1:3000",
        Some(&store),
        &serde_json::json!({
            "type": "set_cron",
            "name": "daily summary",
            "schedule": "1h",
            "message": "summarize",
        }),
    )
    .await;
    assert!(reply.is_none());

    let row = client
        .query_one(
            "SELECT conversation_id FROM agent_cron_job WHERE agent_id = $1",
            &[&agent_id],
        )
        .await
        .expect("cron job exists");
    assert_eq!(row.get::<_, String>("conversation_id"), conversation_id);
}

/// 9.11: prove the documented non-chat feedback path produces well-formed
/// envelopes — restricted key set, ok=false, no chat noise, correlation
/// fields preserved, persisted on disk — for `task_create`,
/// `task_update`, and `task_transfer` failures emitted from
/// `process_outbox_commands_with_stats`.
///
/// **Scope.** Both call sites of the function — `executor.rs` (headless)
/// and `outbox_watcher.rs` (PTY/watcher) — pass through the same
/// `process_outbox_commands_with_stats` ->
/// `process_channel_task_command` -> `persist_outbox_command_result`
/// chain, so the result-dir contract is mode-independent by
/// construction. This test exercises both `agent-id:conv-id`
/// (headless-shaped) and `watcher:binding-id` (watcher-shaped)
/// session_keys to lock that in for the *persistence* layer. It does
/// **not** drive the watcher's outbox-claim loop end to end; the
/// `outbox_watcher` test module covers that path, and the test name
/// here therefore avoids the "both modes" overclaim and pins what is
/// actually proven.
///
/// The leaky-gateway sanitizer path — which `missing_target` failures
/// cannot exercise because their `message` is a static internal string —
/// is covered separately by
/// `channel_task_command_failure_sanitizes_bearer_tokens_in_message`.
#[tokio::test]
async fn results_dir_path_and_envelope_shape_are_session_key_independent_for_failure_envelopes() {
    let _env = ChannelTaskEnvGuard::enabled();

    let failure_cases = [
        (
            "task_create",
            serde_json::json!({
                "type": "task_create",
                "task_key": "NO-TARGET-CREATE",
                "title": "Missing conversation target",
                "idempotency_key": "turn-feedback:create",
            }),
            "missing_target",
            Some("turn-feedback:create"),
        ),
        (
            "task_update",
            serde_json::json!({
                "type": "task_update",
                "task_key": "NO-TARGET-UPDATE",
                "status": "in_progress",
            }),
            "missing_target",
            None,
        ),
        (
            "task_transfer",
            serde_json::json!({
                "type": "task_transfer",
                "task_key": "NO-TARGET-TRANSFER",
                "assignee": "qa-engineer",
            }),
            "missing_target",
            None,
        ),
    ];

    for session_key in ["agent-1:conv-1", "watcher:binding-1"] {
        let tmp = tempdir().expect("temp workspace");
        let maildir_new = tmp.path().join(".choruz-outbox").join("new");
        std::fs::create_dir_all(&maildir_new).expect("maildir new");
        for (idx, (_, command, _, _)) in failure_cases.iter().enumerate() {
            std::fs::write(
                maildir_new.join(format!("cmd-{idx}.json")),
                command.to_string(),
            )
            .expect("write task command");
        }

        let result = super::process_outbox_commands_with_stats(
            session_key,
            "agent-1",
            tmp.path(),
            "http://127.0.0.1:1",
            None,
        )
        .await;

        assert_eq!(
            result.reply, "",
            "mode {session_key}: task command failures must never enter the conversation reply",
        );
        assert_eq!(
            result.processed_count,
            failure_cases.len(),
            "mode {session_key}: every queued task command should be processed",
        );
        assert_eq!(
            result.command_results.len(),
            failure_cases.len(),
            "mode {session_key}: every queued task command should emit one envelope",
        );

        for envelope in &result.command_results {
            assert_eq!(
                envelope["ok"],
                serde_json::Value::Bool(false),
                "mode {session_key}: every failure envelope must set ok=false",
            );
        }
        let by_type: std::collections::HashMap<&str, &serde_json::Value> = result
            .command_results
            .iter()
            .map(|envelope| (envelope["command_type"].as_str().unwrap_or(""), envelope))
            .collect();
        for (command_type, _, expected_code, expected_idempotency_key) in &failure_cases {
            let envelope = by_type.get(command_type).unwrap_or_else(|| {
                panic!("mode {session_key}: missing envelope for {command_type}")
            });
            assert_eq!(
                envelope["error_code"].as_str(),
                Some(*expected_code),
                "mode {session_key}: {command_type} envelope must carry error_code {expected_code}",
            );

            // Correlation: failures on `task_create` (where task_key/task_id
            // are server-generated and therefore null) MUST preserve the
            // caller's `idempotency_key` so multi-create turns can be
            // correlated back to a source command.
            match expected_idempotency_key {
                Some(expected) => assert_eq!(
                    envelope["idempotency_key"].as_str(),
                    Some(*expected),
                    "mode {session_key}: {command_type} envelope must carry idempotency_key for correlation",
                ),
                None => assert!(
                    envelope["idempotency_key"].is_null(),
                    "mode {session_key}: {command_type} envelope should have null idempotency_key when the command did not supply one",
                ),
            }

            // emitted_at: every envelope is timestamped so an agent on
            // turn N can ignore stale results from turn N-3.
            let emitted_at = envelope["emitted_at"]
                .as_str()
                .unwrap_or_else(|| panic!("mode {session_key}: envelope must carry emitted_at"));
            assert!(
                chrono::DateTime::parse_from_rfc3339(emitted_at).is_ok(),
                "mode {session_key}: emitted_at `{emitted_at}` must be RFC3339",
            );

            // The envelope must be restricted to the documented keys —
            // unexpected keys would be an instruction-contract drift the
            // agent did not learn to parse, and a privacy surface we did
            // not promise to keep safe.
            let object = envelope
                .as_object()
                .unwrap_or_else(|| panic!("envelope must be a JSON object"));
            let allowed = [
                "command_type",
                "ok",
                "error_code",
                "message",
                "task_key",
                "task_id",
                "idempotency_key",
                "emitted_at",
            ];
            for key in object.keys() {
                assert!(
                    allowed.contains(&key.as_str()),
                    "mode {session_key}: failure envelope leaks undocumented key `{key}`",
                );
            }
        }

        // Both modes converge on the same on-disk path, and the persisted
        // files match the in-memory result envelopes byte-for-byte.
        let results_dir = tmp.path().join(".choruz-outbox").join("results");
        let mut persisted: Vec<serde_json::Value> = std::fs::read_dir(&results_dir)
            .expect("results dir exists after processing")
            .filter_map(|entry| {
                let path = entry.expect("readable result entry").path();
                // Tempfiles from the atomic-rename writer are filtered
                // out; only finalized `<id>.json` files count as durable
                // envelopes for the agent.
                if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                    return None;
                }
                Some(
                    serde_json::from_str(
                        &std::fs::read_to_string(&path).expect("read result file"),
                    )
                    .expect("result file is JSON"),
                )
            })
            .collect();
        persisted.sort_by(|a, b| {
            a["command_type"]
                .as_str()
                .unwrap_or("")
                .cmp(b["command_type"].as_str().unwrap_or(""))
        });
        let mut expected = result.command_results.clone();
        expected.sort_by(|a, b| {
            a["command_type"]
                .as_str()
                .unwrap_or("")
                .cmp(b["command_type"].as_str().unwrap_or(""))
        });
        assert_eq!(
            persisted, expected,
            "mode {session_key}: persisted JSON envelopes must equal the in-memory command_results",
        );

        // Privacy: the persisted bytes must not echo agent tokens,
        // prompts, bearer headers, raw gateway HTML/JSON dumps, or
        // hidden principal-id giveaways like `bearer ` or `password`.
        for entry in std::fs::read_dir(&results_dir).expect("results dir") {
            let path = entry.expect("readable entry").path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let raw = std::fs::read_to_string(&path).expect("read result file");
            let lower = raw.to_lowercase();
            for forbidden in [
                "bearer ",
                "authorization",
                "agent_token",
                "password",
                "private_key",
                "secret",
                "prompt",
            ] {
                assert!(
                    !lower.contains(forbidden),
                    "mode {session_key}: persisted result must not echo `{forbidden}` (found in {})",
                    path.display(),
                );
            }
        }
    }
}

/// Companion regression for review comment #3: the leaky-gateway path
/// (`extract_gateway_error_detail` copies upstream `error.detail` into
/// the envelope `message`) must be scrubbed before persistence.
/// `missing_target` cases use a static internal message and cannot prove
/// this on their own.
#[test]
fn channel_task_command_failure_sanitizes_bearer_tokens_in_message() {
    let cmd = serde_json::json!({
        "type": "task_create",
        "idempotency_key": "turn-99:scrub-check",
    });
    let envelope = super::channel_task_command_failure(
        "task_create",
        "unauthorized",
        r#"Bearer abc.def.ghi was rejected by gateway. authorization: Bearer xyz token=plain session_token=session "token":"json-token""#,
        &cmd,
    );
    let message = envelope["message"].as_str().expect("message field");
    let lower = message.to_lowercase();
    assert!(
        !lower.contains("bearer "),
        "message must not echo bearer tokens; got `{message}`"
    );
    assert!(
        !lower.contains("authorization"),
        "message must not echo Authorization header; got `{message}`"
    );
    assert!(
        !lower.contains("plain") && !lower.contains("session") && !lower.contains("json-token"),
        "message must not echo generic token values; got `{message}`"
    );
    assert!(
        message.contains("[redacted]"),
        "scrubbed message must mark redactions; got `{message}`"
    );
    // Correlation is preserved even after scrubbing.
    assert_eq!(envelope["idempotency_key"], "turn-99:scrub-check");
    assert_eq!(envelope["error_code"], "unauthorized");
}

#[test]
fn metadata_for_group_send_injects_thread_fields() {
    // thread param → reply_to_id + thread:true + broadcast default TRUE.
    let cmd = serde_json::json!({
        "type": "send", "group": "g", "content": "c",
        "thread": "root-1"
    });
    let meta = metadata_for_group_send_command(&cmd).expect("ok");
    assert_eq!(meta["reply_to_id"], "root-1");
    assert_eq!(meta["thread"], true);
    assert_eq!(
        meta["broadcast"], true,
        "agent thread replies broadcast by default (operator visibility)"
    );

    // Explicit broadcast:false honored.
    let quiet = serde_json::json!({
        "type": "send", "group": "g", "content": "c",
        "thread": "root-1", "broadcast": false
    });
    let qmeta = metadata_for_group_send_command(&quiet).expect("ok");
    assert_eq!(qmeta["broadcast"], false);

    // Invalid thread value rejected.
    let bad = serde_json::json!({
        "type": "send", "group": "g", "content": "c", "thread": 42
    });
    assert!(metadata_for_group_send_command(&bad).is_err());
    let empty = serde_json::json!({
        "type": "send", "group": "g", "content": "c", "thread": ""
    });
    assert!(metadata_for_group_send_command(&empty).is_err());

    // Non-boolean broadcast rejected (string "false" must NOT silently
    // coerce to the broadcast default — that would invert an explicit
    // quiet-reply intent).
    let stringly = serde_json::json!({
        "type": "send", "group": "g", "content": "c",
        "thread": "root-1", "broadcast": "false"
    });
    assert!(metadata_for_group_send_command(&stringly).is_err());

    // No thread param → metadata untouched.
    let plain = serde_json::json!({"type": "send", "group": "g", "content": "c"});
    let pmeta = metadata_for_group_send_command(&plain).expect("ok");
    assert!(pmeta.get("thread").is_none());
}

#[tokio::test]
async fn send_to_group_threads_canonicalize_and_gate_unread() {
    // Agent thread replies through the outbox must (a) canonicalize
    // reply_event_id to the thread root via the shared helper,
    // (b) skip the total_msg_count bump when quiet, (c) bump when
    // broadcast, and (d) error on a missing thread target.
    let Ok(db_url) = std::env::var("CHORUZ_DATABASE_URL") else {
        return;
    };

    let workspace = choruz_common::new_id();
    let agent = choruz_common::new_id();
    let human = choruz_common::new_id();
    let conv = choruz_common::new_id();
    let group_name = format!("thread-grp-{}", &choruz_common::new_id()[..8]);

    let (client, connection) = tokio_postgres::connect(&db_url, NoTls)
        .await
        .expect("connect for thread outbox test");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "INSERT INTO principal (id, workspace_id, type, name, disabled, created_at, updated_at)
                 VALUES ($1, $2, 'agent', 'Thread Agent', FALSE, NOW(), NOW()),
                        ($3, $2, 'human', 'Thread Human', FALSE, NOW(), NOW())",
            &[&agent, &workspace, &human],
        )
        .await
        .expect("seed principals");
    client
            .execute(
                "INSERT INTO conversation (id, workspace_id, type, name, creator_id, created_at, updated_at)
                 VALUES ($1, $2, 'group', $3, $4, NOW(), NOW())",
                &[&conv, &workspace, &group_name, &human],
            )
            .await
            .expect("seed conversation");
    client
        .execute(
            "INSERT INTO conversation_member (conv_id, principal_id, joined_at)
                 VALUES ($1, $2, NOW()), ($1, $3, NOW())",
            &[&conv, &agent, &human],
        )
        .await
        .expect("seed members");
    // Root message from the human, then a threaded reply under it so
    // the canonicalization has a non-root target to resolve.
    let root_id = choruz_common::new_id();
    let mid_id = choruz_common::new_id();
    client
        .execute(
            "INSERT INTO conversation_events
                    (conversation_id, seq, event_id, event_type, sender_id, content,
                     content_type, metadata, created_at)
                 VALUES ($1, 1, $2, 'message', $3, 'root', 'text/plain', '{}'::jsonb, NOW())",
            &[&conv, &root_id, &human],
        )
        .await
        .expect("seed root");
    client
            .execute(
                "INSERT INTO conversation_events
                    (conversation_id, seq, event_id, event_type, sender_id, content,
                     content_type, metadata, reply_event_id, created_at)
                 VALUES ($1, 2, $2, 'message', $3, 'mid reply', 'text/plain',
                         '{\"thread\": true, \"reply_to_id\": \"will-be-root\"}'::jsonb, $4, NOW())",
                &[&conv, &mid_id, &human, &root_id],
            )
            .await
            .expect("seed mid reply");

    let store = EventStore::new(&db_url);

    async fn count_for(c: &tokio_postgres::Client, conv: &str) -> i64 {
        let row = c
            .query_one(
                "SELECT total_msg_count FROM conversation WHERE id = $1",
                &[&conv],
            )
            .await
            .expect("count");
        row.get::<_, i64>(0)
    }
    let before = count_for(&client, &conv).await;

    // (a)+(b): QUIET threaded reply targeting the MID reply — must
    // canonicalize to root and not bump the counter.
    let quiet_meta = serde_json::json!({
        "reply_to_id": mid_id, "thread": true, "broadcast": false
    });
    send_to_group(
        "sess-thread-test",
        &agent,
        &store,
        &group_name,
        "quiet agent reply",
        "text/plain",
        quiet_meta,
    )
    .await
    .expect("quiet threaded send succeeds");
    let quiet_row = client
        .query_one(
            "SELECT reply_event_id FROM conversation_events
                 WHERE conversation_id = $1 AND content = 'quiet agent reply'",
            &[&conv],
        )
        .await
        .expect("quiet row");
    assert_eq!(
        quiet_row.get::<_, Option<String>>(0).as_deref(),
        Some(root_id.as_str()),
        "agent reply targeting a mid reply canonicalizes to the root"
    );
    assert_eq!(
        count_for(&client, &conv).await,
        before,
        "quiet agent thread reply must not bump total_msg_count"
    );

    // (c): broadcast reply bumps.
    let bcast_meta = serde_json::json!({
        "reply_to_id": root_id, "thread": true, "broadcast": true
    });
    send_to_group(
        "sess-thread-test",
        &agent,
        &store,
        &group_name,
        "broadcast agent reply",
        "text/plain",
        bcast_meta,
    )
    .await
    .expect("broadcast threaded send succeeds");
    assert_eq!(
        count_for(&client, &conv).await,
        before + 1,
        "broadcast agent thread reply bumps total_msg_count"
    );

    // (d): missing target errors with an agent-readable message.
    let missing_meta = serde_json::json!({
        "reply_to_id": "no-such-event", "thread": true
    });
    let err = send_to_group(
        "sess-thread-test",
        &agent,
        &store,
        &group_name,
        "ghost",
        "text/plain",
        missing_meta,
    )
    .await
    .expect_err("missing thread target must error");
    assert!(
        err.contains("thread target"),
        "error names the thread target problem: {err}"
    );
}
