use super::*;

#[tokio::test]
async fn remote_completion_does_not_overwrite_a_reconfigured_session_identity() {
    use choruz_session::{InsertCommand, PgSessionStore};
    let database = TestDatabase::create().await;
    let runtime = RuntimeStore::new(database.database_url.clone());
    let store = PgSessionStore::new(&database.database_url);
    let app = choruz_application::ChatApp::new();
    let operator = LocalAuthConfig::from_env()
        .ensure_operator_sync(&app)
        .unwrap();
    let agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Identity fence".into(),
            scopes: vec!["messages:read".into(), "messages:write".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap();
    let conversation = app
        .create_direct_conversation(CreateDirectConversationRequest {
            actor_id: operator.id.clone(),
            peer_principal_id: agent.principal.id.clone(),
            workspace_id: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &operator).await;
    seed_principal_to_db(&database.database_url, &agent.principal).await;
    seed_conversation_to_db(&database.database_url, &conversation).await;
    let host = Uuid::now_v7().to_string();
    let binding = runtime
        .create_binding(CreateBindingInput {
            conversation_id: conversation.id.clone(),
            agent_principal_id: agent.principal.id.clone(),
            driver_type: DriverType::ClaudeTerminal,
            workspace_path: "/test/fenced".into(),
            git_worktree_path: None,
            config_json: json!({"runtime_host_id":host}),
            audit_actor: Some(audit_actor(&operator)),
        })
        .await
        .unwrap();
    let session = Uuid::now_v7().to_string();
    store
        .upsert_session(&session, &agent.principal.id, &conversation.id)
        .await
        .unwrap();
    let client = store.connect().await.unwrap();
    for changed in [false, true] {
        let command = store
            .insert_command(&InsertCommand {
                command_id: Uuid::now_v7().to_string(),
                route_id: Uuid::now_v7().to_string(),
                session_key: session.clone(),
                agent_id: agent.principal.id.clone(),
                conversation_id: conversation.id.clone(),
                message_id: Uuid::now_v7().to_string(),
                turn_id: Uuid::now_v7().to_string(),
                prompt: "Inspect workspace".into(),
                max_attempts: 1,
                metadata: json!({}),
            })
            .await
            .unwrap();
        let (claimed, lease) = store
            .claim_runtime_host_command(&host)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claimed.command_id, command.command_id);
        assert_eq!(claimed.metadata["runtime_binding"]["id"], binding.id);
        if changed {
            client.execute("UPDATE agent_runtime_bindings SET external_session_id='replacement', config_json=config_json || '{\"terminal_generation\":2}'::jsonb WHERE id=$1", &[&binding.id]).await.unwrap();
        } else {
            client.execute("UPDATE agent_runtime_bindings SET config_json=config_json || '{\"terminal_session\":{\"session_id\":\"direct\"}}'::jsonb WHERE id=$1", &[&binding.id]).await.unwrap();
        }
        store
            .complete_runtime_host_command(
                &host,
                "Selected device",
                &command.command_id,
                &lease.attempt_id,
                true,
                &[],
                None,
                0,
                1,
                Some("headless-result"),
                false,
            )
            .await
            .unwrap();
        let current = runtime.get_binding(&binding.id).await.unwrap();
        assert_eq!(
            current.external_session_id.as_deref(),
            Some(if changed {
                "replacement"
            } else {
                "headless-result"
            })
        );
        assert_eq!(
            current.config_json["terminal_session"]["session_id"],
            "direct"
        );
    }
}

#[tokio::test]
async fn headless_progress_tracks_leases_in_binding_snapshots_and_sync() {
    use choruz_session::{CommandStatus, CommandStatusUpdate, InsertCommand, PgSessionStore};

    let database = TestDatabase::create_without_migrations().await;
    database
        .apply_migrations_through("V048__online_groups.sql")
        .await;
    let runtime = RuntimeStore::new(database.database_url.clone());
    let store = PgSessionStore::new(&database.database_url);
    let app = choruz_application::ChatApp::new();
    let operator = LocalAuthConfig::from_env()
        .ensure_operator_sync(&app)
        .unwrap();
    let agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Headless progress".into(),
            scopes: vec!["messages:read".into(), "messages:write".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap();
    let conversation = app
        .create_direct_conversation(CreateDirectConversationRequest {
            actor_id: operator.id.clone(),
            peer_principal_id: agent.principal.id.clone(),
            workspace_id: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &operator).await;
    seed_principal_to_db(&database.database_url, &agent.principal).await;
    seed_conversation_to_db(&database.database_url, &conversation).await;
    let binding = runtime
        .create_binding(CreateBindingInput {
            conversation_id: conversation.id.clone(),
            agent_principal_id: agent.principal.id.clone(),
            driver_type: DriverType::ClaudeTerminal,
            workspace_path: "/worktrees/progress".into(),
            git_worktree_path: None,
            config_json: json!({}),
            audit_actor: Some(audit_actor(&operator)),
        })
        .await
        .unwrap();
    let router = runtime_router_with_db(app, runtime.clone(), &database.database_url);
    let client = store.connect().await.unwrap();
    let session = Uuid::now_v7().to_string();
    store
        .upsert_session(&session, &agent.principal.id, &conversation.id)
        .await
        .unwrap();
    let input = || InsertCommand {
        command_id: Uuid::now_v7().to_string(),
        route_id: Uuid::now_v7().to_string(),
        session_key: session.clone(),
        agent_id: agent.principal.id.clone(),
        conversation_id: conversation.id.clone(),
        message_id: Uuid::now_v7().to_string(),
        turn_id: Uuid::now_v7().to_string(),
        prompt: "Work until released".into(),
        max_attempts: 3,
        metadata: json!({}),
    };
    let first = store.insert_command(&input()).await.unwrap();
    let second = store.insert_command(&input()).await.unwrap();
    let (_, before) = api_json_request(
        router.clone(),
        &operator,
        Method::GET,
        "/v1/bootstrap?limit=10".into(),
    )
    .await;
    let cursor = before["sync_cursor"].as_u64().unwrap();
    let leases = store
        .assign_batch_leases(
            &[first.command_id.clone(), second.command_id.clone()],
            "test-executor",
        )
        .await
        .unwrap();

    assert_eq!(
        runtime.get_binding(&binding.id).await.unwrap().state,
        BindingState::Idle,
        "the pre-migration database reproduces the reported idle binding"
    );
    let backoff_session = Uuid::now_v7().to_string();
    store
        .upsert_session(&backoff_session, &agent.principal.id, &conversation.id)
        .await
        .unwrap();
    client
        .execute(
            "UPDATE session_registry SET status = 'active' WHERE session_key = $1",
            &[&backoff_session],
        )
        .await
        .unwrap();
    apply_migration_files(&client, |name| name.starts_with("V049__")).await;
    assert_eq!(
        store
            .get_session(&backoff_session)
            .await
            .unwrap()
            .unwrap()
            .status,
        choruz_session::SessionStatus::Idle,
        "upgrade clears a stranded pre-fix session with no active command"
    );

    let row = client
        .query_one(
            "SELECT state, in_flight_turn_id FROM agent_runtime_bindings WHERE id = $1",
            &[&binding.id],
        )
        .await
        .unwrap();
    assert_eq!(
        row.get::<_, String>("state"),
        "running",
        "a real batch lease must persist binding progress"
    );
    for path in [
        "/v1/runtime/bindings".to_string(),
        format!("/v1/runtime/bindings/{}", binding.id),
        "/v1/bootstrap?limit=10".to_string(),
    ] {
        let (status, body) =
            api_json_request(router.clone(), &operator, Method::GET, path.clone()).await;
        assert_eq!(status, StatusCode::OK);
        let view = if path.contains("bootstrap") {
            &body["runtime_bindings"][0]
        } else if body.is_array() {
            &body[0]
        } else {
            &body
        };
        assert_eq!(view["id"], binding.id);
        assert_eq!(
            view["state"], "running",
            "refresh/list/detail must agree: {path}"
        );
    }
    let (_, sync) = api_json_request(
        router.clone(),
        &operator,
        Method::GET,
        format!("/v1/sync?cursor={cursor}&limit=100"),
    )
    .await;
    assert!(
        sync["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|change| change["event_type"] == "runtime_binding.updated"
                && change["entity_id"] == binding.id)
    );

    let updated_at: String = client
        .query_one(
            "SELECT updated_at::text FROM agent_runtime_bindings WHERE id = $1",
            &[&binding.id],
        )
        .await
        .unwrap()
        .get(0);
    let (_, snapshot) = api_json_request(
        router.clone(),
        &operator,
        Method::GET,
        "/v1/bootstrap?limit=10".into(),
    )
    .await;
    let unchanged_cursor = snapshot["sync_cursor"].as_u64().unwrap();
    client
        .execute(
            "UPDATE session_registry SET epoch = epoch WHERE session_key = $1",
            &[&session],
        )
        .await
        .unwrap();
    let after_update: String = client
        .query_one(
            "SELECT updated_at::text FROM agent_runtime_bindings WHERE id = $1",
            &[&binding.id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        updated_at, after_update,
        "unchanged progress must not rewrite the binding"
    );
    let (_, unchanged_sync) = api_json_request(
        router.clone(),
        &operator,
        Method::GET,
        format!("/v1/sync?cursor={unchanged_cursor}&limit=100"),
    )
    .await;
    assert!(
        !unchanged_sync["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|change| change["event_type"] == "runtime_binding.updated"
                && change["entity_id"] == binding.id),
        "unchanged progress must not emit another binding update"
    );

    store
        .mark_command_committed_for_attempt(
            &first.command_id,
            &leases[&first.command_id].attempt_id,
        )
        .await
        .unwrap();
    let row = client
        .query_one(
            "SELECT state, in_flight_turn_id FROM agent_runtime_bindings WHERE id = $1",
            &[&binding.id],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>("state"), "running");
    store
        .mark_command_committed_for_attempt(
            &second.command_id,
            &leases[&second.command_id].attempt_id,
        )
        .await
        .unwrap();
    let row = client
        .query_one(
            "SELECT state, in_flight_turn_id FROM agent_runtime_bindings WHERE id = $1",
            &[&binding.id],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>("state"), "idle");
    assert_eq!(row.get::<_, Option<String>>("in_flight_turn_id"), None);

    let retry = store.insert_command(&input()).await.unwrap();
    let old_lease = store
        .assign_lease(&retry.command_id, "test-executor")
        .await
        .unwrap();
    let expired = store
        .check_expired_leases(chrono::Utc::now() + chrono::TimeDelta::seconds(121), 120)
        .await
        .unwrap();
    store
        .handle_lease_expiry(
            expired
                .iter()
                .find(|e| e.command_id == retry.command_id)
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        runtime.get_binding(&binding.id).await.unwrap().state,
        BindingState::Idle
    );
    store
        .update_command_status(&CommandStatusUpdate {
            command_id: retry.command_id.clone(),
            status: CommandStatus::Pending,
            ..Default::default()
        })
        .await
        .unwrap();
    let fresh_lease = store
        .assign_lease(&retry.command_id, "replacement-executor")
        .await
        .unwrap();
    assert!(
        store
            .mark_command_committed_for_attempt(&retry.command_id, &old_lease.attempt_id)
            .await
            .is_err()
    );
    assert_eq!(
        runtime.get_binding(&binding.id).await.unwrap().state,
        BindingState::Running
    );
    store
        .dead_letter_command_for_attempt(
            &choruz_session::InsertDeadLetter {
                source_type: "command".into(),
                source_id: retry.command_id.clone(),
                payload: json!({}),
                error: "test execution failed".into(),
                attempt_count: 2,
            },
            &fresh_lease.attempt_id,
        )
        .await
        .unwrap();
    assert_eq!(
        runtime.get_binding(&binding.id).await.unwrap().state,
        BindingState::Idle
    );

    let retry = store.insert_command(&input()).await.unwrap();
    let lease = store
        .assign_lease(&retry.command_id, "test-executor")
        .await
        .unwrap();
    store
        .update_command_status_for_attempt(
            &CommandStatusUpdate {
                command_id: retry.command_id.clone(),
                status: CommandStatus::RetryScheduled,
                next_retry_at: Some(Some(chrono::Utc::now() + chrono::TimeDelta::minutes(5))),
                ..Default::default()
            },
            &lease.attempt_id,
        )
        .await
        .unwrap();
    assert_eq!(
        runtime.get_binding(&binding.id).await.unwrap().state,
        BindingState::Idle,
        "backoff is not active execution even before the retry becomes due"
    );

    let host = Uuid::now_v7().to_string();
    client.execute("UPDATE agent_runtime_bindings SET config_json = jsonb_build_object('runtime_host_id', $2::text) WHERE id = $1", &[&binding.id, &host]).await.unwrap();
    for success in [true, false] {
        let remote = store.insert_command(&input()).await.unwrap();
        let (claimed, lease) = store
            .claim_runtime_host_command(&host)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claimed.command_id, remote.command_id);
        assert_eq!(
            runtime.get_binding(&binding.id).await.unwrap().state,
            BindingState::Running
        );
        store
            .complete_runtime_host_command(
                &host,
                "Test device",
                &remote.command_id,
                &lease.attempt_id,
                success,
                &[],
                if success { None } else { Some("test failure") },
                0,
                1,
                None,
                false,
            )
            .await
            .unwrap();
        assert_eq!(
            runtime.get_binding(&binding.id).await.unwrap().state,
            BindingState::Idle
        );
    }
    client
        .execute(
            "UPDATE agent_runtime_bindings SET config_json = '{}' WHERE id = $1",
            &[&binding.id],
        )
        .await
        .unwrap();

    // Two conversations can contend on the shared binding. Hold its row lock
    // until both production transitions are waiting, then verify the aggregate.
    let other_session = Uuid::now_v7().to_string();
    store
        .upsert_session(&other_session, &agent.principal.id, &conversation.id)
        .await
        .unwrap();
    let finishing = store.insert_command(&input()).await.unwrap();
    let finishing_lease = store
        .assign_lease(&finishing.command_id, "test-executor")
        .await
        .unwrap();
    let mut other_input = input();
    other_input.session_key = other_session;
    let starting = store.insert_command(&other_input).await.unwrap();
    let mut guard_client = store.connect().await.unwrap();
    let guard = guard_client.transaction().await.unwrap();
    guard
        .query_one(
            "SELECT id FROM agent_runtime_bindings WHERE id = $1 FOR UPDATE",
            &[&binding.id],
        )
        .await
        .unwrap();
    let finish_name = format!("progress-finish-{}", Uuid::now_v7());
    let start_name = format!("progress-start-{}", Uuid::now_v7());
    let finish_store = PgSessionStore::new(&format!(
        "{} application_name={finish_name}",
        database.database_url
    ));
    let start_store = PgSessionStore::new(&format!(
        "{} application_name={start_name}",
        database.database_url
    ));
    let finish = tokio::spawn(async move {
        finish_store
            .mark_command_committed_for_attempt(&finishing.command_id, &finishing_lease.attempt_id)
            .await
    });
    let starting_id = starting.command_id.clone();
    let start = tokio::spawn(async move {
        start_store
            .assign_lease(&starting_id, "test-executor")
            .await
    });
    let blocked = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let count: i64 = client.query_one("SELECT COUNT(*) FROM pg_stat_activity WHERE application_name = ANY($1) AND wait_event_type = 'Lock'", &[&vec![finish_name.clone(), start_name.clone()]]).await.unwrap().get(0);
            if count == 2 { break; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await;
    guard.commit().await.unwrap();
    finish.await.unwrap().unwrap();
    let starting_lease = start.await.unwrap().unwrap();
    assert!(
        blocked.is_ok(),
        "both transitions must overlap at the binding row lock"
    );
    assert_eq!(
        runtime.get_binding(&binding.id).await.unwrap().state,
        BindingState::Running
    );
    store
        .mark_command_committed_for_attempt(&starting.command_id, &starting_lease.attempt_id)
        .await
        .unwrap();
    assert_eq!(
        runtime.get_binding(&binding.id).await.unwrap().state,
        BindingState::Idle
    );

    // Administrative states are not overridden by execution progress.
    for state in ["paused", "error", "disabled"] {
        let command = store.insert_command(&input()).await.unwrap();
        client
            .execute(
                "UPDATE agent_runtime_bindings SET state = $2 WHERE id = $1",
                &[&binding.id, &state],
            )
            .await
            .unwrap();
        let lease = store
            .assign_lease(&command.command_id, "test-executor")
            .await
            .unwrap();
        store
            .mark_command_committed_for_attempt(&command.command_id, &lease.attempt_id)
            .await
            .unwrap();
        let row = client
            .query_one(
                "SELECT state FROM agent_runtime_bindings WHERE id = $1",
                &[&binding.id],
            )
            .await
            .unwrap();
        assert_eq!(row.get::<_, String>(0), state);
    }
}

#[tokio::test]
async fn runtime_bindings_list_detail_and_redact_errors() {
    let database = TestDatabase::create().await;
    let runtime = RuntimeStore::new(database.database_url.clone());
    let app = choruz_application::ChatApp::new();
    let auth = LocalAuthConfig::from_env();
    let operator = auth.ensure_operator_sync(&app).unwrap();
    let human = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: operator.workspace_id.clone(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    let agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Claude Dev".into(),
            scopes: vec![
                "messages:read".into(),
                "messages:write".into(),
                "events:read".into(),
            ],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap();
    let conversation = app
        .create_group(CreateGroupRequest {
            actor_id: operator.id.clone(),
            name: "Runtime Ops".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![human.id.clone(), agent.principal.id.clone()],
            workspace_id: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &operator).await;
    seed_principal_to_db(&database.database_url, &human).await;
    seed_principal_to_db(&database.database_url, &agent.principal).await;
    seed_conversation_to_db(&database.database_url, &conversation).await;
    let binding = runtime
        .create_binding(CreateBindingInput {
            conversation_id: conversation.id.clone(),
            agent_principal_id: agent.principal.id.clone(),
            driver_type: DriverType::ClaudePrint,
            workspace_path: "/worktrees/foo-claude".into(),
            git_worktree_path: Some("/worktrees/foo-claude".into()),
            config_json: json!({"allowed_tools": ["Read", "Edit"]}),
            audit_actor: Some(audit_actor(&operator)),
        })
        .await
        .unwrap();
    runtime
        .update_binding_cursors(&binding.id, 21, 13)
        .await
        .unwrap();
    // Seed session/thread IDs and an error message via direct SQL so the
    // list/detail endpoints have something to return.  Mirrors the kind of
    // UPDATE production code paths do against agent_runtime_bindings.
    {
        let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
            .await
            .expect("connect for binding seed");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        client
            .execute(
                "UPDATE agent_runtime_bindings
                 SET external_session_id = $2,
                     external_thread_id = $3,
                     last_error = $4,
                     state = 'error'
                 WHERE id = $1",
                &[
                    &binding.id,
                    &"sess-123",
                    &"thread-456",
                    &"driver failed with secret=sk-live-123 Bearer claude-secret",
                ],
            )
            .await
            .expect("seed binding session/error");
    }

    let router = runtime_router_with_db(app, runtime, &database.database_url);
    let list_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/runtime/bindings")
                .header(
                    "authorization",
                    format!("Bearer {}", session_token(&operator)),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list_response.status(), StatusCode::OK);
    let list_body = to_bytes(list_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let list_payload: Value = serde_json::from_slice(&list_body).unwrap();
    assert_eq!(list_payload[0]["agent_name"], "Claude Dev");
    assert_eq!(list_payload[0]["conversation_name"], "Runtime Ops");
    assert_eq!(list_payload[0]["driver_type"], "claude_print");
    assert_eq!(list_payload[0]["state"], "error");
    assert_eq!(list_payload[0]["workspace_path"], "/worktrees/foo-claude");
    assert_eq!(list_payload[0]["last_event_cursor"], 21);
    assert_eq!(list_payload[0]["last_acked_event_cursor"], 13);
    let list_error = list_payload[0]["last_error"].as_str().unwrap();
    assert!(list_error.contains("REDACTED"));
    assert!(!list_error.contains("sk-live-123"));
    assert!(!list_error.contains("claude-secret"));

    // A signed-in person in the same workspace has the same runtime view as
    // the legacy bootstrap account. Agent tokens remain excluded separately.
    let human_list_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/runtime/bindings")
                .header("authorization", format!("Bearer {}", session_token(&human)))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(human_list_response.status(), StatusCode::OK);
    let human_list_body = to_bytes(human_list_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let human_list_payload: Value = serde_json::from_slice(&human_list_body).unwrap();
    assert_eq!(human_list_payload[0]["id"], binding.id);

    let detail_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/runtime/bindings/{}", binding.id))
                .header("authorization", format!("Bearer {}", session_token(&human)))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(detail_response.status(), StatusCode::OK);
    let detail_body = to_bytes(detail_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let detail_payload: Value = serde_json::from_slice(&detail_body).unwrap();
    assert_eq!(detail_payload["external_session_id"], "sess-123");
    assert_eq!(detail_payload["external_thread_id"], "thread-456");
    assert_eq!(detail_payload["last_event_cursor"], 21);
    assert_eq!(detail_payload["last_acked_event_cursor"], 13);
    assert_eq!(detail_payload["last_seen_server_seq"], 0);
    let detail_error = detail_payload["last_error"].as_str().unwrap();
    assert!(detail_error.contains("REDACTED"));
    assert!(!detail_error.contains("sk-live-123"));
    assert!(!detail_error.contains("claude-secret"));
}

#[tokio::test]
async fn runtime_binding_actions_allow_workspace_humans_and_write_audit_entries() {
    let database = TestDatabase::create().await;
    let runtime = RuntimeStore::new(database.database_url.clone());
    let app = choruz_application::ChatApp::new();
    let auth = LocalAuthConfig::from_env();
    let operator = auth.ensure_operator_sync(&app).unwrap();
    let human = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: operator.workspace_id.clone(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    let agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Codex Reviewer".into(),
            scopes: vec![
                "messages:read".into(),
                "messages:write".into(),
                "events:read".into(),
            ],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap();
    let conversation = app
        .create_group(CreateGroupRequest {
            actor_id: operator.id.clone(),
            name: "Review Loop".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![human.id.clone(), agent.principal.id.clone()],
            workspace_id: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &operator).await;
    seed_principal_to_db(&database.database_url, &human).await;
    seed_principal_to_db(&database.database_url, &agent.principal).await;
    seed_conversation_to_db(&database.database_url, &conversation).await;
    let binding = runtime
        .create_binding(CreateBindingInput {
            conversation_id: conversation.id,
            agent_principal_id: agent.principal.id,
            driver_type: DriverType::CodexExec,
            workspace_path: "/worktrees/foo-codex".into(),
            git_worktree_path: Some("/worktrees/foo-codex".into()),
            config_json: json!({}),
            audit_actor: Some(audit_actor(&operator)),
        })
        .await
        .unwrap();

    let router = runtime_router_with_db(app, runtime.clone(), &database.database_url);
    let rebind = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/v1/runtime/bindings/{}/rebind", binding.id))
                .header("authorization", format!("Bearer {}", session_token(&human)))
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "workspace_path": "/worktrees/foo-codex-v2"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rebind.status(), StatusCode::OK);
    let rebind_body = to_bytes(rebind.into_body(), usize::MAX).await.unwrap();
    let rebind_payload: Value = serde_json::from_slice(&rebind_body).unwrap();
    assert_eq!(rebind_payload["workspace_path"], "/worktrees/foo-codex-v2");

    let updated = runtime.get_binding(&binding.id).await.unwrap();
    assert_eq!(updated.workspace_path, "/worktrees/foo-codex-v2");
    assert_eq!(updated.state.as_str(), "idle");

    let actions = database.audit_actions().await;
    assert_eq!(
        actions,
        vec!["runtime.binding_created", "runtime.binding_rebound",]
    );
}

#[cfg(unix)]
#[tokio::test]
async fn terminal_routes_do_not_write_conversation_events() {
    let _guard = api_test_env_lock().lock().await;
    let root = isolated_test_dir("terminal-no-events");
    let runtime_dir = root.join("runtime");
    let _runtime_env = EnvVarGuard::set_path("CHORUZ_RUNTIME_DIR", &runtime_dir);
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).expect("create workspace");
    let fake_cli = root.join("codex-wrapper-without-codex-name");
    let codex_home_seen = root.join("codex-home-seen.txt");
    write_executable_script(
        &fake_cli,
        &format!(
            "#!/bin/sh\nprintf '%s' \"$CODEX_HOME\" > '{}'\nsleep 5\n",
            codex_home_seen.display()
        ),
    );

    let database = TestDatabase::create().await;
    let runtime = RuntimeStore::new(database.database_url.clone());
    let app = choruz_application::ChatApp::new();
    let auth = LocalAuthConfig::from_env();
    let operator = auth.ensure_operator_sync(&app).unwrap();
    let human = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: operator.workspace_id.clone(),
            principal_type: PrincipalType::Human,
            name: "Terminal Human".into(),
            avatar_url: None,
        })
        .unwrap();
    let agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Codex Terminal".into(),
            scopes: vec![
                "messages:read".into(),
                "messages:write".into(),
                "events:read".into(),
            ],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap();
    let conversation = app
        .create_direct_conversation(CreateDirectConversationRequest {
            actor_id: human.id.clone(),
            peer_principal_id: agent.principal.id.clone(),
            workspace_id: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &operator).await;
    seed_principal_to_db(&database.database_url, &human).await;
    seed_principal_to_db(&database.database_url, &agent.principal).await;
    seed_conversation_to_db(&database.database_url, &conversation).await;

    let binding = runtime
        .create_binding(CreateBindingInput {
            conversation_id: conversation.id.clone(),
            agent_principal_id: agent.principal.id.clone(),
            driver_type: DriverType::CodexTerminal,
            workspace_path: workspace.to_string_lossy().to_string(),
            git_worktree_path: None,
            config_json: json!({ "binary_path": fake_cli.to_string_lossy() }),
            audit_actor: Some(audit_actor(&operator)),
        })
        .await
        .expect("create terminal binding");
    let router = runtime_router_with_db(app, runtime, &database.database_url);

    let ensure = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/v1/terminals/{}/ensure", binding.id))
                .header("authorization", format!("Bearer {}", session_token(&human)))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ensure.status(), StatusCode::OK);

    let input = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/v1/terminals/{}/input", binding.id))
                .header("authorization", format!("Bearer {}", session_token(&human)))
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "data": "hello terminal" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(input.status(), StatusCode::OK);

    for _ in 0..20 {
        if fs::read_to_string(&codex_home_seen).is_ok_and(|value| value.contains("/codex-homes/")) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let seen_home = fs::read_to_string(&codex_home_seen).expect("fake CLI saw CODEX_HOME");
    assert!(
        seen_home.contains("/codex-homes/"),
        "Codex terminal wrapper should receive managed CODEX_HOME"
    );

    assert_eq!(
        conversation_event_count(&database.database_url, &conversation.id).await,
        0,
        "terminal ensure/input must not persist transcript or preview events"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn codex_disconnect_cleanup_captures_binding_local_jsonl_before_drop() {
    let root = isolated_test_dir("codex-cleanup-capture");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).expect("create workspace");

    let database = TestDatabase::create().await;
    let runtime = RuntimeStore::new(database.database_url.clone());
    let app = choruz_application::ChatApp::new();
    let auth = LocalAuthConfig::from_env();
    let operator = auth.ensure_operator_sync(&app).unwrap();
    let agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Codex Capture".into(),
            scopes: vec![
                "messages:read".into(),
                "messages:write".into(),
                "events:read".into(),
            ],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap();
    let conversation = app
        .create_direct_conversation(CreateDirectConversationRequest {
            actor_id: operator.id.clone(),
            peer_principal_id: agent.principal.id.clone(),
            workspace_id: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &operator).await;
    seed_principal_to_db(&database.database_url, &agent.principal).await;
    seed_conversation_to_db(&database.database_url, &conversation).await;

    let binding = runtime
        .create_binding(CreateBindingInput {
            conversation_id: conversation.id.clone(),
            agent_principal_id: agent.principal.id.clone(),
            driver_type: DriverType::CodexTerminal,
            workspace_path: workspace.to_string_lossy().to_string(),
            git_worktree_path: None,
            config_json: json!({ "binary_path": "codex" }),
            audit_actor: Some(audit_actor(&operator)),
        })
        .await
        .expect("create binding");

    let managed_home = root.join("codex-home");
    let sessions = managed_home.join("sessions");
    fs::create_dir_all(&sessions).expect("create managed sessions");
    let prepared = runtime
        .begin_codex_terminal_capture(
            &binding.id,
            CodexTerminalCaptureInput {
                binding_id: binding.id.clone(),
                conversation_id: binding.conversation_id.clone(),
                agent_principal_id: binding.agent_principal_id.clone(),
                company_id: conversation.workspace_id.clone(),
                driver_type: binding.driver_type.as_str().into(),
                workspace_id: agent.principal.workspace_id.clone(),
                workspace_path: binding.workspace_path.clone(),
                native_home_path: fs::canonicalize(&managed_home)
                    .unwrap()
                    .to_string_lossy()
                    .to_string(),
                sessions_path: fs::canonicalize(&sessions)
                    .unwrap()
                    .to_string_lossy()
                    .to_string(),
                spawn_started_at: chrono::Utc::now(),
                baseline_session_files: vec![],
                binding_updated_at: binding.updated_at,
            },
        )
        .await
        .expect("begin capture");

    let session_file = sessions.join("2026/05/29/session.jsonl");
    fs::create_dir_all(session_file.parent().unwrap()).expect("create session dir");
    fs::write(
        &session_file,
        format!(
            r#"{{"type":"session_meta","payload":{{"id":"session-cleanup","cwd":"{}"}}}}"#,
            workspace.to_string_lossy()
        ) + "\n",
    )
    .expect("write session jsonl");

    let captured = crate::handlers_terminals::capture_codex_terminal_before_cleanup(
        &crate::host_runtime::RuntimeHost::Local(crate::host_runtime::LocalHost::new()),
        &runtime,
        &binding.id,
        prepared,
    )
    .await
    .expect("cleanup capture")
    .expect("captured anchor");

    assert_eq!(
        captured.valid_terminal_session_id_for_workspace(Some(&agent.principal.workspace_id)),
        Some("session-cleanup".into())
    );
    assert!(captured.config_json.get("terminal_capture").is_none());
}

#[cfg(unix)]
#[tokio::test]
async fn codex_terminal_open_reconciles_capture_metadata_after_gateway_restart_window() {
    let _guard = api_test_env_lock().lock().await;
    let root = isolated_test_dir("codex-restart-reconcile");
    let runtime_dir = root.join("runtime");
    let _runtime_env = EnvVarGuard::set_path("CHORUZ_RUNTIME_DIR", &runtime_dir);
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).expect("create workspace");
    let fake_cli = root.join("codex-wrapper-without-codex-name");
    write_executable_script(&fake_cli, "#!/bin/sh\nsleep 5\n");

    let database = TestDatabase::create().await;
    let runtime = RuntimeStore::new(database.database_url.clone());
    let app = choruz_application::ChatApp::new();
    let auth = LocalAuthConfig::from_env();
    let operator = auth.ensure_operator_sync(&app).unwrap();
    let agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Codex Restart".into(),
            scopes: vec![
                "messages:read".into(),
                "messages:write".into(),
                "events:read".into(),
            ],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap();
    let conversation = app
        .create_direct_conversation(CreateDirectConversationRequest {
            actor_id: operator.id.clone(),
            peer_principal_id: agent.principal.id.clone(),
            workspace_id: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &operator).await;
    seed_principal_to_db(&database.database_url, &agent.principal).await;
    seed_conversation_to_db(&database.database_url, &conversation).await;

    let binding = runtime
        .create_binding(CreateBindingInput {
            conversation_id: conversation.id.clone(),
            agent_principal_id: agent.principal.id.clone(),
            driver_type: DriverType::CodexTerminal,
            workspace_path: workspace.to_string_lossy().to_string(),
            git_worktree_path: None,
            config_json: json!({ "binary_path": fake_cli.to_string_lossy() }),
            audit_actor: Some(audit_actor(&operator)),
        })
        .await
        .expect("create binding");

    let managed_home = runtime_dir.join("codex-homes").join(&binding.id);
    let sessions = managed_home.join("sessions");
    fs::create_dir_all(&sessions).expect("create managed sessions");
    let prepared = runtime
        .begin_codex_terminal_capture(
            &binding.id,
            CodexTerminalCaptureInput {
                binding_id: binding.id.clone(),
                conversation_id: binding.conversation_id.clone(),
                agent_principal_id: binding.agent_principal_id.clone(),
                company_id: conversation.workspace_id.clone(),
                driver_type: binding.driver_type.as_str().into(),
                workspace_id: agent.principal.workspace_id.clone(),
                workspace_path: binding.workspace_path.clone(),
                native_home_path: fs::canonicalize(&managed_home)
                    .unwrap()
                    .to_string_lossy()
                    .to_string(),
                sessions_path: fs::canonicalize(&sessions)
                    .unwrap()
                    .to_string_lossy()
                    .to_string(),
                spawn_started_at: chrono::Utc::now(),
                baseline_session_files: vec![],
                binding_updated_at: binding.updated_at,
            },
        )
        .await
        .expect("begin capture");

    let session_file = sessions.join("2026/05/29/session.jsonl");
    fs::create_dir_all(session_file.parent().unwrap()).expect("create session dir");
    fs::write(
        &session_file,
        format!(
            r#"{{"type":"session_meta","payload":{{"id":"session-restart","cwd":"{}"}}}}"#,
            workspace.to_string_lossy()
        ) + "\n",
    )
    .expect("write session jsonl");
    assert!(prepared.terminal_session_anchor().is_none());

    let router = runtime_router_with_db(app, runtime.clone(), &database.database_url);
    let ensure = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/v1/terminals/{}/ensure", binding.id))
                .header(
                    "authorization",
                    format!("Bearer {}", session_token(&operator)),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ensure.status(), StatusCode::OK);

    let updated = runtime
        .get_binding(&binding.id)
        .await
        .expect("reload binding");
    let anchor = updated.terminal_session_anchor().expect("terminal anchor");
    assert_eq!(anchor.session_id, "session-restart");
    assert_eq!(
        anchor.native_home_path,
        fs::canonicalize(managed_home)
            .unwrap()
            .to_string_lossy()
            .to_string()
    );
    assert_eq!(
        conversation_event_count(&database.database_url, &conversation.id).await,
        0,
        "restart reconciliation must not write transcript events"
    );
}

#[tokio::test]
async fn runtime_policy_fields_round_trip_and_validate_inputs() {
    let database = TestDatabase::create().await;
    let runtime = RuntimeStore::new(database.database_url.clone());
    let app = choruz_application::ChatApp::new();
    let auth = LocalAuthConfig::from_env();
    let operator = auth.ensure_operator_sync(&app).unwrap();
    let human = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: operator.workspace_id.clone(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    let coordinator = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "AI Manager".into(),
            scopes: vec![
                "messages:read".into(),
                "messages:write".into(),
                "events:read".into(),
            ],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap();
    let conversation = app
        .create_group(CreateGroupRequest {
            actor_id: operator.id.clone(),
            name: "Hybrid Routing".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![human.id.clone(), coordinator.principal.id.clone()],
            workspace_id: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &operator).await;
    seed_principal_to_db(&database.database_url, &human).await;
    seed_principal_to_db(&database.database_url, &coordinator.principal).await;
    seed_conversation_to_db(&database.database_url, &conversation).await;
    let router = runtime_router_with_db(app, runtime.clone(), &database.database_url);
    let token = session_token(&operator);

    let defaults = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/runtime/policies/{}", conversation.id))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(defaults.status(), StatusCode::OK);
    let defaults_body = to_bytes(defaults.into_body(), usize::MAX).await.unwrap();
    let defaults_payload: Value = serde_json::from_slice(&defaults_body).unwrap();
    assert_eq!(defaults_payload["auto_mode"], "mentioned_only");
    assert_eq!(defaults_payload["untagged_human_mode"], "mentioned_only");
    assert_eq!(
        defaults_payload["default_coordinator_agent_id"],
        Value::Null
    );

    let upsert = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri(format!("/v1/runtime/policies/{}", conversation.id))
                .header("authorization", format!("Bearer {token}"))
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "auto_mode": "metadata_only",
                        "allow_agent_to_agent": true,
                        "default_coordinator_agent_id": coordinator.principal.id,
                        "untagged_human_mode": "coordinator_only",
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(upsert.status(), StatusCode::OK);
    let upsert_body = to_bytes(upsert.into_body(), usize::MAX).await.unwrap();
    let upsert_payload: Value = serde_json::from_slice(&upsert_body).unwrap();
    assert_eq!(upsert_payload["auto_mode"], "metadata_only");
    assert_eq!(upsert_payload["allow_agent_to_agent"], true);
    assert_eq!(
        upsert_payload["default_coordinator_agent_id"].as_str(),
        Some(coordinator.principal.id.as_str())
    );
    assert_eq!(upsert_payload["untagged_human_mode"], "coordinator_only");

    let stored = runtime.get_policy(&conversation.id).await.unwrap();
    assert_eq!(
        stored.default_coordinator_agent_id.as_deref(),
        Some(coordinator.principal.id.as_str())
    );
    assert_eq!(stored.untagged_human_mode.as_str(), "coordinator_only");

    let bad_mode = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri(format!("/v1/runtime/policies/{}", conversation.id))
                .header("authorization", format!("Bearer {token}"))
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "untagged_human_mode": "wake_everyone",
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bad_mode.status(), StatusCode::BAD_REQUEST);

    let human_coordinator = router
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri(format!("/v1/runtime/policies/{}", conversation.id))
                .header("authorization", format!("Bearer {token}"))
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "default_coordinator_agent_id": human.id,
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(human_coordinator.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn runtime_status_api_allows_workspace_humans_and_redacts_errors() {
    let database = TestDatabase::create().await;
    let runtime = RuntimeStore::new(database.database_url.clone());
    let session_store = PgSessionStore::new(&database.database_url);
    let app = choruz_application::ChatApp::new();
    let auth = LocalAuthConfig::from_env();
    let operator = auth.ensure_operator_sync(&app).unwrap();
    let human = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: operator.workspace_id.clone(),
            principal_type: PrincipalType::Human,
            name: "Runtime Human".into(),
            avatar_url: None,
        })
        .unwrap();
    let busy_agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Busy Agent".into(),
            scopes: vec![
                "messages:read".into(),
                "messages:write".into(),
                "events:read".into(),
            ],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap();
    let idle_agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Idle Agent".into(),
            scopes: vec![
                "messages:read".into(),
                "messages:write".into(),
                "events:read".into(),
            ],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap();
    let queued_agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Queued Agent".into(),
            scopes: vec![
                "messages:read".into(),
                "messages:write".into(),
                "events:read".into(),
            ],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap();
    let conversation = app
        .create_group(CreateGroupRequest {
            actor_id: operator.id.clone(),
            name: "Runtime Status Group".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![
                human.id.clone(),
                busy_agent.principal.id.clone(),
                idle_agent.principal.id.clone(),
                queued_agent.principal.id.clone(),
            ],
            workspace_id: None,
        })
        .unwrap();
    let other_admin = choruz_domain::Principal {
        id: "runtime-status-other-operator".into(),
        workspace_id: "runtime-status-other-workspace".into(),
        principal_type: PrincipalType::Human,
        name: "Other Runtime Operator".into(),
        avatar_url: None,
        scopes: vec!["operator".into()],
        secret_hash: None,
        disabled: false,
        deleted_at: None,
        channel_visibility: choruz_domain::ChannelVisibility::Visible,
        created_at: choruz_common::now(),
        updated_at: choruz_common::now(),
        user_id: None,
    };

    seed_principal_to_db(&database.database_url, &operator).await;
    seed_principal_to_db(&database.database_url, &human).await;
    seed_principal_to_db(&database.database_url, &busy_agent.principal).await;
    seed_principal_to_db(&database.database_url, &idle_agent.principal).await;
    seed_principal_to_db(&database.database_url, &queued_agent.principal).await;
    seed_principal_to_db(&database.database_url, &other_admin).await;
    seed_conversation_to_db(&database.database_url, &conversation).await;

    let busy_session_key = format!("{}:{}", busy_agent.principal.id, conversation.id);
    session_store
        .upsert_session(
            &busy_session_key,
            &busy_agent.principal.id,
            &conversation.id,
        )
        .await
        .unwrap();
    let busy_active = session_store
        .insert_command(&InsertCommand {
            command_id: choruz_common::new_id(),
            route_id: choruz_common::new_id(),
            session_key: busy_session_key.clone(),
            agent_id: busy_agent.principal.id.clone(),
            conversation_id: conversation.id.clone(),
            message_id: choruz_common::new_id(),
            turn_id: choruz_common::new_id(),
            prompt: "active prompt".into(),
            max_attempts: 5,
            metadata: json!({}),
        })
        .await
        .unwrap();
    session_store
        .assign_lease(&busy_active.command_id, "runtime-status-test-node")
        .await
        .unwrap();
    session_store
        .update_command_status(&CommandStatusUpdate {
            command_id: busy_active.command_id.clone(),
            status: CommandStatus::Started,
            last_error: Some("driver failed with secret=sk-live-123 Bearer runtime-secret".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    for prompt in ["queued behind active 1", "queued behind active 2"] {
        session_store
            .insert_command(&InsertCommand {
                command_id: choruz_common::new_id(),
                route_id: choruz_common::new_id(),
                session_key: busy_session_key.clone(),
                agent_id: busy_agent.principal.id.clone(),
                conversation_id: conversation.id.clone(),
                message_id: choruz_common::new_id(),
                turn_id: choruz_common::new_id(),
                prompt: prompt.into(),
                max_attempts: 5,
                metadata: json!({}),
            })
            .await
            .unwrap();
    }

    let queued_session_key = format!("{}:{}", queued_agent.principal.id, conversation.id);
    session_store
        .upsert_session(
            &queued_session_key,
            &queued_agent.principal.id,
            &conversation.id,
        )
        .await
        .unwrap();
    session_store
        .insert_command(&InsertCommand {
            command_id: choruz_common::new_id(),
            route_id: choruz_common::new_id(),
            session_key: queued_session_key,
            agent_id: queued_agent.principal.id.clone(),
            conversation_id: conversation.id.clone(),
            message_id: choruz_common::new_id(),
            turn_id: choruz_common::new_id(),
            prompt: "queued only".into(),
            max_attempts: 5,
            metadata: json!({}),
        })
        .await
        .unwrap();

    let router = runtime_router_with_db(app, runtime, &database.database_url);
    let uri = format!("/v1/conversations/{}/runtime-status", conversation.id);
    let (human_status, _) =
        api_json_request(router.clone(), &human, Method::GET, uri.clone()).await;
    assert_eq!(human_status, StatusCode::OK);

    let (status, payload) =
        api_json_request(router.clone(), &operator, Method::GET, uri.clone()).await;
    assert_eq!(status, StatusCode::OK);
    let rows = payload.as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|row| row["agent_principal_id"] != human.id));
    let busy = rows
        .iter()
        .find(|row| row["agent_principal_id"] == busy_agent.principal.id)
        .unwrap();
    assert_eq!(busy["agent_name"], "Busy Agent");
    assert_eq!(busy["status"], "busy");
    assert_eq!(busy["queued_count"], 2);
    assert_eq!(busy["active_command"]["status"], "started");
    assert_eq!(busy["active_command"]["attempt_count"], 1);
    assert!(
        busy["active_command"]["lease_age_seconds"]
            .as_i64()
            .is_some_and(|age| age >= 0)
    );
    let active_error = busy["active_command"]["last_error"].as_str().unwrap();
    assert!(active_error.contains("REDACTED"));
    assert!(!active_error.contains("sk-live-123"));
    assert!(!active_error.contains("runtime-secret"));
    let row_error = busy["last_error"].as_str().unwrap();
    assert!(row_error.contains("REDACTED"));
    assert!(!row_error.contains("sk-live-123"));

    let queued = rows
        .iter()
        .find(|row| row["agent_principal_id"] == queued_agent.principal.id)
        .unwrap();
    assert_eq!(queued["agent_name"], "Queued Agent");
    assert_eq!(queued["status"], "queued");
    assert_eq!(queued["queued_count"], 1);
    assert_eq!(queued["active_command"], Value::Null);

    let idle = rows
        .iter()
        .find(|row| row["agent_principal_id"] == idle_agent.principal.id)
        .unwrap();
    assert_eq!(idle["agent_name"], "Idle Agent");
    assert_eq!(idle["status"], "idle");
    assert_eq!(idle["queued_count"], 0);
    assert_eq!(idle["active_command"], Value::Null);

    let (other_admin_status, _) = api_json_request(router, &other_admin, Method::GET, uri).await;
    assert_eq!(other_admin_status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn gateway_restores_tls_without_a_pairing_request() {
    const CHILD: &str = "CHORUZ_TEST_FRESH_TLS_PROCESS";
    if std::env::var_os(CHILD).is_some() {
        assert!(rustls::crypto::CryptoProvider::get_default().is_none());
        let _router = crate::router(choruz_application::ChatApp::new());
        assert!(rustls::crypto::CryptoProvider::get_default().is_some());
        let _config = rustls::ClientConfig::builder();
        return;
    }
    // TLS selection is process-global; a subprocess proves cold-start behavior
    // without borrowing another parallel test's pairing initialization.
    let output = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tests::runtime::gateway_restores_tls_without_a_pairing_request",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .kill_on_drop(true)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "cold-start TLS regression: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[tokio::test]
async fn remote_control_pairing_redeem_and_revoke_round_trip() {
    let (gateway_url, mut gateway_events) = spawn_pairing_gateway().await;
    let _env = ChannelTaskEnvGuard::remote_control_with_gateway(&gateway_url);
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let operator = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "remote-control-workspace".into(),
            principal_type: PrincipalType::Human,
            name: "Remote Operator".into(),
            avatar_url: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &operator).await;
    let router = router_with_db(app, &database.database_url);

    let (settings_status, settings) = api_json_request(
        router.clone(),
        &operator,
        Method::GET,
        "/v1/remote-control/settings".into(),
    )
    .await;
    assert_eq!(settings_status, StatusCode::OK);
    assert!(settings.get("data_mode").is_none());

    let (pairing_status, pairing) = api_json_payload_request(
        router.clone(),
        &operator,
        Method::POST,
        "/v1/remote-control/pairings".into(),
        Value::Null,
    )
    .await;
    assert_eq!(pairing_status, StatusCode::OK);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), gateway_events.recv())
            .await
            .unwrap(),
        Some(PairingGatewayEvent::Connected)
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(50), gateway_events.recv())
            .await
            .is_err(),
        "the API-owned host socket must outlive the credential request"
    );
    let credential = pairing["credential"].as_str().unwrap();
    assert!(credential.starts_with("v1."));
    assert_eq!(credential.len(), 48);
    let session_key = "a".repeat(43);
    let unauthenticated = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/remote-control/pairings/redeem")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "credential": credential,
                        "device_name": "Untrusted browser",
                        "session_key": session_key,
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

    let (redeem_status, redeemed) = api_json_payload_request(
        router.clone(),
        &operator,
        Method::POST,
        "/v1/remote-control/pairings/redeem".into(),
        json!({
            "credential": credential,
            "device_name": "Test browser",
            "session_key": session_key,
        }),
    )
    .await;
    assert_eq!(redeem_status, StatusCode::OK);
    let device_id = redeemed["device_id"].as_str().unwrap();
    let (db_client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .unwrap();
    tokio::spawn(async move { connection.await.unwrap() });
    let stored_key: String = db_client
        .query_one(
            "SELECT session_key_wrapped FROM remote_control_device WHERE id = $1",
            &[&device_id],
        )
        .await
        .unwrap()
        .get(0);
    assert!(stored_key.starts_with("v1."));
    assert!(!stored_key.contains(&session_key));

    let (bridge_status, bridge) = api_json_request(
        router.clone(),
        &operator,
        Method::GET,
        "/v1/remote-control/bridge-config".into(),
    )
    .await;
    assert_eq!(bridge_status, StatusCode::OK);
    assert_eq!(bridge["session_key"], session_key);
    assert_eq!(bridge["revoked_device_ids"], json!([]));
    let first_room = bridge["transport_session_id"]
        .as_str()
        .expect("transport room");
    let (next_status, next_bridge) = api_json_request(
        router.clone(),
        &operator,
        Method::GET,
        "/v1/remote-control/bridge-config".into(),
    )
    .await;
    assert_eq!(next_status, StatusCode::OK);
    let next_room = next_bridge["transport_session_id"]
        .as_str()
        .expect("next transport room");
    assert_ne!(first_room, next_room);
    assert!(!first_room.is_empty());
    assert_ne!(first_room, device_id);
    assert_eq!(next_bridge["session_key"], session_key);

    let (revoke_status, _) = api_json_payload_request(
        router.clone(),
        &operator,
        Method::DELETE,
        format!("/v1/remote-control/devices/{device_id}"),
        Value::Null,
    )
    .await;
    assert_eq!(revoke_status, StatusCode::OK);

    let (after_status, after) = api_json_request(
        router,
        &operator,
        Method::GET,
        "/v1/remote-control/bridge-config".into(),
    )
    .await;
    assert_eq!(after_status, StatusCode::OK);
    assert!(after["session_key"].is_null());
    assert_eq!(after["revoked_device_ids"], json!([device_id]));
}

#[tokio::test]
async fn runtime_host_pairing_is_single_use_and_host_token_is_revocable() {
    let _env = ChannelTaskEnvGuard::remote_control();
    let _runtime_guard = api_test_env_lock().lock().await;
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let operator = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "runtime-host-company".into(),
            principal_type: PrincipalType::Human,
            name: "Runtime Host Operator".into(),
            avatar_url: None,
        })
        .unwrap();
    let agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Remote Builder".into(),
            scopes: vec!["messages:read".into(), "messages:write".into()],
            workspace_id: Some(operator.workspace_id.clone()),
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let conversation = app
        .create_group(CreateGroupRequest {
            actor_id: operator.id.clone(),
            name: "Cross-host build".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![agent.id.clone()],
            workspace_id: Some(operator.workspace_id.clone()),
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &operator).await;
    seed_principal_to_db(&database.database_url, &agent).await;
    seed_conversation_to_db(&database.database_url, &conversation).await;
    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .unwrap();
    tokio::spawn(async move { connection.await.unwrap() });
    client
        .execute(
            "INSERT INTO company (id, name, slug, owner_id) VALUES ($1, 'Runtime Hosts', $1, $2)",
            &[&operator.workspace_id, &operator.id],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO company_member (company_id, principal_id)
             VALUES ($1, $2)",
            &[&operator.workspace_id, &operator.id],
        )
        .await
        .unwrap();
    let router = router_with_db(app, &database.database_url);

    let (pairing_status, pairing) = api_json_payload_request(
        router.clone(),
        &operator,
        Method::POST,
        format!(
            "/v1/companies/{}/runtime-host-pairings",
            operator.workspace_id
        ),
        Value::Null,
    )
    .await;
    assert_eq!(pairing_status, StatusCode::CREATED);
    let code = pairing["code"].as_str().unwrap();
    assert_eq!(code.len(), 8);
    assert!(code.bytes().all(|byte| byte.is_ascii_digit()));

    let redeem = |name: &str| {
        let mut request = Request::builder()
            .method(Method::POST)
            .uri("/v1/runtime-host-pairings/redeem")
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({ "code": code, "name": name }).to_string(),
            ))
            .unwrap();
        request.extensions_mut().insert(axum::extract::ConnectInfo(
            "127.0.0.1:41000".parse::<std::net::SocketAddr>().unwrap(),
        ));
        request
    };
    let invalid_name = router
        .clone()
        .oneshot(redeem("Build\nServer"))
        .await
        .unwrap();
    assert_eq!(invalid_name.status(), StatusCode::BAD_REQUEST);
    let response = router
        .clone()
        .oneshot(redeem("Build Server West"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    let host_id = body["host"]["id"].as_str().unwrap().to_owned();
    let host_token = body["host_token"].as_str().unwrap().to_owned();
    assert!(!host_token.is_empty());

    let replay = router
        .clone()
        .oneshot(redeem("Replay Server"))
        .await
        .unwrap();
    assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);

    let heartbeat = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/v1/runtime-hosts/{host_id}/heartbeat"))
                .header("x-choruz-host-token", &host_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(heartbeat.status(), StatusCode::NO_CONTENT);

    let (list_status, hosts) = api_json_request(
        router.clone(),
        &operator,
        Method::GET,
        format!("/v1/companies/{}/runtime-hosts", operator.workspace_id),
    )
    .await;
    assert_eq!(list_status, StatusCode::OK);
    assert_eq!(hosts[0]["name"], "Build Server West");
    assert_eq!(hosts[0]["status"], "online");

    // A device answers the dashboard's requests over its host link, which
    // it dials as a WebSocket carrying the host token in its first frame.
    let base_url = serve_router(router.clone()).await;
    let rejected = connect_host_link(&base_url, &host_id, "not-the-token").await;
    assert!(
        rejected.is_err(),
        "a wrong host token must not open a link: {rejected:?}"
    );
    let mut device = connect_host_link(&base_url, &host_id, &host_token)
        .await
        .expect("open the host link");
    let device_answers = tokio::spawn(async move {
        while let Some(Ok(frame)) = device.next().await {
            let Message::Text(text) = frame else { continue };
            let call: Value = serde_json::from_str(&text).unwrap();
            if call["kind"] != "call" {
                continue;
            }
            let reply = match (
                call["request"]["call"].as_str(),
                call["request"]["request"]["op"].as_str(),
            ) {
                (Some("host"), Some("filesystem_list")) => json!({
                    "kind": "result",
                    "id": call["id"],
                    "ok": {
                        "path": call["request"]["request"]["path"],
                        "parent": "/srv",
                        "entries": [{"name": "app", "type": "directory", "path": "/srv/projects/app"}]
                    }
                }),
                (Some("host"), Some("scan_sessions")) => json!({
                    "kind": "result",
                    "id": call["id"],
                    "ok": {
                        "workspace_path": "/srv/projects",
                        "sessions": [{
                            "harness": "claude",
                            "native_session_id": "remote-claude-session",
                            "title": "Remote Claude work",
                            "workspace_path": "/srv/projects/app",
                            "updated_at": "2026-09-03T12:00:00Z",
                            "model": "claude-sonnet-4-5",
                            "branch": "main",
                            "archived": false
                        }],
                        "warnings": []
                    }
                }),
                (Some("host"), Some("ensure_outbox_helper")) => json!({
                    "kind": "result",
                    "id": call["id"],
                }),
                other => json!({
                    "kind": "result",
                    "id": call["id"],
                    "error": {"kind": "validation", "message": format!("unexpected call {other:?}")}
                }),
            };
            device
                .send(Message::Text(reply.to_string().into()))
                .await
                .unwrap();
        }
    });

    let (operation_status, operation_result) = api_json_payload_request(
        router.clone(),
        &operator,
        Method::POST,
        format!("/v1/runtime-hosts/{host_id}/operations"),
        json!({
            "kind": "filesystem.list",
            "request": { "path": "/srv/projects", "include_files": false }
        }),
    )
    .await;
    assert_eq!(operation_status, StatusCode::OK, "{operation_result}");
    assert_eq!(operation_result["path"], "/srv/projects");
    assert_eq!(operation_result["entries"][0]["name"], "app");

    let (import_status, imported) = api_json_payload_request(
        router.clone(),
        &operator,
        Method::POST,
        "/v1/workspace-sessions/import".into(),
        json!({
            "company_id": operator.workspace_id,
            "runtime_host_id": host_id,
            "workspace_path": "/srv/projects",
            "sessions": [{
                "harness": "claude",
                "native_session_id": "remote-claude-session",
                "workspace_path": "/srv/projects/app"
            }]
        }),
    )
    .await;
    assert_eq!(
        import_status,
        StatusCode::OK,
        "remote import failed: {imported}"
    );
    let imported_binding_id = imported["imported"][0]["binding_id"].as_str().unwrap();
    let row = client
        .query_one(
            "SELECT config_json, runtime_host_id FROM agent_runtime_bindings b
             JOIN native_session_import n ON n.binding_id = b.id
             WHERE b.id = $1",
            &[&imported_binding_id],
        )
        .await
        .unwrap();
    let config: Value = row.get("config_json");
    assert_eq!(config["runtime_host_id"], host_id);
    assert_eq!(
        config["interaction_mode"], "terminal",
        "an imported session on a remote device is a terminal DM like a local one"
    );
    device_answers.abort();
    let (offline_status, offline) = api_json_payload_request(
        router.clone(),
        &operator,
        Method::POST,
        format!("/v1/runtime-hosts/{host_id}/operations"),
        json!({ "kind": "filesystem.home", "request": {} }),
    )
    .await;
    assert_eq!(offline_status, StatusCode::CONFLICT, "{offline}");
    assert_eq!(
        row.get::<_, Option<String>>("runtime_host_id").as_deref(),
        Some(host_id.as_str())
    );

    let (invalid_binding_status, _) = api_json_payload_request(
        router.clone(),
        &operator,
        Method::POST,
        "/v1/runtime/bindings".into(),
        json!({
            "conversation_id": conversation.id,
            "agent_principal_id": agent.id,
            "driver_type": "codex_terminal",
            "workspace_path": "/srv/runtime-host-project",
            "config_json": { "runtime_host_id": "host-from-another-company" }
        }),
    )
    .await;
    assert_eq!(invalid_binding_status, StatusCode::BAD_REQUEST);

    let (binding_status, binding) = api_json_payload_request(
        router.clone(),
        &operator,
        Method::POST,
        "/v1/runtime/bindings".into(),
        json!({
            "conversation_id": conversation.id,
            "agent_principal_id": agent.id,
            "driver_type": "codex_terminal",
            "workspace_path": "/srv/runtime-host-project",
            "config_json": { "runtime_host_id": format!(" {host_id} ") }
        }),
    )
    .await;
    assert_eq!(binding_status, StatusCode::CREATED);
    assert_eq!(binding["runtime_host_id"], host_id);

    let sessions = PgSessionStore::new(&database.database_url);
    let session_key = format!("{}:{}", agent.id, conversation.id);
    sessions
        .upsert_session(&session_key, &agent.id, &conversation.id)
        .await
        .unwrap();
    let command = sessions
        .insert_command(&InsertCommand {
            command_id: Uuid::now_v7().to_string(),
            route_id: Uuid::now_v7().to_string(),
            session_key,
            agent_id: agent.id.clone(),
            conversation_id: conversation.id.clone(),
            message_id: Uuid::now_v7().to_string(),
            turn_id: Uuid::now_v7().to_string(),
            prompt: "Implement the runtime-host test".into(),
            max_attempts: 3,
            metadata: json!({}),
        })
        .await
        .unwrap();
    assert_eq!(command.metadata["runtime_host_id"], host_id);
    assert!(
        sessions
            .find_pending_commands(100)
            .await
            .unwrap()
            .iter()
            .all(|candidate| candidate.command_id != command.command_id),
        "the local dispatcher must not execute a remote-host command"
    );

    let claim_request = || {
        Request::builder()
            .method(Method::POST)
            .uri(format!("/v1/runtime-hosts/{host_id}/commands/claim"))
            .header("x-choruz-host-token", &host_token)
            .body(Body::empty())
            .unwrap()
    };
    let (first_claim, second_claim) = tokio::join!(
        router.clone().oneshot(claim_request()),
        router.clone().oneshot(claim_request()),
    );
    let first_claim: Value = serde_json::from_slice(
        &to_bytes(first_claim.unwrap().into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    let second_claim: Value = serde_json::from_slice(
        &to_bytes(second_claim.unwrap().into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    let claimed = match (first_claim.is_null(), second_claim.is_null()) {
        (false, true) => first_claim,
        (true, false) => second_claim,
        state => panic!("exactly one concurrent claim must win, got {state:?}"),
    };
    assert_eq!(claimed["command_id"], command.command_id);
    assert_eq!(claimed["driver_type"], "codex_terminal");
    assert_eq!(claimed["workspace_path"], "/srv/runtime-host-project");
    assert!(claimed["model"].is_null());
    assert!(claimed["external_session_id"].is_null());
    let attempt_id = claimed["attempt_id"].as_str().unwrap();

    let omitted_success = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!(
                    "/v1/runtime-hosts/{host_id}/commands/{}/complete",
                    command.command_id
                ))
                .header("x-choruz-host-token", &host_token)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "attempt_id": attempt_id }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(omitted_success.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let command_heartbeat = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!(
                    "/v1/runtime-hosts/{host_id}/commands/{}/heartbeat",
                    command.command_id
                ))
                .header("x-choruz-host-token", &host_token)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "attempt_id": attempt_id }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(command_heartbeat.status(), StatusCode::NO_CONTENT);

    let completed = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!(
                    "/v1/runtime-hosts/{host_id}/commands/{}/complete",
                    command.command_id
                ))
                .header("x-choruz-host-token", &host_token)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "attempt_id": attempt_id,
                        "succeeded": true,
                        "contents": [
                            "Implemented on the west build server",
                            "The remote verification also passed"
                        ],
                        "execution_duration_ms": 42,
                        "external_session_id": "codex-thread-west"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let completed_status = completed.status();
    let completed_body = to_bytes(completed.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        completed_status,
        StatusCode::NO_CONTENT,
        "{}",
        String::from_utf8_lossy(&completed_body)
    );
    let event = client
        .query_one(
            "SELECT content, metadata, reply_event_id FROM conversation_events WHERE turn_id = $1",
            &[&command.turn_id],
        )
        .await
        .unwrap();
    assert_eq!(
        event.get::<_, String>("content"),
        "Implemented on the west build server"
    );
    let metadata: Value = event.get("metadata");
    assert_eq!(metadata["runtime_host_id"], host_id);
    assert_eq!(metadata["runtime_host_name"], "Build Server West");
    let outbox_count: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM event_outbox
             WHERE aggregate_id = $1 AND payload->>'message_id' = $2",
            &[&conversation.id, &event.get::<_, String>("reply_event_id")],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(outbox_count, 1);
    let additional_reply_count: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM conversation_events
             WHERE metadata->>'command_id' = $1",
            &[&command.command_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(additional_reply_count, 2);
    let additional_outbox_count: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM event_outbox
             WHERE payload->'metadata'->>'command_id' = $1",
            &[&command.command_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(additional_outbox_count, 2);
    let persisted_session: Option<String> = client
        .query_one(
            "SELECT external_session_id FROM agent_runtime_bindings WHERE id = $1",
            &[&binding["id"].as_str().unwrap()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(persisted_session.as_deref(), Some("codex-thread-west"));

    let stale_completion = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!(
                    "/v1/runtime-hosts/{host_id}/commands/{}/complete",
                    command.command_id
                ))
                .header("x-choruz-host-token", &host_token)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "attempt_id": attempt_id, "succeeded": true, "content": "duplicate" })
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stale_completion.status(), StatusCode::CONFLICT);

    let failing_command = sessions
        .insert_command(&InsertCommand {
            command_id: Uuid::now_v7().to_string(),
            route_id: Uuid::now_v7().to_string(),
            session_key: format!("{}:{}", agent.id, conversation.id),
            agent_id: agent.id.clone(),
            conversation_id: conversation.id.clone(),
            message_id: Uuid::now_v7().to_string(),
            turn_id: Uuid::now_v7().to_string(),
            prompt: "Fail once and become observable".into(),
            max_attempts: 1,
            metadata: json!({}),
        })
        .await
        .unwrap();
    let failed_claim = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/v1/runtime-hosts/{host_id}/commands/claim"))
                .header("x-choruz-host-token", &host_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let failed_claim: Value = serde_json::from_slice(
        &to_bytes(failed_claim.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(failed_claim["command_id"], failing_command.command_id);
    let failed_completion = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!(
                    "/v1/runtime-hosts/{host_id}/commands/{}/complete",
                    failing_command.command_id
                ))
                .header("x-choruz-host-token", &host_token)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "attempt_id": failed_claim["attempt_id"],
                        "succeeded": false,
                        "error": "remote harness exited",
                        "clear_external_session": true
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(failed_completion.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        sessions
            .get_command(&failing_command.command_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        CommandStatus::DeadLetter
    );
    let dead_letter_count: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM dead_letters WHERE source_id = $1",
            &[&failing_command.command_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(dead_letter_count, 1);
    let cleared_session: Option<String> = client
        .query_one(
            "SELECT external_session_id FROM agent_runtime_bindings WHERE id = $1",
            &[&binding["id"].as_str().unwrap()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(cleared_session.is_none());

    let queued_for_remote = sessions
        .insert_command(&InsertCommand {
            command_id: Uuid::now_v7().to_string(),
            route_id: Uuid::now_v7().to_string(),
            session_key: format!("{}:{}", agent.id, conversation.id),
            agent_id: agent.id.clone(),
            conversation_id: conversation.id.clone(),
            message_id: Uuid::now_v7().to_string(),
            turn_id: Uuid::now_v7().to_string(),
            prompt: "Move this queued command back to local execution".into(),
            max_attempts: 3,
            metadata: json!({}),
        })
        .await
        .unwrap();
    assert_eq!(queued_for_remote.metadata["runtime_host_id"], host_id);

    let mut move_device = connect_host_link(&base_url, &host_id, &host_token)
        .await
        .unwrap();
    let move_ack = tokio::spawn(async move {
        while let Some(Ok(Message::Text(text))) = move_device.next().await {
            let call: Value = serde_json::from_str(&text).unwrap();
            if call["kind"] != "call" {
                continue;
            }
            assert_eq!(call["request"]["call"], "terminal_close");
            move_device
                .send(Message::Text(
                    json!({"kind":"result", "id":call["id"], "ok":null})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            return true;
        }
        false
    });

    let (move_local_status, _) = api_json_payload_request(
        router.clone(),
        &operator,
        Method::PUT,
        format!(
            "/v1/runtime/bindings/{}/host",
            binding["id"].as_str().unwrap()
        ),
        json!({ "runtime_host_id": null }),
    )
    .await;
    assert_eq!(move_local_status, StatusCode::NO_CONTENT);
    assert!(move_ack.await.unwrap());
    let binding_config: Value = client
        .query_one(
            "SELECT config_json FROM agent_runtime_bindings WHERE id = $1",
            &[&binding["id"].as_str().unwrap()],
        )
        .await
        .unwrap()
        .get("config_json");
    assert!(
        !binding_config
            .as_object()
            .expect("binding config is an object")
            .contains_key("runtime_host_id")
    );
    let moved_command = sessions
        .get_command(&queued_for_remote.command_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(moved_command.status, CommandStatus::Pending);
    assert!(
        !moved_command
            .metadata
            .as_object()
            .unwrap()
            .contains_key("runtime_host_id")
    );
    assert!(
        sessions
            .find_pending_commands(100)
            .await
            .unwrap()
            .iter()
            .any(|candidate| candidate.command_id == queued_for_remote.command_id)
    );

    let runtime_dir = isolated_test_dir("runtime-host-outbox-mirror");
    let _runtime_env = EnvVarGuard::set_path("CHORUZ_RUNTIME_DIR", &runtime_dir);
    let ship = |binding_id: &str, token: &str, commands: Value| {
        Request::builder()
            .method(Method::POST)
            .uri(format!(
                "/v1/runtime-hosts/{host_id}/bindings/{binding_id}/outbox"
            ))
            .header("x-choruz-host-token", token)
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "commands": commands }).to_string()))
            .unwrap()
    };
    let shipped_command = json!([{
        "name": "cmd-00000000000000000001-a.json",
        "command": { "type": "share_file", "group": "cross-host-build", "path": "out/report.md" },
        "files": [{ "path": "out/report.md", "content": "IyBSZXBvcnQK" }]
    }]);
    let wrong_token = router
        .clone()
        .oneshot(ship(
            imported_binding_id,
            "not-a-host-token",
            shipped_command.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(wrong_token.status(), StatusCode::UNAUTHORIZED);
    let local_binding = router
        .clone()
        .oneshot(ship(
            binding["id"].as_str().unwrap(),
            &host_token,
            shipped_command.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(
        local_binding.status(),
        StatusCode::FORBIDDEN,
        "a host ships only for bindings placed on it"
    );
    let shipped = router
        .clone()
        .oneshot(ship(
            imported_binding_id,
            &host_token,
            shipped_command.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(shipped.status(), StatusCode::NO_CONTENT);
    let mirror = choruz_host_runtime::outbox::remote_outbox_dir(imported_binding_id, &host_id);
    assert_eq!(
        mirror,
        runtime_dir
            .join("remote-outbox")
            .join(imported_binding_id)
            .join(&host_id),
        "the mirror is keyed by binding and by the device that shipped"
    );
    let mirrored_command: Value = serde_json::from_slice(
        &fs::read(
            mirror
                .join(".choruz-outbox")
                .join("new")
                .join("cmd-00000000000000000001-a.json"),
        )
        .expect("the shipped command lands in the mirror"),
    )
    .unwrap();
    assert_eq!(mirrored_command["type"], "share_file");
    assert_eq!(
        fs::read_to_string(mirror.join("out").join("report.md")).unwrap(),
        "# Report\n",
        "the shared file travels with its command, base64 on the wire"
    );
    let mirrored_command_path = mirror
        .join(".choruz-outbox/new")
        .join("cmd-00000000000000000001-a.json");
    fs::remove_file(&mirrored_command_path).expect("simulate the pipeline draining the command");
    let retried = router
        .clone()
        .oneshot(ship(imported_binding_id, &host_token, shipped_command))
        .await
        .unwrap();
    assert_eq!(retried.status(), StatusCode::NO_CONTENT);
    assert!(
        !mirrored_command_path.exists(),
        "a lost acknowledgement must not republish an accepted command"
    );

    // A file attached to a turn on the device is fetched by the host with
    // its token, but only for a command placed on it and only a file that
    // command names; the agent's own attachment access still applies.
    let (upload_status, uploaded) = api_json_payload_request(
        router.clone(),
        &operator,
        Method::POST,
        "/v1/attachments".into(),
        json!({
            "actor_id": operator.id,
            "filename": "brief.txt",
            "content_type": "text/plain",
            "data_base64": "YXR0YWNoZWQtZGF0YQ=="
        }),
    )
    .await;
    assert_eq!(upload_status, StatusCode::CREATED, "{uploaded}");
    let attachment_id = uploaded["id"].as_str().unwrap().to_owned();
    client
        .execute(
            "INSERT INTO conversation_events
               (conversation_id, seq, event_id, event_type, sender_id, content, content_type,
                metadata, client_msg_id, created_at)
             VALUES ($1, 9001, 'attached-message', 'message', $2, 'see attached', 'text/plain',
                     $3, 'attached-message-client', NOW())",
            &[
                &conversation.id,
                &operator.id,
                &json!({ "attachment_id": attachment_id }),
            ],
        )
        .await
        .unwrap();
    let attachment_metadata = json!({
        "attachments": [{
            "attachment_id": attachment_id,
            "filename": "brief.txt",
            "mime_type": "text/plain"
        }]
    });
    let mut on_device = attachment_metadata.clone();
    on_device["runtime_host_id"] = json!(host_id);
    let remote_with_attachment = sessions
        .insert_command(&InsertCommand {
            command_id: Uuid::now_v7().to_string(),
            route_id: Uuid::now_v7().to_string(),
            session_key: format!("{}:{}", agent.id, conversation.id),
            agent_id: agent.id.clone(),
            conversation_id: conversation.id.clone(),
            message_id: Uuid::now_v7().to_string(),
            turn_id: Uuid::now_v7().to_string(),
            prompt: "Read the attached brief".into(),
            max_attempts: 3,
            metadata: on_device,
        })
        .await
        .unwrap();
    let local_with_attachment = sessions
        .insert_command(&InsertCommand {
            command_id: Uuid::now_v7().to_string(),
            route_id: Uuid::now_v7().to_string(),
            session_key: format!("{}:{}", agent.id, conversation.id),
            agent_id: agent.id.clone(),
            conversation_id: conversation.id.clone(),
            message_id: Uuid::now_v7().to_string(),
            turn_id: Uuid::now_v7().to_string(),
            prompt: "Read the attached brief locally".into(),
            max_attempts: 3,
            metadata: attachment_metadata,
        })
        .await
        .unwrap();
    let fetch_attachment = |command_id: &str, attachment: &str| {
        Request::builder()
            .method(Method::GET)
            .uri(format!(
                "/v1/runtime-hosts/{host_id}/commands/{command_id}/attachments/{attachment}"
            ))
            .header("x-choruz-host-token", &host_token)
            .body(Body::empty())
            .unwrap()
    };
    let local_command = router
        .clone()
        .oneshot(fetch_attachment(
            &local_with_attachment.command_id,
            &attachment_id,
        ))
        .await
        .unwrap();
    assert_eq!(
        local_command.status(),
        StatusCode::FORBIDDEN,
        "a host fetches only for turns placed on it"
    );
    let unnamed = router
        .clone()
        .oneshot(fetch_attachment(
            &remote_with_attachment.command_id,
            "att-not-on-this-message",
        ))
        .await
        .unwrap();
    assert_eq!(
        unnamed.status(),
        StatusCode::FORBIDDEN,
        "a host fetches only the files the turn names"
    );
    let fetched = router
        .clone()
        .oneshot(fetch_attachment(
            &remote_with_attachment.command_id,
            &attachment_id,
        ))
        .await
        .unwrap();
    assert_eq!(fetched.status(), StatusCode::OK);
    assert_eq!(
        fetched
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/plain")
    );
    assert_eq!(
        to_bytes(fetched.into_body(), usize::MAX).await.unwrap(),
        "attached-data".as_bytes()
    );

    let (offline_revoke_status, _) = api_json_payload_request(
        router.clone(),
        &operator,
        Method::DELETE,
        format!("/v1/runtime-hosts/{host_id}"),
        Value::Null,
    )
    .await;
    assert_eq!(offline_revoke_status, StatusCode::CONFLICT);
    client
        .execute(
            "UPDATE agent_runtime_bindings SET config_json=jsonb_set(
          config_json, '{runtime_host_id}', to_jsonb($2::text)) WHERE id=$1",
            &[&binding["id"].as_str().unwrap(), &host_id],
        )
        .await
        .unwrap();
    let expected_closes: i64 = client
        .query_one(
            "SELECT count(*) FROM agent_runtime_bindings WHERE config_json->>'runtime_host_id'=$1",
            &[&host_id],
        )
        .await
        .unwrap()
        .get(0);
    assert!(expected_closes > 1);
    let moved_away_id: String = client.query_one(
        "SELECT id FROM agent_runtime_bindings WHERE config_json->>'runtime_host_id'=$1 ORDER BY id DESC LIMIT 1",
        &[&host_id],
    ).await.unwrap().get(0);
    let other_host = Uuid::now_v7().to_string();
    client
        .execute(
            "INSERT INTO runtime_host (id, company_id, name, token_hash, status)
         SELECT $2, company_id, 'Other destination', $2, 'offline' FROM runtime_host WHERE id=$1",
            &[&host_id, &other_host],
        )
        .await
        .unwrap();
    let expected_closes = expected_closes - 1;
    let mut reconnected = connect_host_link(&base_url, &host_id, &host_token)
        .await
        .unwrap();
    let (close_started, observed_close) = tokio::sync::oneshot::channel();
    let (release_close, wait_for_assignment) = tokio::sync::oneshot::channel();
    let close_acknowledgements = tokio::spawn(async move {
        let mut count = 0;
        let mut close_started = Some(close_started);
        let mut wait_for_assignment = Some(wait_for_assignment);
        while let Some(Ok(Message::Text(text))) = reconnected.next().await {
            let call: Value = serde_json::from_str(&text).unwrap();
            if call["kind"] != "call" {
                continue;
            }
            assert_eq!(call["request"]["call"], "terminal_close");
            count += 1;
            if let Some(started) = close_started.take() {
                started.send(()).unwrap();
                wait_for_assignment.take().unwrap().await.unwrap();
            }
            reconnected
                .send(Message::Text(
                    json!({"kind":"result", "id":call["id"], "ok":null})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            if count == expected_closes {
                break;
            }
        }
        count
    });
    let revoke_router = router.clone();
    let revoke_actor = operator.clone();
    let revoke_id = host_id.clone();
    let revocation = tokio::spawn(async move {
        api_json_payload_request(
            revoke_router,
            &revoke_actor,
            Method::DELETE,
            format!("/v1/runtime-hosts/{revoke_id}"),
            Value::Null,
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), observed_close)
        .await
        .unwrap()
        .unwrap();
    // Revocation has enumerated A's bindings and is waiting for the first
    // device acknowledgement. Commit a move of a later binding to B under the
    // same binding lock used by assignment. Revocation must not close B.
    let mut moving_client = RuntimeStore::new(database.database_url.clone())
        .connect()
        .await
        .unwrap();
    let moving_tx = moving_client.transaction().await.unwrap();
    crate::handlers_terminals::lock_terminal_launch(&moving_tx, &moved_away_id)
        .await
        .unwrap();
    moving_tx
        .execute(
            "UPDATE agent_runtime_bindings SET config_json=jsonb_set(
          config_json - 'terminal_session' - 'terminal_capture',
          '{runtime_host_id}', to_jsonb($2::text)), updated_at=NOW() WHERE id=$1",
            &[&moved_away_id, &other_host],
        )
        .await
        .unwrap();
    moving_tx.commit().await.unwrap();
    let moving_id: String = client.query_one(
        "SELECT id FROM agent_runtime_bindings WHERE config_json->>'runtime_host_id'=$1 ORDER BY id LIMIT 1",
        &[&host_id],
    ).await.unwrap().get(0);
    let assign_router = router.clone();
    let assign_actor = operator.clone();
    let assign_host = host_id.clone();
    let assign_id = moving_id.clone();
    let assignment = tokio::spawn(async move {
        api_json_payload_request(
            assign_router,
            &assign_actor,
            Method::PUT,
            format!("/v1/runtime/bindings/{assign_id}/host"),
            json!({"runtime_host_id":assign_host}),
        )
        .await
    });
    // The device acknowledgement holds revocation open. Wait for the competing
    // assignment's database lock, not a scheduler-dependent sleep.
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let blocked: bool = client.query_one(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock')", &[],
            ).await.unwrap().get(0);
            if blocked { break; }
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    release_close.send(()).unwrap();
    let (revoke_status, _) = revocation.await.unwrap();
    assert_eq!(revoke_status, StatusCode::NO_CONTENT);
    assert_eq!(close_acknowledgements.await.unwrap(), expected_closes);
    let (assign_status, _) = assignment.await.unwrap();
    assert_eq!(assign_status, StatusCode::BAD_REQUEST);
    let assigned: Option<String> = client
        .query_one(
            "SELECT config_json->>'runtime_host_id' FROM agent_runtime_bindings WHERE id=$1",
            &[&moving_id],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        assigned.is_none(),
        "a revoked device cannot receive a late assignment"
    );
    let kept_host: String = client
        .query_one(
            "SELECT config_json->>'runtime_host_id' FROM agent_runtime_bindings WHERE id=$1",
            &[&moved_away_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        kept_host, other_host,
        "revoking A must leave the moved B binding alone"
    );
    let rejected = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/v1/runtime-hosts/{host_id}/heartbeat"))
                .header("x-choruz-host-token", host_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn runtime_binding_creation_defaults_mention_aliases_from_agent_name() {
    let database = TestDatabase::create().await;
    let runtime = RuntimeStore::new(database.database_url.clone());
    let app = choruz_application::ChatApp::new();
    let auth = LocalAuthConfig::from_env();
    let operator = auth.ensure_operator_sync(&app).unwrap();
    let human = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: operator.workspace_id.clone(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    let non_member = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: operator.workspace_id.clone(),
            principal_type: PrincipalType::Human,
            name: "Bob".into(),
            avatar_url: None,
        })
        .unwrap();
    let agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Claude Code 1".into(),
            scopes: vec![
                "messages:read".into(),
                "messages:write".into(),
                "events:read".into(),
            ],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap();
    let conversation = app
        .create_group(CreateGroupRequest {
            actor_id: operator.id.clone(),
            name: "Runtime Mention Test".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![human.id.clone(), agent.principal.id.clone()],
            workspace_id: None,
        })
        .unwrap();
    // Persist principals and conversation to the test DB so DbService can find them.
    let event_store = choruz_store::EventStore::new(&database.database_url);
    persist_principal_to_db(&event_store, &operator).await;
    persist_principal_to_db(&event_store, &human).await;
    persist_principal_to_db(&event_store, &non_member).await;
    persist_principal_to_db(&event_store, &agent.principal).await;
    seed_conversation_to_db(&database.database_url, &conversation).await;
    let router = runtime_router_with_db(app, runtime.clone(), &database.database_url);

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/runtime/bindings")
                .header("authorization", format!("Bearer {}", session_token(&human)))
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "conversation_id": conversation.id,
                        "agent_principal_id": agent.principal.id,
                        "driver_type": "opencode_terminal",
                        "workspace_path": "/worktrees/claude-code-1",
                        "git_worktree_path": null,
                        "config_json": {
                            "allowed_tools": ["Read"]
                        }
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let binding = runtime
        .get_binding(payload["id"].as_str().unwrap())
        .await
        .unwrap();

    assert_eq!(binding.driver_type, DriverType::OpenCodeTerminal);
    assert_eq!(payload["driver_type"], "opencode_terminal");
    assert_eq!(binding.config_json["agent_name"], "Claude Code 1");
    assert_eq!(binding.config_json["mention_aliases"][0], "Claude Code 1");
    assert_eq!(binding.config_json["allowed_tools"][0], "Read");

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/runtime/bindings")
                .header(
                    "authorization",
                    format!("Bearer {}", session_token(&non_member)),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "conversation_id": conversation.id,
                        "agent_principal_id": agent.principal.id,
                        "driver_type": "claude_print",
                        "workspace_path": "/worktrees/forbidden",
                        "git_worktree_path": null,
                        "config_json": {}
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn harness_account_binding_trigger_rejects_unverified_models() {
    let database = TestDatabase::create().await;
    let runtime = RuntimeStore::new(database.database_url.clone());
    let event_store = choruz_store::EventStore::new(&database.database_url);
    let app = choruz_application::ChatApp::new();
    let operator = LocalAuthConfig::from_env()
        .ensure_operator_sync(&app)
        .unwrap();
    let agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Account-bound Codex".into(),
            scopes: vec!["messages:read".into(), "messages:write".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap();
    let conversation = app
        .create_direct_conversation(CreateDirectConversationRequest {
            actor_id: operator.id.clone(),
            peer_principal_id: agent.principal.id.clone(),
            workspace_id: None,
        })
        .unwrap();
    persist_principal_to_db(&event_store, &operator).await;
    persist_principal_to_db(&event_store, &agent.principal).await;
    seed_conversation_to_db(&database.database_url, &conversation).await;

    let account_id = Uuid::now_v7().to_string();
    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect harness account test database");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "INSERT INTO company (id, name, slug, owner_id)
             VALUES ($1, 'Harness Account Test', $1, $2)",
            &[&operator.workspace_id, &operator.id],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO harness_account
                (id, company_id, driver_type, name, profile_kind, status, models_json)
             VALUES ($1, $2, 'codex_terminal', 'Work account', 'isolated', 'active',
                     '[{\"id\":\"gpt-5.6-sol\"}]'::jsonb)",
            &[&account_id, &operator.workspace_id],
        )
        .await
        .unwrap();

    let invalid = runtime
        .create_binding(CreateBindingInput {
            conversation_id: conversation.id.clone(),
            agent_principal_id: agent.principal.id.clone(),
            driver_type: DriverType::CodexTerminal,
            workspace_path: "/worktrees/account-bound-codex".into(),
            git_worktree_path: None,
            config_json: json!({
                "harness_account_id": account_id,
                "model": "gpt-5.4-mini"
            }),
            audit_actor: None,
        })
        .await
        .unwrap_err();
    assert!(
        invalid
            .to_string()
            .contains("invalid active harness account or model")
    );

    let binding = runtime
        .create_binding(CreateBindingInput {
            conversation_id: conversation.id,
            agent_principal_id: agent.principal.id,
            driver_type: DriverType::CodexTerminal,
            workspace_path: "/worktrees/account-bound-codex".into(),
            git_worktree_path: None,
            config_json: json!({
                "harness_account_id": account_id,
                "harness_account_name": "Spoofed account",
                "harness_account_profile_kind": "default",
                "model": "gpt-5.6-sol"
            }),
            audit_actor: None,
        })
        .await
        .unwrap();
    assert_eq!(binding.config_json["harness_account_id"], account_id);
    assert_eq!(binding.config_json["harness_account_name"], "Work account");
    assert_eq!(
        binding.config_json["harness_account_profile_kind"],
        "isolated"
    );
}

#[tokio::test]
async fn bootstrap_carries_runtime_bindings_and_the_feed_names_new_ones() {
    let database = TestDatabase::create().await;
    let runtime = RuntimeStore::new(database.database_url.clone());
    let app = choruz_application::ChatApp::new();
    let operator = LocalAuthConfig::from_env()
        .ensure_operator_sync(&app)
        .unwrap();
    let agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Terminal Dev".into(),
            scopes: vec!["messages:read".into(), "messages:write".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap();
    let conversation = app
        .create_direct_conversation(CreateDirectConversationRequest {
            actor_id: operator.id.clone(),
            peer_principal_id: agent.principal.id.clone(),
            workspace_id: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &operator).await;
    seed_principal_to_db(&database.database_url, &agent.principal).await;
    seed_conversation_to_db(&database.database_url, &conversation).await;
    let terminal = runtime
        .create_binding(CreateBindingInput {
            conversation_id: conversation.id.clone(),
            agent_principal_id: agent.principal.id.clone(),
            driver_type: DriverType::ClaudeTerminal,
            workspace_path: "/worktrees/terminal-dev".into(),
            git_worktree_path: None,
            config_json: json!({}),
            audit_actor: Some(audit_actor(&operator)),
        })
        .await
        .unwrap();
    let app_for_feed = app.clone();
    let router = runtime_router_with_db(app, runtime.clone(), &database.database_url);

    let (status, bootstrap) = api_json_request(
        router.clone(),
        &operator,
        Method::GET,
        "/v1/bootstrap?limit=10".into(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (list_status, list) = api_json_request(
        router.clone(),
        &operator,
        Method::GET,
        "/v1/runtime/bindings".into(),
    )
    .await;
    assert_eq!(list_status, StatusCode::OK);
    assert_eq!(
        bootstrap["runtime_bindings"], list,
        "the snapshot and the list endpoint come from the same query"
    );
    let bound = &bootstrap["runtime_bindings"][0];
    assert_eq!(bound["id"], terminal.id);
    assert_eq!(bound["conversation_id"], conversation.id);
    assert_eq!(bound["conversation_type"], "direct");
    assert_eq!(bound["agent_name"], "Terminal Dev");
    assert_eq!(bound["interaction_mode"], "session");
    assert!(
        bound["conversation_name"]
            .as_str()
            .unwrap()
            .contains("Terminal Dev"),
        "a nameless DM is labelled by its members: {bound}"
    );

    let (detail_status, detail) = api_json_request(
        router.clone(),
        &operator,
        Method::GET,
        format!("/v1/runtime/bindings/{}", terminal.id),
    )
    .await;
    assert_eq!(detail_status, StatusCode::OK);
    assert_eq!(detail, *bound);

    // A binding created after the snapshot reaches the feed by id, so a
    // client re-reads that one binding instead of the whole snapshot. An
    // agent holds one binding per conversation, so the second one gets its
    // own agent and DM.
    let cursor = bootstrap["sync_cursor"].as_u64().unwrap();
    let print_agent = app_for_feed
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Print Dev".into(),
            scopes: vec!["messages:read".into(), "messages:write".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap();
    let print_conversation = app_for_feed
        .create_direct_conversation(CreateDirectConversationRequest {
            actor_id: operator.id.clone(),
            peer_principal_id: print_agent.principal.id.clone(),
            workspace_id: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &print_agent.principal).await;
    seed_conversation_to_db(&database.database_url, &print_conversation).await;
    let message_mode = runtime
        .create_binding(CreateBindingInput {
            conversation_id: print_conversation.id.clone(),
            agent_principal_id: print_agent.principal.id.clone(),
            driver_type: DriverType::ClaudePrint,
            workspace_path: "/worktrees/print-dev".into(),
            git_worktree_path: None,
            config_json: json!({}),
            audit_actor: Some(audit_actor(&operator)),
        })
        .await
        .unwrap();
    let (sync_status, sync) = api_json_request(
        router.clone(),
        &operator,
        Method::GET,
        format!("/v1/sync?cursor={cursor}&limit=100"),
    )
    .await;
    assert_eq!(sync_status, StatusCode::OK);
    let binding_events: Vec<(&str, &str)> = sync["changes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|change| change["entity_type"] == "runtime_binding")
        .map(|change| {
            (
                change["event_type"].as_str().unwrap(),
                change["entity_id"].as_str().unwrap(),
            )
        })
        .collect();
    assert!(
        binding_events.contains(&("runtime_binding.created", message_mode.id.as_str())),
        "feed lacks the new binding: {binding_events:?}"
    );
    let (detail_status, detail) = api_json_request(
        router.clone(),
        &operator,
        Method::GET,
        format!("/v1/runtime/bindings/{}", message_mode.id),
    )
    .await;
    assert_eq!(detail_status, StatusCode::OK);
    assert_eq!(detail["interaction_mode"], "message");
}

/// Serve a router on a loopback port for the WebSocket routes a `oneshot`
/// request cannot exercise; the state, including the host link hub, stays
/// shared with every clone of the router.
async fn serve_router(router: axum::Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap()
    });
    format!("http://{address}")
}

type DeviceSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Dial the host link as a device and complete the hello/welcome exchange.
async fn connect_host_link(
    base_url: &str,
    host_id: &str,
    host_token: &str,
) -> Result<DeviceSocket, String> {
    let url = format!(
        "{}{}",
        base_url.replacen("http://", "ws://", 1),
        choruz_host_runtime::link::LINK_PATH
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|error| error.to_string())?;
    socket
        .send(Message::Text(
            json!({
                "kind": "hello",
                "host_id": host_id,
                "host_token": host_token,
                "protocol": choruz_host_runtime::link::LINK_PROTOCOL,
            })
            .to_string()
            .into(),
        ))
        .await
        .map_err(|error| error.to_string())?;
    match tokio::time::timeout(Duration::from_secs(5), socket.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => {
            let frame: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
            if frame["kind"] == "welcome" {
                Ok(socket)
            } else {
                Err(format!("unexpected first frame {frame}"))
            }
        }
        other => Err(format!("link was not welcomed: {other:?}")),
    }
}

#[cfg(unix)]
#[tokio::test]
async fn remote_terminal_binding_streams_through_the_host_link() {
    let _env = ChannelTaskEnvGuard::remote_control();
    let _guard = api_test_env_lock().lock().await;
    let root = isolated_test_dir("remote-terminal-link");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).expect("create workspace");
    let fake_cli = root.join("fake-claude");
    write_executable_script(
        &fake_cli,
        "#!/bin/sh\nprintf 'READY\\r\\n'\nIFS= read -r line\nprintf 'ECHO:%s\\r\\n' \"$line\"\nsleep 30\n",
    );

    let database = TestDatabase::create().await;
    let runtime = RuntimeStore::new(database.database_url.clone());
    let app = choruz_application::ChatApp::new();
    let auth = LocalAuthConfig::from_env();
    let operator = auth.ensure_operator_sync(&app).unwrap();
    let agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Remote Claude".into(),
            scopes: vec!["messages:read".into(), "messages:write".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap();
    let conversation = app
        .create_direct_conversation(CreateDirectConversationRequest {
            actor_id: operator.id.clone(),
            peer_principal_id: agent.principal.id.clone(),
            workspace_id: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &operator).await;
    seed_principal_to_db(&database.database_url, &agent.principal).await;
    seed_conversation_to_db(&database.database_url, &conversation).await;
    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .unwrap();
    tokio::spawn(async move { connection.await.unwrap() });
    client
        .execute(
            "INSERT INTO company (id, name, slug, owner_id) VALUES ($1, 'Remote Terminals', $1, $2)",
            &[&operator.workspace_id, &operator.id],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO company_member (company_id, principal_id) VALUES ($1, $2)",
            &[&operator.workspace_id, &operator.id],
        )
        .await
        .unwrap();
    let router = runtime_router_with_db(app, runtime.clone(), &database.database_url);

    let (_, pairing) = api_json_payload_request(
        router.clone(),
        &operator,
        Method::POST,
        format!(
            "/v1/companies/{}/runtime-host-pairings",
            operator.workspace_id
        ),
        Value::Null,
    )
    .await;
    let mut redeem = Request::builder()
        .method(Method::POST)
        .uri("/v1/runtime-host-pairings/redeem")
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({ "code": pairing["code"], "name": "Laptop" }).to_string(),
        ))
        .unwrap();
    redeem.extensions_mut().insert(axum::extract::ConnectInfo(
        "127.0.0.1:41001".parse::<std::net::SocketAddr>().unwrap(),
    ));
    let response = router.clone().oneshot(redeem).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let paired: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    let host_id = paired["host"]["id"].as_str().unwrap().to_owned();
    let host_token = paired["host_token"].as_str().unwrap().to_owned();

    let binding = runtime
        .create_binding(CreateBindingInput {
            conversation_id: conversation.id.clone(),
            agent_principal_id: agent.principal.id.clone(),
            driver_type: DriverType::ClaudeTerminal,
            workspace_path: workspace.to_string_lossy().to_string(),
            git_worktree_path: None,
            config_json: json!({
                "binary_path": fake_cli.to_string_lossy(),
                "runtime_host_id": host_id,
            }),
            audit_actor: Some(audit_actor(&operator)),
        })
        .await
        .expect("create remote terminal binding");
    let base_url = serve_router(router.clone()).await;

    // Without a connected device the terminal cannot open.
    let unreachable = tokio_tungstenite::connect_async(format!(
        "{}/v1/ws/terminals/{}?token={}",
        base_url.replacen("http://", "ws://", 1),
        binding.id,
        session_token(&operator)
    ))
    .await;
    assert!(
        unreachable.is_err(),
        "terminal opened without a device link"
    );

    // The device: the same code the connector runs, on this machine.
    let device_socket = connect_host_link_raw(&base_url).await;
    let (mut device_sink, mut device_source) = device_socket.split();
    let (outbound_tx, mut outbound_rx) = tokio::sync::mpsc::channel::<String>(64);
    let (inbound_tx, inbound_rx) = tokio::sync::mpsc::channel::<String>(64);
    tokio::spawn(async move {
        while let Some(text) = outbound_rx.recv().await {
            if device_sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });
    tokio::spawn(async move {
        while let Some(Ok(message)) = device_source.next().await {
            if let Message::Text(text) = message
                && inbound_tx.send(text.to_string()).await.is_err()
            {
                break;
            }
        }
    });
    let pool = choruz_host_runtime::new_terminal_pool();
    let device = tokio::spawn(choruz_host_runtime::link::run_device_link(
        outbound_tx,
        inbound_rx,
        choruz_host_runtime::link::DeviceIdentity {
            host_id: host_id.clone(),
            host_token,
        },
        pool.clone(),
    ));
    tokio::time::timeout(Duration::from_secs(5), async {
        while !router_state_has_link(&base_url, &operator, &host_id).await {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("device link registers");

    let (mut terminal, _) = tokio_tungstenite::connect_async(format!(
        "{}/v1/ws/terminals/{}?token={}&cols=80&rows=24",
        base_url.replacen("http://", "ws://", 1),
        binding.id,
        session_token(&operator)
    ))
    .await
    .expect("open the remote terminal");
    let mut transcript = Vec::new();
    read_terminal_until(&mut terminal, &mut transcript, "READY").await;
    terminal
        .send(Message::Text("hello over the link\n".into()))
        .await
        .unwrap();
    let seen =
        read_terminal_until(&mut terminal, &mut transcript, "ECHO:hello over the link").await;
    assert!(seen.contains("READY"), "{seen}");
    assert!(
        choruz_host_runtime::live_terminal_exists(&pool, &binding.id),
        "the terminal runs on the device, not on the gateway"
    );

    terminal.close(None).await.unwrap();
    drop(terminal);
    tokio::time::timeout(Duration::from_secs(5), async {
        while choruz_host_runtime::live_terminal_exists(&pool, &binding.id) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("closing the browser socket closes the device terminal");
    device.abort();
}

async fn read_terminal_until(
    terminal: &mut DeviceSocket,
    transcript: &mut Vec<u8>,
    needle: &str,
) -> String {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if String::from_utf8_lossy(transcript).contains(needle) {
                break;
            }
            match terminal.next().await {
                Some(Ok(Message::Binary(bytes))) => transcript.extend_from_slice(&bytes),
                Some(Ok(_)) => {}
                other => panic!("terminal closed before {needle}: {other:?}"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {needle}"));
    String::from_utf8_lossy(transcript).into_owned()
}

async fn connect_host_link_raw(base_url: &str) -> DeviceSocket {
    let url = format!(
        "{}{}",
        base_url.replacen("http://", "ws://", 1),
        choruz_host_runtime::link::LINK_PATH
    );
    tokio_tungstenite::connect_async(&url)
        .await
        .expect("dial the host link")
        .0
}

/// Whether the host link hub knows the device: the host list reports the
/// device online once its hello was accepted.
async fn router_state_has_link(
    base_url: &str,
    operator: &choruz_domain::Principal,
    host_id: &str,
) -> bool {
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{base_url}/v1/runtime-hosts/{host_id}/operations"))
        .bearer_auth(session_token(operator))
        .json(&json!({ "kind": "filesystem.home", "request": {} }))
        .send()
        .await;
    matches!(response, Ok(response) if response.status() == reqwest::StatusCode::OK)
}
