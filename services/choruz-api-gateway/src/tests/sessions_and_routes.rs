use super::*;

#[tokio::test]
async fn native_session_import_lock_key_executes_against_postgres() {
    let database = TestDatabase::create_without_migrations().await;
    let (mut client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect native import lock database");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let transaction = client
        .transaction()
        .await
        .expect("begin native import lock transaction");
    let lock_key =
        native_session_import_lock_key("/projects/example", "codex_exec", "session-with-history");
    transaction
        .query_one(
            "SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0::bigint))",
            &[&lock_key],
        )
        .await
        .expect("acquire native import advisory lock");
    transaction.commit().await.expect("commit lock transaction");
}

#[tokio::test]
async fn native_session_import_runs_end_to_end_and_is_idempotent() {
    let _guard = api_test_env_lock().lock().await;
    let real_home = env::var("HOME").expect("test home");
    let home = tempfile::Builder::new()
        .prefix(".choruz-native-import-")
        .tempdir_in(real_home)
        .expect("native import home");
    let workspace = home.path().join("projects/example");
    let company_workspace = home.path().join("company-root");
    fs::create_dir_all(&workspace).expect("native import workspace");
    fs::create_dir_all(&company_workspace).expect("company workspace");
    let claude_project = home.path().join(".claude/projects/example");
    fs::create_dir_all(&claude_project).expect("claude project index");
    let session_id = "11111111-2222-4333-8444-555555555555";
    fs::write(
        claude_project.join(format!("{session_id}.jsonl")),
        format!(
            "{{\"type\":\"user\",\"cwd\":{},\"customTitle\":\"Imported history\"}}\n",
            serde_json::to_string(&workspace.to_string_lossy()).unwrap()
        ),
    )
    .expect("write claude session fixture");
    let _home = EnvVarGuard::set_path("HOME", home.path());

    let database = TestDatabase::create().await;
    let db =
        choruz_application::DbService::new(choruz_store::EventStore::new(&database.database_url));
    let human = db
        .create_human_user("session-importer", "password-123")
        .await
        .expect("create session importer");
    let company = db
        .create_company(CreateCompanyRequest {
            actor_id: human.id.clone(),
            name: "Import Company".into(),
            slug: Some("import-company".into()),
            description: None,
            folder_path: Some(company_workspace.to_string_lossy().into_owned()),
        })
        .await
        .expect("create import company");
    let app = router_with_db(choruz_application::ChatApp::new(), &database.database_url);
    let payload = json!({
        "company_id": company.id,
        "workspace_path": home.path(),
        "sessions": [{
            "harness": "claude",
            "native_session_id": session_id,
            "workspace_path": workspace,
        }],
    });

    for already_imported in [false, true] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/workspace-sessions/import")
                    .header(CONTENT_TYPE, "application/json")
                    .header("authorization", format!("Bearer {}", session_token(&human)))
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .expect("import session response");
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read import response");
        assert_eq!(
            status,
            StatusCode::OK,
            "import failed: {}",
            String::from_utf8_lossy(&body)
        );
        let body: Value = serde_json::from_slice(&body).expect("parse import response");
        assert_eq!(body["imported"][0]["already_imported"], already_imported);
        assert_eq!(body["imported"][0]["native_session_id"], session_id);
    }

    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect imported session database");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let row = client
        .query_one(
            "SELECT COUNT(*)::bigint, MIN(b.external_session_id)
             FROM native_session_import n
             JOIN agent_runtime_bindings b ON b.id = n.binding_id",
            &[],
        )
        .await
        .expect("query imported session");
    assert_eq!(row.get::<_, i64>(0), 1);
    assert_eq!(row.get::<_, Option<String>>(1).as_deref(), Some(session_id));
    let company_folder = client
        .query_one(
            "SELECT folder_path FROM company WHERE id = $1",
            &[&company.id],
        )
        .await
        .expect("read importing company folder")
        .get::<_, Option<String>>(0);
    assert_eq!(
        company_folder.as_deref(),
        Some(company_workspace.to_string_lossy().as_ref()),
        "importing a session must not attach its workspace to the company"
    );
}

