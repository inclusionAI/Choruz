use std::{env, fs, path::PathBuf, process::Command, sync::OnceLock};

use choruz_agent_runtime::{
    AuditActor, AutoMode, BindingState, CodexTerminalCaptureInput, CreateBindingInput, DriverType,
    RuntimeStore, TerminalSessionAnchorInput, UntaggedHumanMode, UpsertPolicyInput,
    normalize_workspace_path,
};
use serde_json::json;
use tokio_postgres::NoTls;
use uuid::Uuid;

struct TestDatabase {
    database_url: String,
    admin_database_url: String,
    database_name: String,
}

impl TestDatabase {
    async fn create() -> Self {
        let admin_database_url = connection_string("postgres");
        let database_name = format!("choruz_runtime_{}", Uuid::now_v7().simple());
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
                    .filter(|s| !s.is_empty())
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

    async fn audit_actions(&self) -> Vec<String> {
        let (client, connection) = tokio_postgres::connect(&self.database_url, NoTls)
            .await
            .expect("connect temp db");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        client
            .query("SELECT action FROM audit_log ORDER BY created_at ASC", &[])
            .await
            .expect("query audit log")
            .into_iter()
            .map(|row| row.get::<_, String>(0))
            .collect()
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
                    .expect("connect human db for cleanup");
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
                .unwrap_or_else(|error| panic!("connect human db after auto-start: {error}"))
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

fn audit_actor() -> AuditActor {
    AuditActor {
        actor_id: "human-1".into(),
        workspace_id: "ws-acme".into(),
    }
}

async fn seed_prerequisites(database_url: &str, principal_ids: &[&str], conversation_ids: &[&str]) {
    let (client, connection) = tokio_postgres::connect(database_url, NoTls)
        .await
        .expect("connect for seeding");
    tokio::spawn(async move {
        let _ = connection.await;
    });

    // Ensure human-1 principal always exists (used by audit_actor) and is
    // represented as the human operator rather than an agent.
    client
        .execute(
            "INSERT INTO principal (id, workspace_id, type, name)
             VALUES ('human-1', 'ws-acme', 'human', 'human-1')
             ON CONFLICT (id) DO NOTHING",
            &[],
        )
        .await
        .expect("seed human principal");

    for id in principal_ids {
        if *id == "human-1" {
            continue;
        }
        client
            .execute(
                "INSERT INTO principal (id, workspace_id, type, name)
                 VALUES ($1, 'ws-acme', 'agent', $1)
                 ON CONFLICT (id) DO NOTHING",
                &[id],
            )
            .await
            .expect("seed principal");
    }

    for id in conversation_ids {
        client
            .execute(
                "INSERT INTO conversation (id, workspace_id, type, name, creator_id)
                 VALUES ($1, 'ws-acme', 'group', $1, 'human-1')
                 ON CONFLICT (id) DO NOTHING",
                &[id],
            )
            .await
            .expect("seed conversation");
    }
}

#[test]
fn workspace_paths_are_normalized_and_guarded() {
    assert_eq!(
        normalize_workspace_path("/tmp/project///").expect("normalize path"),
        "/tmp/project"
    );
    assert!(normalize_workspace_path("").is_err());
    assert!(normalize_workspace_path("../tmp/project").is_err());
}

#[test]
fn state_transitions_are_guarded() {
    assert!(BindingState::Idle.can_transition_to(&BindingState::Running));
    assert!(BindingState::Running.can_transition_to(&BindingState::Idle));
    assert!(!BindingState::Disabled.can_transition_to(&BindingState::Running));
}

#[tokio::test]
async fn binding_defaults_and_uniqueness_are_enforced() {
    let database = TestDatabase::create().await;
    seed_prerequisites(&database.database_url, &["agent-1"], &["conv-1"]).await;
    let store = RuntimeStore::new(database.database_url.clone());

    let binding = store
        .create_binding(CreateBindingInput {
            conversation_id: "conv-1".into(),
            agent_principal_id: "agent-1".into(),
            driver_type: DriverType::ClaudePrint,
            workspace_path: "/tmp/worktrees/claude/".into(),
            git_worktree_path: Some("/tmp/worktrees/claude".into()),
            config_json: json!({}),
            audit_actor: Some(audit_actor()),
        })
        .await
        .expect("create binding");

    assert_eq!(binding.state, BindingState::Idle);
    assert_eq!(binding.last_event_cursor, 0);
    assert_eq!(binding.last_acked_event_cursor, 0);
    assert_eq!(binding.config_json, json!({}));
    assert_eq!(binding.workspace_path, "/tmp/worktrees/claude");

    let duplicate = store
        .create_binding(CreateBindingInput {
            conversation_id: "conv-1".into(),
            agent_principal_id: "agent-1".into(),
            driver_type: DriverType::ClaudePrint,
            workspace_path: "/tmp/worktrees/claude".into(),
            git_worktree_path: None,
            config_json: json!({}),
            audit_actor: None,
        })
        .await;
    assert!(duplicate.is_err(), "duplicate binding should be rejected");
}

#[tokio::test]
async fn disabling_agent_bindings_is_atomic_and_idempotent() {
    let database = TestDatabase::create().await;
    seed_prerequisites(
        &database.database_url,
        &["agent-disable"],
        &["conv-disable"],
    )
    .await;
    let store = RuntimeStore::new(database.database_url.clone());
    let binding = store
        .create_binding(CreateBindingInput {
            conversation_id: "conv-disable".into(),
            agent_principal_id: "agent-disable".into(),
            driver_type: DriverType::OpenCodeTerminal,
            workspace_path: "/tmp/worktrees/disable".into(),
            git_worktree_path: None,
            config_json: json!({}),
            audit_actor: None,
        })
        .await
        .expect("create binding");

    assert_eq!(
        store
            .disable_bindings_by_agent("agent-disable")
            .await
            .expect("disable binding"),
        1
    );
    assert_eq!(
        store.get_binding(&binding.id).await.unwrap().state,
        BindingState::Disabled
    );
    assert_eq!(
        store
            .disable_bindings_by_agent("agent-disable")
            .await
            .expect("repeat disable"),
        0
    );
}

#[tokio::test]
async fn binding_creation_waits_for_disable_and_rejects_the_disabled_agent() {
    let database = TestDatabase::create().await;
    seed_prerequisites(
        &database.database_url,
        &["agent-disable-race"],
        &["conv-disable-race"],
    )
    .await;
    let store = RuntimeStore::new(database.database_url.clone());
    let (mut lock_client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect lock client");
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let disable_tx = lock_client.transaction().await.expect("begin disable");
    disable_tx
        .query_one(
            "SELECT id FROM principal WHERE id = $1 FOR UPDATE",
            &[&"agent-disable-race"],
        )
        .await
        .expect("lock Agent for disable");
    disable_tx
        .execute(
            "UPDATE principal SET disabled = TRUE WHERE id = $1",
            &[&"agent-disable-race"],
        )
        .await
        .expect("disable Agent in transaction");

    let mut create = tokio::spawn(async move {
        store
            .create_binding(CreateBindingInput {
                conversation_id: "conv-disable-race".into(),
                agent_principal_id: "agent-disable-race".into(),
                driver_type: DriverType::CodexTerminal,
                workspace_path: "/tmp/worktrees/disable-race".into(),
                git_worktree_path: None,
                config_json: json!({}),
                audit_actor: None,
            })
            .await
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut create)
            .await
            .is_err(),
        "binding creation must wait for the principal disable lock"
    );

    disable_tx.commit().await.expect("commit disable");
    let result = create.await.expect("join binding creation");
    assert!(
        result.is_err(),
        "disabled Agent must reject binding creation"
    );

    let (verify_client, verify_connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect verification client");
    tokio::spawn(async move {
        let _ = verify_connection.await;
    });
    let count: i64 = verify_client
        .query_one(
            "SELECT COUNT(*) FROM agent_runtime_bindings WHERE agent_principal_id = $1",
            &[&"agent-disable-race"],
        )
        .await
        .expect("count bindings")
        .get(0);
    assert_eq!(count, 0, "no executable binding may survive the race");
}

#[tokio::test]
async fn terminal_session_anchor_preserves_unrelated_config_and_validates_binding() {
    let database = TestDatabase::create().await;
    seed_prerequisites(
        &database.database_url,
        &["agent-terminal"],
        &["conv-terminal"],
    )
    .await;
    let store = RuntimeStore::new(database.database_url.clone());

    let binding = store
        .create_binding(CreateBindingInput {
            conversation_id: "conv-terminal".into(),
            agent_principal_id: "agent-terminal".into(),
            driver_type: DriverType::CodexTerminal,
            workspace_path: "/tmp/worktrees/codex-terminal".into(),
            git_worktree_path: None,
            config_json: json!({
                "binary_path": "codex",
                "mention_aliases": ["Codex"],
                "allowed_tools": ["Read"]
            }),
            audit_actor: Some(audit_actor()),
        })
        .await
        .expect("create binding");

    let updated = store
        .write_terminal_session_anchor(
            &binding.id,
            TerminalSessionAnchorInput {
                session_id: "00000000-0000-0000-0000-000000000001".into(),
                source: "native_cli".into(),
                provenance: "terminal_process_captured".into(),
                binding_id: binding.id.clone(),
                conversation_id: binding.conversation_id.clone(),
                agent_principal_id: binding.agent_principal_id.clone(),
                company_id: "company-acme".into(),
                driver_type: binding.driver_type.as_str().into(),
                workspace_id: "ws-acme".into(),
                workspace_path: binding.workspace_path.clone(),
                native_home_path: "/tmp/choruz/codex-homes/binding".into(),
                native_session_path: "/tmp/choruz/codex-homes/binding/sessions/session.jsonl"
                    .into(),
                binding_generation: binding.terminal_generation(),
                binding_updated_at: binding.updated_at,
            },
        )
        .await
        .expect("write terminal anchor");

    assert_eq!(updated.config_json["binary_path"], "codex");
    assert_eq!(updated.config_json["mention_aliases"][0], "Codex");
    assert_eq!(updated.config_json["allowed_tools"][0], "Read");
    assert_eq!(
        updated
            .valid_terminal_session_id_for_workspace(Some("ws-acme"))
            .as_deref(),
        Some("00000000-0000-0000-0000-000000000001")
    );
    assert_eq!(
        updated.valid_terminal_session_id_for_workspace(Some("ws-other")),
        None
    );

    let rebound = store
        .rebind_workspace(
            &binding.id,
            "/tmp/worktrees/codex-terminal-next",
            &audit_actor(),
        )
        .await
        .expect("rebind workspace");
    assert!(rebound.config_json.get("terminal_session").is_none());
    assert_eq!(rebound.config_json["binary_path"], "codex");
    assert_eq!(rebound.valid_terminal_session_id(), None);

    let stale_capture = store
        .write_terminal_session_anchor(
            &binding.id,
            TerminalSessionAnchorInput {
                session_id: "00000000-0000-0000-0000-000000000002".into(),
                source: "native_cli".into(),
                provenance: "terminal_process_captured".into(),
                binding_id: binding.id.clone(),
                conversation_id: binding.conversation_id.clone(),
                agent_principal_id: binding.agent_principal_id.clone(),
                company_id: "company-acme".into(),
                driver_type: binding.driver_type.as_str().into(),
                workspace_id: "ws-acme".into(),
                workspace_path: binding.workspace_path.clone(),
                native_home_path: "/tmp/choruz/codex-homes/binding".into(),
                native_session_path: "/tmp/choruz/codex-homes/binding/sessions/session-2.jsonl"
                    .into(),
                binding_generation: binding.terminal_generation(),
                binding_updated_at: binding.updated_at,
            },
        )
        .await;
    assert!(
        stale_capture.is_err(),
        "delayed captures from the old binding context must not resurrect anchors"
    );
}

#[tokio::test]
async fn terminal_session_anchor_rejects_delayed_capture_after_reset_touch() {
    let database = TestDatabase::create().await;
    seed_prerequisites(
        &database.database_url,
        &["agent-terminal-reset"],
        &["conv-terminal-reset"],
    )
    .await;
    let store = RuntimeStore::new(database.database_url.clone());

    let binding = store
        .create_binding(CreateBindingInput {
            conversation_id: "conv-terminal-reset".into(),
            agent_principal_id: "agent-terminal-reset".into(),
            driver_type: DriverType::CodexTerminal,
            workspace_path: "/tmp/worktrees/codex-terminal-reset".into(),
            git_worktree_path: None,
            config_json: json!({ "binary_path": "codex" }),
            audit_actor: Some(audit_actor()),
        })
        .await
        .expect("create binding");

    let captured_before_reset = TerminalSessionAnchorInput {
        session_id: "00000000-0000-0000-0000-000000000011".into(),
        source: "native_cli".into(),
        provenance: "terminal_process_captured".into(),
        binding_id: binding.id.clone(),
        conversation_id: binding.conversation_id.clone(),
        agent_principal_id: binding.agent_principal_id.clone(),
        company_id: "company-acme".into(),
        driver_type: binding.driver_type.as_str().into(),
        workspace_id: "ws-acme".into(),
        workspace_path: binding.workspace_path.clone(),
        native_home_path: "/tmp/choruz/codex-homes/binding".into(),
        native_session_path: "/tmp/choruz/codex-homes/binding/sessions/session.jsonl".into(),
        binding_generation: binding.terminal_generation(),
        binding_updated_at: binding.updated_at,
    };

    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect for reset simulation");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "UPDATE agent_runtime_bindings
             SET config_json = config_json - 'terminal_session',
                 updated_at = updated_at + interval '1 second'
             WHERE id = $1",
            &[&binding.id],
        )
        .await
        .expect("simulate reset touch");

    let stale_capture = store
        .write_terminal_session_anchor(&binding.id, captured_before_reset)
        .await;
    assert!(
        stale_capture.is_err(),
        "captures observed before a reset must not recreate a terminal anchor"
    );
    let current = store.get_binding(&binding.id).await.expect("load binding");
    assert_eq!(current.valid_terminal_session_id(), None);
}

#[tokio::test]
async fn codex_terminal_capture_metadata_persists_before_anchor_and_clears_on_anchor() {
    let database = TestDatabase::create().await;
    seed_prerequisites(
        &database.database_url,
        &["agent-terminal-capture"],
        &["conv-terminal-capture"],
    )
    .await;
    let store = RuntimeStore::new(database.database_url.clone());

    let binding = store
        .create_binding(CreateBindingInput {
            conversation_id: "conv-terminal-capture".into(),
            agent_principal_id: "agent-terminal-capture".into(),
            driver_type: DriverType::CodexTerminal,
            workspace_path: "/tmp/worktrees/codex-terminal-capture".into(),
            git_worktree_path: None,
            config_json: json!({ "binary_path": "codex" }),
            audit_actor: Some(audit_actor()),
        })
        .await
        .expect("create binding");

    let prepared = store
        .begin_codex_terminal_capture(
            &binding.id,
            CodexTerminalCaptureInput {
                binding_id: binding.id.clone(),
                conversation_id: binding.conversation_id.clone(),
                agent_principal_id: binding.agent_principal_id.clone(),
                company_id: "company-acme".into(),
                driver_type: binding.driver_type.as_str().into(),
                workspace_id: "ws-acme".into(),
                workspace_path: binding.workspace_path.clone(),
                native_home_path: "/tmp/choruz/codex-homes/binding".into(),
                sessions_path: "/tmp/choruz/codex-homes/binding/sessions".into(),
                spawn_started_at: chrono::Utc::now(),
                baseline_session_files: vec!["/tmp/old.jsonl".into()],
                binding_updated_at: binding.updated_at,
            },
        )
        .await
        .expect("begin capture");

    assert_eq!(prepared.terminal_generation(), 1);
    let capture = prepared
        .codex_terminal_capture_metadata()
        .expect("capture metadata");
    assert_eq!(capture.binding_generation, 1);
    assert_eq!(capture.baseline_session_files[0], "/tmp/old.jsonl");
    assert_eq!(prepared.config_json["binary_path"], "codex");
    assert_eq!(prepared.config_json["agent_workspace_id"], "ws-acme");
    assert_eq!(
        prepared.config_json["conversation_workspace_id"],
        "company-acme"
    );

    let anchored = store
        .write_terminal_session_anchor(
            &prepared.id,
            TerminalSessionAnchorInput {
                session_id: "00000000-0000-0000-0000-000000000031".into(),
                source: "native_cli".into(),
                provenance: "terminal_process_captured".into(),
                binding_id: prepared.id.clone(),
                conversation_id: prepared.conversation_id.clone(),
                agent_principal_id: prepared.agent_principal_id.clone(),
                company_id: "company-acme".into(),
                driver_type: prepared.driver_type.as_str().into(),
                workspace_id: "ws-acme".into(),
                workspace_path: prepared.workspace_path.clone(),
                native_home_path: "/tmp/choruz/codex-homes/binding".into(),
                native_session_path: "/tmp/choruz/codex-homes/binding/sessions/session.jsonl"
                    .into(),
                binding_generation: prepared.terminal_generation(),
                binding_updated_at: prepared.updated_at,
            },
        )
        .await
        .expect("write anchor");

    assert!(anchored.config_json.get("terminal_capture").is_none());
    let anchor = anchored.terminal_session_anchor().expect("anchor");
    assert_eq!(anchor.company_id, "company-acme");
    assert_eq!(anchor.binding_generation, Some(1));
    assert_eq!(anchor.native_home_path, "/tmp/choruz/codex-homes/binding");
}

#[tokio::test]
async fn codex_terminal_anchor_rejects_same_native_session_for_same_workspace_binding() {
    let database = TestDatabase::create().await;
    seed_prerequisites(
        &database.database_url,
        &["agent-terminal-same-ws-a", "agent-terminal-same-ws-b"],
        &["conv-terminal-same-ws-a", "conv-terminal-same-ws-b"],
    )
    .await;
    let store = RuntimeStore::new(database.database_url.clone());

    let create = |conversation_id: &str, agent_principal_id: &str| CreateBindingInput {
        conversation_id: conversation_id.into(),
        agent_principal_id: agent_principal_id.into(),
        driver_type: DriverType::CodexTerminal,
        workspace_path: "/tmp/worktrees/shared-codex-terminal".into(),
        git_worktree_path: None,
        config_json: json!({ "binary_path": "codex" }),
        audit_actor: Some(audit_actor()),
    };
    let binding_a = store
        .create_binding(create(
            "conv-terminal-same-ws-a",
            "agent-terminal-same-ws-a",
        ))
        .await
        .expect("create binding a");
    let binding_b = store
        .create_binding(create(
            "conv-terminal-same-ws-b",
            "agent-terminal-same-ws-b",
        ))
        .await
        .expect("create binding b");

    let prepare = |binding: &choruz_agent_runtime::RuntimeBinding| CodexTerminalCaptureInput {
        binding_id: binding.id.clone(),
        conversation_id: binding.conversation_id.clone(),
        agent_principal_id: binding.agent_principal_id.clone(),
        company_id: "company-acme".into(),
        driver_type: binding.driver_type.as_str().into(),
        workspace_id: "ws-acme".into(),
        workspace_path: binding.workspace_path.clone(),
        native_home_path: format!("/tmp/choruz/codex-homes/{}", binding.id),
        sessions_path: format!("/tmp/choruz/codex-homes/{}/sessions", binding.id),
        spawn_started_at: chrono::Utc::now(),
        baseline_session_files: vec![],
        binding_updated_at: binding.updated_at,
    };
    let prepared_a = store
        .begin_codex_terminal_capture(&binding_a.id, prepare(&binding_a))
        .await
        .expect("prepare a");
    let prepared_b = store
        .begin_codex_terminal_capture(&binding_b.id, prepare(&binding_b))
        .await
        .expect("prepare b");

    let session_id = "00000000-0000-0000-0000-000000000041";
    store
        .write_terminal_session_anchor(
            &prepared_a.id,
            TerminalSessionAnchorInput {
                session_id: session_id.into(),
                source: "native_cli".into(),
                provenance: "terminal_process_captured".into(),
                binding_id: prepared_a.id.clone(),
                conversation_id: prepared_a.conversation_id.clone(),
                agent_principal_id: prepared_a.agent_principal_id.clone(),
                company_id: "company-acme".into(),
                driver_type: prepared_a.driver_type.as_str().into(),
                workspace_id: "ws-acme".into(),
                workspace_path: prepared_a.workspace_path.clone(),
                native_home_path: format!("/tmp/choruz/codex-homes/{}", prepared_a.id),
                native_session_path: format!(
                    "/tmp/choruz/codex-homes/{}/sessions/session.jsonl",
                    prepared_a.id
                ),
                binding_generation: prepared_a.terminal_generation(),
                binding_updated_at: prepared_a.updated_at,
            },
        )
        .await
        .expect("write anchor a");

    let duplicate = store
        .write_terminal_session_anchor(
            &prepared_b.id,
            TerminalSessionAnchorInput {
                session_id: session_id.into(),
                source: "native_cli".into(),
                provenance: "terminal_process_captured".into(),
                binding_id: prepared_b.id.clone(),
                conversation_id: prepared_b.conversation_id.clone(),
                agent_principal_id: prepared_b.agent_principal_id.clone(),
                company_id: "company-acme".into(),
                driver_type: prepared_b.driver_type.as_str().into(),
                workspace_id: "ws-acme".into(),
                workspace_path: prepared_b.workspace_path.clone(),
                native_home_path: format!("/tmp/choruz/codex-homes/{}", prepared_b.id),
                native_session_path: format!(
                    "/tmp/choruz/codex-homes/{}/sessions/session.jsonl",
                    prepared_b.id
                ),
                binding_generation: prepared_b.terminal_generation(),
                binding_updated_at: prepared_b.updated_at,
            },
        )
        .await;

    assert!(
        duplicate.is_err(),
        "same native Codex session id must not attach to another binding"
    );
}

#[tokio::test]
async fn policy_defaults_and_upsert_work() {
    let database = TestDatabase::create().await;
    seed_prerequisites(
        &database.database_url,
        &["agent-reviewer"],
        &["conv-policy"],
    )
    .await;
    let store = RuntimeStore::new(database.database_url.clone());

    let default_policy = store
        .get_policy("conv-policy")
        .await
        .expect("default policy");
    assert_eq!(default_policy.auto_mode, AutoMode::MentionedOnly);
    assert_eq!(default_policy.max_auto_turns, 4);
    assert_eq!(
        default_policy.untagged_human_mode,
        UntaggedHumanMode::MentionedOnly
    );
    assert_eq!(default_policy.default_coordinator_agent_id, None);

    let updated = store
        .upsert_policy(UpsertPolicyInput {
            conversation_id: "conv-policy".into(),
            auto_mode: AutoMode::MetadataOnly,
            max_auto_turns: 2,
            max_workflow_turns: 20,
            require_human_after_n_turns: 2,
            allow_agent_to_agent: true,
            allow_file_write: false,
            default_reviewer_agent_id: Some("agent-reviewer".into()),
            default_coordinator_agent_id: Some("agent-reviewer".into()),
            untagged_human_mode: UntaggedHumanMode::CoordinatorOnly,
            audit_actor: Some(audit_actor()),
        })
        .await
        .expect("upsert policy");

    assert_eq!(updated.auto_mode, AutoMode::MetadataOnly);
    assert_eq!(updated.max_auto_turns, 2);
    assert!(updated.allow_agent_to_agent);
    assert_eq!(
        updated.default_reviewer_agent_id.as_deref(),
        Some("agent-reviewer")
    );
    assert_eq!(
        updated.default_coordinator_agent_id.as_deref(),
        Some("agent-reviewer")
    );
    assert_eq!(
        updated.untagged_human_mode,
        UntaggedHumanMode::CoordinatorOnly
    );

    let reloaded = store
        .get_policy("conv-policy")
        .await
        .expect("reload policy");
    assert_eq!(
        reloaded.default_coordinator_agent_id.as_deref(),
        Some("agent-reviewer")
    );
    assert_eq!(
        reloaded.untagged_human_mode,
        UntaggedHumanMode::CoordinatorOnly
    );
}

#[tokio::test]
async fn policy_rejects_invalid_default_coordinator() {
    let database = TestDatabase::create().await;
    seed_prerequisites(
        &database.database_url,
        &["agent-reviewer"],
        &["conv-policy"],
    )
    .await;
    let store = RuntimeStore::new(database.database_url.clone());

    let missing = store
        .upsert_policy(UpsertPolicyInput {
            conversation_id: "conv-policy".into(),
            auto_mode: AutoMode::MetadataOnly,
            max_auto_turns: 2,
            max_workflow_turns: 20,
            require_human_after_n_turns: 2,
            allow_agent_to_agent: true,
            allow_file_write: false,
            default_reviewer_agent_id: None,
            default_coordinator_agent_id: Some("missing-agent".into()),
            untagged_human_mode: UntaggedHumanMode::CoordinatorOnly,
            audit_actor: None,
        })
        .await;
    assert!(
        missing.is_err(),
        "missing coordinator principal should be rejected"
    );

    let empty = store
        .upsert_policy(UpsertPolicyInput {
            conversation_id: "conv-policy".into(),
            auto_mode: AutoMode::MetadataOnly,
            max_auto_turns: 2,
            max_workflow_turns: 20,
            require_human_after_n_turns: 2,
            allow_agent_to_agent: true,
            allow_file_write: false,
            default_reviewer_agent_id: None,
            default_coordinator_agent_id: Some(" ".into()),
            untagged_human_mode: UntaggedHumanMode::CoordinatorOnly,
            audit_actor: None,
        })
        .await;
    assert!(empty.is_err(), "empty coordinator id should be rejected");
}

#[tokio::test]
async fn state_changes_and_rebind_write_audit_entries() {
    let database = TestDatabase::create().await;
    seed_prerequisites(&database.database_url, &["agent-audit"], &["conv-audit"]).await;
    let store = RuntimeStore::new(database.database_url.clone());
    let actor = audit_actor();
    let binding = store
        .create_binding(CreateBindingInput {
            conversation_id: "conv-audit".into(),
            agent_principal_id: "agent-audit".into(),
            driver_type: DriverType::ClaudePrint,
            workspace_path: "/tmp/worktrees/audit".into(),
            git_worktree_path: None,
            config_json: json!({}),
            audit_actor: Some(actor.clone()),
        })
        .await
        .expect("create binding");

    store
        .update_binding_state(&binding.id, BindingState::Paused, Some(&actor))
        .await
        .expect("pause binding");
    store
        .update_binding_state(&binding.id, BindingState::Idle, Some(&actor))
        .await
        .expect("resume binding");
    store
        .rebind_workspace(&binding.id, "/tmp/worktrees/audit-next", &actor)
        .await
        .expect("rebind workspace");

    let actions = database.audit_actions().await;
    assert_eq!(
        actions,
        vec![
            "runtime.binding_created",
            "runtime.binding_paused",
            "runtime.binding_resumed",
            "runtime.binding_rebound",
        ]
    );
}
