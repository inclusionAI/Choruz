use super::*;

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
                "traceId": "trace-telem002",
                "durationMs": 7,
                "data": {
                    "conversation_id": "conv-safe-correlation",
                    "content_len": 27,
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