#[tokio::test]
async fn local_installation_allows_multiple_humans_with_isolated_workspaces() {
    let database = TestDatabase::create().await;
    let db =
        choruz_application::DbService::new(choruz_store::EventStore::new(&database.database_url));

    let human = db
        .create_human_user("local-user", "password-123")
        .await
        .expect("first signup should create a user");
    assert_eq!(human.principal_type, PrincipalType::Human);

    let second = db
        .create_human_user("second-user", "password-456")
        .await
        .expect("second signup should create another user");
    assert_eq!(second.principal_type, PrincipalType::Human);
    assert_ne!(human.id, second.id);
    assert_ne!(human.workspace_id, second.workspace_id);

    let operator = db
        .ensure_local_operator("ws-local", "operator")
        .await
        .expect("configured operator should not reuse a signup account");
    assert_ne!(operator.id, human.id);
    assert_ne!(operator.id, second.id);
    assert_eq!(operator.workspace_id, "ws-local");

    let other_operator = db
        .ensure_local_operator("ws-other", "other-operator")
        .await
        .expect("another configured operator should receive its own default company");
    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect for default company lookup");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    for principal in [&operator, &other_operator] {
        let row = client
            .query_one(
                "SELECT c.slug, COUNT(cm.principal_id)::BIGINT AS member_count
                 FROM company c
                 JOIN company_member cm
                   ON cm.company_id = c.id AND cm.principal_id = $2
                 WHERE c.id = $1
                 GROUP BY c.slug",
                &[&principal.workspace_id, &principal.id],
            )
            .await
            .expect("operator default company and membership should exist");
        assert_eq!(row.get::<_, String>("slug"), "default");
        assert_eq!(row.get::<_, i64>("member_count"), 1);
    }

    let error = db
        .create_human_user("LOCAL-USER", "password-789")
        .await
        .expect_err("usernames remain unique, case-insensitively");
    assert!(matches!(error, AppError::Conflict(_)));
}

#[tokio::test]
async fn local_bootstrap_only_issues_sessions_to_loopback_browsers() {
    let database = TestDatabase::create().await;
    let router = router_with_db(choruz_application::ChatApp::new(), &database.database_url);

    let request = |peer: &str, return_port: &str| {
        let mut request = Request::builder()
            .method(Method::GET)
            .uri(format!(
                "/v1/auth/local/bootstrap?return_port={return_port}"
            ))
            .header("host", "127.0.0.1:30292")
            .body(Body::empty())
            .unwrap();
        request.extensions_mut().insert(axum::extract::ConnectInfo(
            peer.parse::<std::net::SocketAddr>().unwrap(),
        ));
        request
    };

    let local = router
        .clone()
        .oneshot(request("127.0.0.1:41000", "3100"))
        .await
        .unwrap();
    assert_eq!(local.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        local.headers().get("location").unwrap(),
        "http://127.0.0.1:3100/dashboard"
    );
    let cookie = local.headers().get("set-cookie").unwrap().to_str().unwrap();
    assert!(cookie.starts_with("choruz_session="));
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Lax"));

    let remote = router
        .clone()
        .oneshot(request("203.0.113.8:41000", "3100"))
        .await
        .unwrap();
    assert_eq!(remote.status(), StatusCode::FORBIDDEN);
    assert!(remote.headers().get("set-cookie").is_none());

    let mut proxied = request("127.0.0.1:41000", "3100");
    proxied
        .headers_mut()
        .insert("x-forwarded-for", "203.0.113.8".parse().unwrap());
    let proxied = router.clone().oneshot(proxied).await.unwrap();
    assert_eq!(proxied.status(), StatusCode::FORBIDDEN);
    assert!(proxied.headers().get("set-cookie").is_none());

    let invalid = router
        .oneshot(request("127.0.0.1:41000", "0"))
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    assert!(invalid.headers().get("set-cookie").is_none());
}

