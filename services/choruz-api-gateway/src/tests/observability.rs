use super::*;

#[tokio::test]
async fn activity_tools_scope_page_aggregate_and_transactionally_prune() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let mut actors = Vec::new();
    for name in ["owner", "other"] {
        let actor = app
            .create_principal(CreatePrincipalRequest {
                workspace_id: "activity-tools".into(),
                principal_type: PrincipalType::Human,
                name: name.into(),
                avatar_url: None,
            })
            .unwrap();
        seed_principal_to_db(&database.database_url, &actor).await;
        actors.push(actor);
    }
    let agent = app
        .create_agent(CreateAgentRequest {
            actor_id: actors[0].id.clone(),
            name: "tools agent".into(),
            scopes: vec![],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    seed_principal_to_db(&database.database_url, &agent).await;
    let router = router_with_db(app, &database.database_url);
    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });
    for actor in &actors {
        let events:Vec<_> = (1..=3).map(|n|json!({"eventId":format!("e{n}"),"schemaVersion":1,"traceId":"tools-trace","spanId":"span","sessionId":"session","name":"http_request.finished","ts":"2026-01-02T00:00:00Z","durationMs":n*10,"data":{"outcome":if n==1 {"failed"} else {"succeeded"},"password":"private-secret"}})).collect();
        let (status, _) = api_json_payload_request(
            router.clone(),
            actor,
            Method::POST,
            "/v1/telemetry".into(),
            json!({"events":events}),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }
    client
        .execute(
            "UPDATE telemetry_event SET created_at='2026-01-02T00:00:00Z'",
            &[],
        )
        .await
        .unwrap();
    client.execute("INSERT INTO audit_log(id,workspace_id,actor_id,action,target_type,target_id,metadata,created_at) VALUES('audit-owned','activity-tools',$1,'terminal.submit_finished','binding','binding-id','{\"trace_id\":\"tools-trace\",\"outcome\":\"failed\",\"prompt\":\"private-prompt\"}','2026-01-02T00:00:00Z')",&[&actors[0].id]).await.unwrap();
    client
        .execute(
            "INSERT INTO company(id,name,slug,owner_id) VALUES('tools-company','tools','tools',$1)",
            &[&actors[0].id],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO company_member(company_id,principal_id) VALUES('tools-company',$1)",
            &[&actors[0].id],
        )
        .await
        .unwrap();
    client.execute("INSERT INTO audit_log(id,workspace_id,actor_id,action,target_type,target_id,metadata,created_at) VALUES('company-audit','tools-company',$1,'company.updated','company','tools-company','{\"trace_id\":\"tools-trace\"}','2026-01-02T00:00:00Z')",&[&actors[0].id]).await.unwrap();
    let range = "since=2026-01-01T00:00:00Z&until=2026-01-03T00:00:00Z&trace_id=tools-trace";
    let (status, page) = api_json_request(
        router.clone(),
        &actors[0],
        Method::GET,
        format!("/v1/activity?{range}&limit=2"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["records"].as_array().unwrap().len(), 2);
    assert!(!page.to_string().contains("private-secret"));
    let (_, last) = api_json_request(
        router.clone(),
        &actors[0],
        Method::GET,
        format!(
            "/v1/activity?{range}&limit=2&cursor={}",
            page["next_cursor"].as_str().unwrap()
        ),
    )
    .await;
    assert_eq!(last["records"].as_array().unwrap().len(), 1);
    assert!(last["next_cursor"].is_null());
    assert_ne!(page["records"][0]["id"], last["records"][0]["id"]);
    let (_, summary) = api_json_request(
        router.clone(),
        &actors[0],
        Method::GET,
        format!("/v1/activity/summary?{range}"),
    )
    .await;
    assert_eq!(summary["groups"][0]["count"], 3);
    assert_eq!(summary["groups"][0]["failed"], 1);
    assert_eq!(summary["groups"][0]["mean_duration_ms"], 20.0);
    let (_, audit) = api_json_request(
        router.clone(),
        &actors[0],
        Method::GET,
        format!("/v1/activity?{range}&source=audit"),
    )
    .await;
    assert_eq!(audit["records"].as_array().unwrap().len(), 2);
    assert!(
        audit["records"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["data"]["outcome"] == "failed")
    );
    assert!(!audit.to_string().contains("private-prompt"));
    client
        .execute(
            "DELETE FROM company_member WHERE company_id='tools-company'",
            &[],
        )
        .await
        .unwrap();
    let (_, without_company) = api_json_request(
        router.clone(),
        &actors[0],
        Method::GET,
        format!("/v1/activity?{range}&source=audit"),
    )
    .await;
    assert_eq!(without_company["records"].as_array().unwrap().len(), 1);
    assert_eq!(
        api_json_request(
            router.clone(),
            &agent,
            Method::GET,
            format!("/v1/activity?{range}")
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    for path in [
        format!("/v1/activity?{range}&limit=0"),
        format!("/v1/activity?{range}&cursor=bad"),
        format!("/v1/activity?{range}&actor_id=other"),
        "/v1/activity?since=2026-01-01T00:00:00Z&until=2026-03-01T00:00:00Z".into(),
    ] {
        assert_eq!(
            api_json_request(router.clone(), &actors[0], Method::GET, path)
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    let body = json!({"before":"2026-01-03T00:00:00Z"});
    let (_, preview) = api_json_payload_request(
        router.clone(),
        &actors[0],
        Method::POST,
        "/v1/activity/prune".into(),
        body,
    )
    .await;
    assert_eq!(preview["count"], 3);
    assert_eq!(preview["applied"], false);
    client.batch_execute("ALTER TABLE audit_log ADD CONSTRAINT reject_retention_audit CHECK(action != 'activity.pruned')").await.unwrap();
    let apply = json!({"before":"2026-01-03T00:00:00Z","apply":true});
    assert_eq!(
        api_json_payload_request(
            router.clone(),
            &actors[0],
            Method::POST,
            "/v1/activity/prune".into(),
            apply.clone()
        )
        .await
        .0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        client
            .query_one(
                "SELECT count(*) FROM telemetry_event WHERE principal_id=$1",
                &[&actors[0].id]
            )
            .await
            .unwrap()
            .get::<_, i64>(0),
        3,
        "delete rolls back when audit cannot commit"
    );
    client
        .batch_execute("ALTER TABLE audit_log DROP CONSTRAINT reject_retention_audit")
        .await
        .unwrap();
    let (status, done) = api_json_payload_request(
        router.clone(),
        &actors[0],
        Method::POST,
        "/v1/activity/prune".into(),
        apply,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(done["count"], 3);
    assert_eq!(done["applied"], true);
    assert_eq!(
        client
            .query_one(
                "SELECT count(*) FROM telemetry_event WHERE principal_id=$1",
                &[&actors[1].id]
            )
            .await
            .unwrap()
            .get::<_, i64>(0),
        3
    );
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM audit_log WHERE id='audit-owned'", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    assert_eq!(
        client
            .query_one(
                "SELECT count(*) FROM audit_log WHERE actor_id=$1 AND action='activity.pruned'",
                &[&actors[0].id]
            )
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    client.execute("INSERT INTO telemetry_event(workspace_id,principal_id,event_id,schema_version,trace_id,span_id,session_id,name,occurred_at,created_at) SELECT 'activity-tools',$1,'batch-'||n,1,'batch','span','session','click','2026-01-02T00:00:00Z','2026-01-02T00:00:00Z' FROM generate_series(1,1002) AS n",&[&actors[0].id]).await.unwrap();
    let batch = json!({"before":"2026-01-03T00:00:00Z","apply":true});
    let (_, first) = api_json_payload_request(
        router.clone(),
        &actors[0],
        Method::POST,
        "/v1/activity/prune".into(),
        batch.clone(),
    )
    .await;
    assert_eq!(first["count"], 1000);
    assert_eq!(first["has_more"], true);
    let (_, second) = api_json_payload_request(
        router.clone(),
        &actors[0],
        Method::POST,
        "/v1/activity/prune".into(),
        batch,
    )
    .await;
    assert_eq!(second["count"], 2);
    assert_eq!(second["has_more"], false);
    assert_eq!(
        api_json_payload_request(
            router.clone(),
            &agent,
            Method::POST,
            "/v1/activity/prune".into(),
            json!({"before":"2026-01-03T00:00:00Z"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        api_json_payload_request(
            router.clone(),
            &actors[0],
            Method::POST,
            "/v1/activity/prune".into(),
            json!({"before":chrono::Utc::now()})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let metrics = choruz_common::metrics::text();
    assert!(metrics.contains("choruz_activity_batches_total{outcome=\"committed\"}"));
    assert!(metrics.contains("choruz_http_responses_total{class=\"4xx\"}"));
}

#[tokio::test]
async fn telemetry_batches_are_atomic_idempotent_and_actor_scoped() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let mut actors = Vec::new();
    for workspace in ["activity-a", "activity-b"] {
        let actor = app
            .create_principal(CreatePrincipalRequest {
                workspace_id: workspace.into(),
                principal_type: PrincipalType::Human,
                name: workspace.into(),
                avatar_url: None,
            })
            .unwrap();
        seed_principal_to_db(&database.database_url, &actor).await;
        actors.push(actor);
    }
    let router = router_with_db(app, &database.database_url);
    let event = json!({"eventId":"same-id", "schemaVersion":1, "traceId":"trace", "spanId":"span", "sessionId":"session", "name":"ui_click", "ts":"2026-01-01T00:00:00Z"});
    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });

    // Force the second insert to fail in this test's private database.
    client.batch_execute("ALTER TABLE telemetry_event ADD CONSTRAINT activity_test_failure CHECK (event_id != 'fail')").await.unwrap();
    let mut failing = event.clone();
    failing["eventId"] = json!("fail");
    let (status, _) = api_json_payload_request(
        router.clone(),
        &actors[0],
        Method::POST,
        "/v1/telemetry".into(),
        json!({"events":[event, failing]}),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let count: i64 = client
        .query_one("SELECT count(*) FROM telemetry_event", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(count, 0, "the first insert must roll back too");

    let mut invalid = event.clone();
    invalid["schemaVersion"] = json!(99);
    let (status, _) = api_json_payload_request(
        router.clone(),
        &actors[0],
        Method::POST,
        "/v1/telemetry".into(),
        json!({"events":[event, invalid]}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    for actor in &actors {
        for _ in 0..2 {
            let (status, _) = api_json_payload_request(
                router.clone(),
                actor,
                Method::POST,
                "/v1/telemetry".into(),
                json!({"events":[event]}),
            )
            .await;
            assert_eq!(status, StatusCode::NO_CONTENT);
        }
        let row = client
            .query_one(
                "SELECT count(*), min(workspace_id) FROM telemetry_event WHERE principal_id=$1",
                &[&actor.id],
            )
            .await
            .unwrap();
        assert_eq!(row.get::<_, i64>(0), 1);
        assert_eq!(row.get::<_, String>(1), actor.workspace_id);
    }
}

#[tokio::test]
async fn metrics_endpoint_reports_prometheus_text() {
    let app = choruz_application::ChatApp::new();
    app.create_principal(CreatePrincipalRequest {
        workspace_id: "ws-acme".into(),
        principal_type: PrincipalType::Human,
        name: "Alice".into(),
        avatar_url: None,
    })
    .unwrap();

    let response = router(app)
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/plain; version=0.0.4")
    );

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    let has_line = |expected: &str| text.lines().any(|line| line == expected);
    assert!(has_line("# TYPE choruz_principals_total gauge"));
    assert!(has_line("choruz_principals_total 1"));
    for gauge in [
        "choruz_conversations_total",
        "choruz_messages_total",
        "choruz_audit_logs_total",
        "choruz_event_backlog_total",
    ] {
        assert!(has_line(&format!("# TYPE {gauge} gauge")), "{gauge}");
    }
    assert!(has_line("# TYPE choruz_http_requests_total counter"));
    assert!(prometheus_metric_value(&text, "choruz_http_requests_total") >= 1);
    assert!(has_line("# TYPE choruz_http_request_duration histogram"));
    for bucket in ["0.05", "0.2", "1", "+Inf"] {
        assert!(
            text.contains(&format!(
                "choruz_http_request_duration_bucket{{le=\"{bucket}\"}} "
            )),
            "missing latency bucket {bucket}"
        );
    }
    for counter in [
        "choruz_channel_task_creates_total",
        "choruz_channel_task_updates_total",
        "choruz_channel_task_mutation_errors_total",
        "choruz_channel_task_load_errors_total",
    ] {
        assert!(has_line(&format!("# TYPE {counter} counter")), "{counter}");
    }
    assert!(text.contains("choruz_channel_task_creates_total"));
    assert!(text.contains("choruz_channel_task_updates_total"));
    assert!(text.contains("choruz_channel_task_mutation_errors_total"));
    assert!(text.contains("choruz_channel_task_load_errors_total"));
    let legacy_metric_prefix = ["e", "chat_"].concat();
    assert!(
        !text.contains(&legacy_metric_prefix),
        "metrics must not dual-emit legacy names"
    );
    assert_eq!(
        prometheus_metric_value(&text, "choruz_channel_task_load_errors_total"),
        0
    );
    assert!(!text.contains("choruz_channel_task_open_total{status="));
}

#[tokio::test]
async fn api_error_responses_redact_secrets() {
    let response = ApiError(AppError::Internal(
        "secret=sk-live-123 Bearer token-456".into(),
    ))
    .into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let detail = payload["error"]["detail"].as_str().unwrap();
    assert!(detail.contains("REDACTED"));
    assert!(!detail.contains("sk-live-123"));
    assert!(!detail.contains("token-456"));
}

#[tokio::test]
async fn webhook_deliveries_retry_until_success() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let operator = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-acme".into(),
            principal_type: PrincipalType::Human,
            name: "Operator".into(),
            avatar_url: None,
        })
        .unwrap();
    let human = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-acme".into(),
            principal_type: PrincipalType::Human,
            name: "Human".into(),
            avatar_url: None,
        })
        .unwrap();
    let agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "WebhookBot".into(),
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

    // Seed data to DB for DbService lookups
    seed_principal_to_db(&database.database_url, &operator).await;
    seed_principal_to_db(&database.database_url, &human).await;
    seed_principal_to_db(&database.database_url, &agent.principal).await;
    seed_conversation_to_db(&database.database_url, &conversation).await;

    let webhook_state = WebhookReceiverState {
        attempts: Arc::new(AtomicUsize::new(0)),
        payloads: Arc::new(Mutex::new(Vec::new())),
        headers: Arc::new(Mutex::new(Vec::new())),
    };
    let webhook_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let webhook_address = webhook_listener.local_addr().unwrap();
    let webhook_server_state = webhook_state.clone();
    let webhook_server = tokio::spawn(async move {
        axum::serve(
            webhook_listener,
            Router::new()
                .route("/hook", post(webhook_receiver))
                .with_state(webhook_server_state),
        )
        .await
        .unwrap();
    });

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn({
        let test_router = router_with_db(app.clone(), &database.database_url);
        async move {
            axum::serve(listener, test_router).await.unwrap();
        }
    });

    let client = Client::new();
    let configure = client
        .post(format!(
            "http://{address}/v1/principals/{}/event-webhook",
            agent.principal.id
        ))
        .header(
            "authorization",
            format!("Bearer {}", session_token(&operator)),
        )
        .json(&SetEventWebhookRequest {
            actor_id: operator.id.clone(),
            url: format!("http://{webhook_address}/hook"),
            event_types: vec!["message.created".into()],
            secret: None,
        })
        .send()
        .await
        .unwrap();
    assert_eq!(configure.status(), StatusCode::OK);

    let send = client
        .post(format!("http://{address}/v1/messages"))
        .header("authorization", format!("Bearer {}", session_token(&human)))
        .json(&SendMessageRequest {
            actor_id: human.id.clone(),
            conversation_id: conversation.id,
            idempotency_key: "hook-msg".into(),
            content: "deliver".into(),
            content_type: "text".into(),
            metadata: json!({}),
            trace_id: None,
        })
        .send()
        .await
        .unwrap();
    assert_eq!(send.status(), StatusCode::CREATED);

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        webhook_state.attempts.load(Ordering::SeqCst),
        1,
        "initial delivery should fail once"
    );

    let flush = client
        .post(format!("http://{address}/v1/webhooks/flush"))
        .header(
            "authorization",
            format!("Bearer {}", session_token(&operator)),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(flush.status(), StatusCode::OK);
    let flush_body: Value = flush.json().await.unwrap();
    assert_eq!(flush_body["attempted"], 1);
    assert_eq!(flush_body["delivered"], 1);

    let payload = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(payload) = webhook_state.payloads.lock().await.first().cloned() {
                return payload;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(payload["event_type"], "message.created");
    assert!(payload["event_id"].as_str().is_some());
    let headers = webhook_state.headers.lock().await;
    let headers = headers.first().expect("captured webhook headers");
    assert!(headers.contains_key("x-choruz-event-id"));
    assert!(headers.contains_key("x-choruz-timestamp"));
    assert!(headers.contains_key("x-choruz-signature"));
    let legacy_prefix = ["e", "chat"].concat();
    assert!(!headers.contains_key(format!("x-{legacy_prefix}-event-id")));
    assert!(!headers.contains_key(format!("x-{legacy_prefix}-timestamp")));
    assert!(!headers.contains_key(format!("x-{legacy_prefix}-signature")));
    assert!(app.collect_pending_webhook_deliveries().is_empty());

    server.abort();
    webhook_server.abort();
}

#[tokio::test]
async fn telemetry_ingest_redacts_sensitive_payloads_before_persisting() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let principal = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "telem002-workspace".into(),
            principal_type: PrincipalType::Human,
            name: "Telem002 User".into(),
            avatar_url: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &principal).await;
    let router = router_with_db(app, &database.database_url);

    let (status, _) = api_json_payload_request(
        router,
        &principal,
        Method::POST,
        "/v1/telemetry".into(),
        json!({
            "events": [{
                "name": "telem002_sensitive_payload",
                "eventId": choruz_common::new_id(),
                "schemaVersion": 1,
                "spanId": "span",
                "sessionId": "session",
                "ts": chrono::Utc::now(),
                "traceId": "trace-telem002",
                "durationMs": 7,
                "data": {
                    "conversation_id": "conv-safe-correlation",
                    "content_len": 27,
                    "authenticationCode": "sensitive-auth-code",
                    "pairing_credential": "sensitive-pairing-credential",
                    "session_token": "session-token-test-value",
                    "agent_secret": "agent-secret-test-value",
                    "authorization": "Bearer bearer-token-test-value",
                    "workspace_path": "/Users/alice/private/team007-workspace",
                    "workspacePath": "/Users/alice/private/camel-workspace",
                    "file_path": "/tmp/team007-private.txt",
                    "filePath": "/tmp/camel-private.txt",
                    "payloadBase64": "camel-payload-base64-test-value",
                    "message": {
                        "private": true,
                        "content": "private-content-test-value",
                        "preview": "private-preview-test-value"
                    },
                    "attachment": {
                        "filename": "safe-name.txt",
                        "fileName": "camel-safe-name.txt",
                        "size_bytes": 42,
                        "attachment_bytes": "attachment-bytes-test-value",
                        "data_base64": "attachment-base64-test-value"
                    }
                }
            }]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect for telemetry assertion");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let row = client
        .query_one(
            "SELECT data FROM telemetry_event WHERE principal_id = $1 AND name = $2",
            &[&principal.id, &"telem002_sensitive_payload"],
        )
        .await
        .expect("query telemetry event");
    let data: Value = row.get("data");
    let serialized = data.to_string();

    for sensitive in [
        "sensitive-auth-code",
        "sensitive-pairing-credential",
        "session-token-test-value",
        "agent-secret-test-value",
        "bearer-token-test-value",
        "private-content-test-value",
        "private-preview-test-value",
        "attachment-bytes-test-value",
        "attachment-base64-test-value",
        "safe-name.txt",
        "camel-safe-name.txt",
        "/Users/alice/private/team007-workspace",
        "/Users/alice/private/camel-workspace",
        "/tmp/team007-private.txt",
        "/tmp/camel-private.txt",
        "camel-payload-base64-test-value",
    ] {
        assert!(
            !serialized.contains(sensitive),
            "telemetry persisted sensitive value {sensitive}: {serialized}"
        );
    }

    assert_eq!(data["conversation_id"], "conv-safe-correlation");
    assert_eq!(data["content_len"], 27);
    assert_eq!(data["attachment"]["filename"], "[REDACTED]");
    assert_eq!(data["attachment"]["fileName"], "[REDACTED]");
    assert_eq!(data["workspace_path"], "[REDACTED]");
    assert_eq!(data["workspacePath"], "[REDACTED]");
    assert_eq!(data["file_path"], "[REDACTED]");
    assert_eq!(data["filePath"], "[REDACTED]");
    assert_eq!(data["payloadBase64"], "[REDACTED]");
    assert_eq!(data["attachment"]["size_bytes"], 42);
    assert_eq!(data["session_token"], "[REDACTED]");
    assert_eq!(data["agent_secret"], "[REDACTED]");
    assert_eq!(data["message"]["content"], "[REDACTED]");
    assert_eq!(data["message"]["preview"], "[REDACTED]");
    assert_eq!(data["attachment"]["attachment_bytes"], "[REDACTED]");
    assert_eq!(data["attachment"]["data_base64"], "[REDACTED]");
}

// ─────────────────────────────────────────────────────────────────────
// Message threads — Phase 1 write path
// ─────────────────────────────────────────────────────────────────────
