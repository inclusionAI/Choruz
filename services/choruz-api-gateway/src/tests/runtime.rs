use super::*;

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
        if codex_home_seen.exists() {
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

    let (revoke_status, _) = api_json_payload_request(
        router.clone(),
        &operator,
        Method::DELETE,
        format!("/v1/runtime-hosts/{host_id}"),
        Value::Null,
    )
    .await;
    assert_eq!(revoke_status, StatusCode::NO_CONTENT);
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

// NOTE: websocket_stream_receives_message_events was removed along with the
// `/v1/ws/events/{principal_id}` polling endpoint (see lib.rs comment above
// the `/v1/ws/terminals/...` route). Real-time message push now lives on the
// pipeline's fanout server (crate choruz-fanout, port :3020, path /ws/fanout)
// and is exercised by that crate's own tests rather than by choruz-api-gateway here.

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
    assert_eq!(bound["interaction_mode"], "terminal");
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