#[tokio::test]
async fn concurrent_local_operator_logins_converge_on_one_principal() {
    let database = TestDatabase::create().await;
    let db =
        choruz_application::DbService::new(choruz_store::EventStore::new(&database.database_url));

    let attempts: Vec<_> = (0..100)
        .map(|_| {
            let db = db.clone();
            tokio::spawn(async move { db.ensure_local_operator("ws-local", "operator").await })
        })
        .collect();

    for attempt in attempts {
        let principal = attempt
            .await
            .expect("operator login task should not panic")
            .expect("concurrent operator login should succeed");
        assert_eq!(
            principal.id,
            choruz_auth::local_user_principal_id("ws-local", "operator")
        );
    }

    let client = db
        .store()
        .connect()
        .await
        .expect("connect to count operators");
    let operator_count: i64 = client
        .query_one(
            "SELECT COUNT(*)::BIGINT FROM principal
             WHERE type = 'human' AND lower(name) = lower('operator')",
            &[],
        )
        .await
        .expect("count operators")
        .get(0);
    assert_eq!(operator_count, 1);
}

#[tokio::test]
async fn workspace_migration_rejects_impersonated_actor_id() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let operator = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-operator".into(),
            principal_type: PrincipalType::Human,
            name: "Operator".into(),
            avatar_url: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &operator).await;
    let test_router = router_with_db(app, &database.database_url);
    let payload = json!({
        "actor_id": "another-human",
        "workspace_id": "ws-target"
    });

    let (conversation_status, _) = api_json_payload_request(
        test_router.clone(),
        &operator,
        Method::PATCH,
        "/v1/conversations/missing/workspace".into(),
        payload.clone(),
    )
    .await;
    let (principal_status, _) = api_json_payload_request(
        test_router,
        &operator,
        Method::PATCH,
        "/v1/principals/missing/workspace".into(),
        payload,
    )
    .await;

    assert_eq!(conversation_status, StatusCode::FORBIDDEN);
    assert_eq!(principal_status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn disabled_kanban_plugin_does_not_register_routes() {
    let _env = ChannelTaskEnvGuard::disabled();
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let auth = LocalAuthConfig::from_env();
    let operator = auth.ensure_operator_sync(&app).unwrap();
    let router = router_with_db(app, &database.database_url);
    let unauthenticated = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/conversations/channel-disabled/tasks")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthenticated.status(), StatusCode::NOT_FOUND);

    let cases = [
        (
            Method::GET,
            "/v1/conversations/channel-disabled/tasks",
            Value::Null,
        ),
        (
            Method::POST,
            "/v1/conversations/channel-disabled/tasks",
            json!({
                "task_key": "CHAN-1",
                "title": "Should stay gated",
                "idempotency_key": "turn-1"
            }),
        ),
        (
            Method::POST,
            "/v1/conversations/channel-disabled/tasks/from-message",
            json!({
                "message_id": "msg-1",
                "title": "Should stay gated",
                "assignee_principal_id": "agent-1"
            }),
        ),
        (Method::GET, "/v1/tasks/task-disabled", Value::Null),
        (
            Method::PATCH,
            "/v1/tasks/task-disabled",
            json!({
                "status": "blocked",
                "blocked_reason": "Waiting for rollout"
            }),
        ),
    ];

    for (method, uri, payload) in cases {
        let (status, body) =
            api_json_payload_request(router.clone(), &operator, method, uri.to_string(), payload)
                .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}: {body}");
    }
}

#[tokio::test]
async fn disabled_remote_ssh_plugin_does_not_register_routes() {
    let _env = ChannelTaskEnvGuard::disabled();
    let database = TestDatabase::create().await;
    let router = router_with_db(choruz_application::ChatApp::new(), &database.database_url);
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/ssh/hosts")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn legacy_ssh_connect_route_is_not_registered() {
    let database = TestDatabase::create().await;
    let router = router_with_db(choruz_application::ChatApp::new(), &database.database_url);
    let legacy_route = ["/v1/ssh/connect-", "e", "chat"].concat();
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(legacy_route)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
