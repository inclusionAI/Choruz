use super::*;

#[tokio::test]
async fn conversation_export_includes_channel_tasks_with_safe_projection() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let operator = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-channel-task-export".into(),
            principal_type: PrincipalType::Human,
            name: "Export Operator".into(),
            avatar_url: None,
        })
        .unwrap();
    let non_member = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Export Non Member".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let member = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Export Member".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let visible_agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Export Visible Agent".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let internal_agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Export Internal Agent".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: Some(choruz_domain::ChannelVisibility::Internal),
        })
        .unwrap()
        .principal;
    let conversation = app
        .create_group(CreateGroupRequest {
            actor_id: operator.id.clone(),
            name: "Channel Task Export".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![member.id.clone(), visible_agent.id.clone()],
            workspace_id: None,
        })
        .unwrap();
    let other_conversation = app
        .create_group(CreateGroupRequest {
            actor_id: operator.id.clone(),
            name: "Other Channel Task Export".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![member.id.clone(), visible_agent.id.clone()],
            workspace_id: None,
        })
        .unwrap();
    for principal in [
        &operator,
        &member,
        &non_member,
        &visible_agent,
        &internal_agent,
    ] {
        seed_principal_to_db(&database.database_url, principal).await;
    }
    seed_conversation_to_db(&database.database_url, &conversation).await;
    seed_conversation_to_db(&database.database_url, &other_conversation).await;

    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect for channel task export seed");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "
            INSERT INTO conversation_events
              (conversation_id, seq, event_id, event_type, sender_id, content,
               content_type, metadata, client_msg_id, created_at)
            VALUES
              ($1, 1, 'export-message-1', 'message', $2, 'exported message', 'text/plain',
               '{}'::jsonb, 'export-message-client-1', NOW()),
              ($1, 2, 'export-system-event', 'system', $2, 'internal event', 'text/plain',
               '{}'::jsonb, 'export-system-client-1', NOW()),
              ($3, 1, 'other-export-message', 'message', $2, 'other message', 'text/plain',
               '{}'::jsonb, 'other-export-client-1', NOW())
            ",
            &[&conversation.id, &member.id, &other_conversation.id],
        )
        .await
        .expect("seed export messages");
    client
        .execute(
            "
            INSERT INTO group_workflow_task
              (id, conversation_id, task_key, title, status, assignee_principal_id,
               blocked_reason, context_label, source_kind, source_message_id, created_by, version)
            VALUES
              ('export-visible-task', $1, 'EXP-1', 'Export visible task', 'in_progress', $2,
               NULL, 'Release prep', 'message', 'export-message-1', $3, 2),
              ('export-internal-task', $1, 'EXP-2', 'Export internal task', 'blocked', $4,
               'Waiting on safe input', 'Private runtime', 'message', 'export-system-event', $4, 1),
              ('other-export-task', $5, 'OTHER-1', 'Other export task', 'todo', $2,
               NULL, NULL, 'message', 'other-export-message', $3, 1)
            ",
            &[
                &conversation.id,
                &visible_agent.id,
                &member.id,
                &internal_agent.id,
                &other_conversation.id,
            ],
        )
        .await
        .expect("seed export channel tasks");
    client
        .execute(
            "
            INSERT INTO group_workflow_event
              (id, conversation_id, task_id, actor_principal_id, kind, payload,
               resulting_version, created_at)
            VALUES
              ('export-safe-event', $1, 'export-visible-task', $2, 'task.started',
               jsonb_build_object(
                 'previous', jsonb_build_object(
                   'status', 'todo',
                   'assignee_principal_id', $2::TEXT,
                   'source_message_id', 'export-system-event'
                 ),
                 'new', jsonb_build_object(
                   'status', 'in_progress',
                   'assignee_principal_id', $2::TEXT,
                   'source_message_id', 'export-message-1'
                 ),
                 'metadata', jsonb_build_object('workflow', 'secret-workflow'),
                 'routing_role_map', jsonb_build_object('owner', $2::TEXT),
                 'subagent_lineage', jsonb_build_array($3::TEXT),
                 'tool_diagnostics', jsonb_build_object('secret', true),
                 'idempotency_payload_hash', 'hash-that-must-not-export',
                 'reason_code', 'private_runtime_label'
               ),
               2, NOW()),
              ('export-internal-actor-event', $1, 'export-visible-task', $3, 'task.blocked',
               jsonb_build_object(
                 'new', jsonb_build_object(
                   'status', 'blocked',
                   'assignee_principal_id', $3::TEXT
                 )
               ),
               3, NOW() + INTERVAL '1 second'),
              ('export-unauthorized-diagnostic-event', $1, 'export-visible-task', $3, 'external_check.passed',
               jsonb_build_object(
                 'workflow_diagnostic', jsonb_build_object(
                   'reason_code', 'workflow_status_unauthorized'
                 ),
                 'new', jsonb_build_object('status', 'done')
               ),
               NULL, NOW() + INTERVAL '2 seconds'),
              ('export-mismatched-conversation-event', $1, 'other-export-task', $2, 'task.started',
               jsonb_build_object('new', jsonb_build_object('status', 'done')),
               9, NOW() + INTERVAL '3 seconds'),
              ('other-conversation-event', $4, 'other-export-task', $2, 'task.started',
               jsonb_build_object('new', jsonb_build_object('status', 'done')),
               2, NOW() + INTERVAL '4 seconds')
            ",
            &[
                &conversation.id,
                &non_member.id,
                &internal_agent.id,
                &other_conversation.id,
            ],
        )
        .await
        .expect("seed export channel task events");

    let router = router_with_db(app, &database.database_url);
    let (non_member_export_status, _) = api_json_request(
        router.clone(),
        &non_member,
        Method::GET,
        format!(
            "/v1/export/conversations/{}?actor_id={}",
            conversation.id, non_member.id
        ),
    )
    .await;
    assert_eq!(non_member_export_status, StatusCode::FORBIDDEN);

    let (export_status, export_body) = api_json_request(
        router,
        &member,
        Method::GET,
        format!(
            "/v1/export/conversations/{}?actor_id={}",
            conversation.id, member.id
        ),
    )
    .await;
    assert_eq!(export_status, StatusCode::OK);
    assert_eq!(export_body["conversation"]["id"], conversation.id);
    assert_eq!(export_body["messages"].as_array().unwrap().len(), 1);
    assert_eq!(export_body["messages"][0]["id"], "export-message-1");
    assert_eq!(export_body["messages"][0]["content"], "exported message");

    let task_exports = export_body["channel_tasks"]
        .as_array()
        .expect("exported channel tasks");
    assert_eq!(task_exports.len(), 2);
    assert_eq!(task_exports[0]["task"]["task_id"], "export-visible-task");
    assert_eq!(task_exports[0]["task"]["conversation_id"], conversation.id);
    assert_eq!(task_exports[0]["task"]["task_key"], "EXP-1");
    assert_eq!(
        task_exports[0]["task"]["assignee_principal_id"],
        visible_agent.id
    );
    assert_eq!(
        task_exports[0]["task"]["source_message_id"],
        "export-message-1"
    );
    assert_eq!(task_exports[1]["task"]["task_id"], "export-internal-task");
    assert_eq!(task_exports[1]["task"].get("assignee_principal_id"), None);
    assert_eq!(task_exports[1]["task"].get("created_by"), None);
    assert_eq!(task_exports[1]["task"].get("source_message_id"), None);
    assert_eq!(
        task_exports[1]["task"]["blocked_reason"],
        "Waiting on safe input"
    );

    let events = task_exports[0]["events"].as_array().expect("task events");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["event_id"], "export-safe-event");
    assert_eq!(events[0]["kind"], "channel_task.workflow_event");
    assert_eq!(events[0]["workflow_kind"], "task.started");
    assert_eq!(events[0]["status_effect"], "in_progress");
    assert_eq!(events[0]["new"]["status"], "in_progress");
    assert_eq!(events[0]["new"]["source_message_id"], "export-message-1");
    assert_eq!(events[0]["new"].get("assignee_principal_id"), None);
    assert_eq!(events[0]["previous"].get("source_message_id"), None);
    assert_eq!(events[0].get("actor_principal_id"), None);
    assert_eq!(events[1]["event_id"], "export-internal-actor-event");
    assert_eq!(events[1].get("actor_principal_id"), None);
    assert_eq!(events[1]["new"].get("assignee_principal_id"), None);
    assert!(task_exports[1]["events"].as_array().unwrap().is_empty());

    let channel_task_payload = serde_json::to_string(&export_body["channel_tasks"]).unwrap();
    for forbidden in [
        "secret-workflow",
        "routing_role_map",
        "subagent_lineage",
        "tool_diagnostics",
        "hash-that-must-not-export",
        "private_runtime_label",
        "workflow_status_unauthorized",
        "export-unauthorized-diagnostic-event",
        "export-mismatched-conversation-event",
        "other-conversation-event",
        "other-export-task",
        non_member.id.as_str(),
        internal_agent.id.as_str(),
    ] {
        assert!(
            !channel_task_payload.contains(forbidden),
            "channel task export leaked forbidden value: {forbidden}"
        );
    }
}

#[tokio::test]
async fn channel_task_real_internal_agent_provisioning_rejects_assignment_and_redacts_surfaces() {
    let _env = ChannelTaskEnvGuard::enabled();
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let operator = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-channel-task-real-internal".into(),
            principal_type: PrincipalType::Human,
            name: "Operator".into(),
            avatar_url: None,
        })
        .unwrap();
    let human = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: operator.workspace_id.clone(),
            principal_type: PrincipalType::Human,
            name: "Human".into(),
            avatar_url: None,
        })
        .unwrap();
    let visible_agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Visible Agent".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    for principal in [&operator, &human, &visible_agent] {
        seed_principal_to_db(&database.database_url, principal).await;
    }

    let internal_agent_name = "Real Internal Delegate";
    let router = router_with_db(app.clone(), &database.database_url);
    let (internal_create_status, internal_create_body) = api_json_payload_request(
        router.clone(),
        &operator,
        Method::POST,
        "/v1/agents".to_string(),
        json!({
            "actor_id": operator.id.clone(),
            "name": internal_agent_name,
            "scopes": ["messages:read"],
            "channel_visibility": "internal"
        }),
    )
    .await;
    assert_eq!(internal_create_status, StatusCode::CREATED);
    let internal_agent: choruz_domain::Principal =
        serde_json::from_value(internal_create_body["principal"].clone())
            .expect("deserialize created internal principal");
    assert_eq!(
        internal_agent.channel_visibility,
        choruz_domain::ChannelVisibility::Internal
    );
    assert!(
        app.get_principal(&internal_agent.id).is_ok(),
        "POST /v1/agents should mirror the created agent into ChatApp state"
    );

    let conversation = app
        .create_group(CreateGroupRequest {
            actor_id: operator.id.clone(),
            name: "Real Internal Agent Redaction".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![
                human.id.clone(),
                visible_agent.id.clone(),
                internal_agent.id.clone(),
            ],
            workspace_id: None,
        })
        .unwrap();
    seed_conversation_to_db(&database.database_url, &conversation).await;

    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect for real internal channel task seed");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "
            INSERT INTO conversation_events
              (conversation_id, seq, event_id, event_type, sender_id, content, content_type)
            VALUES
              ($1, 1, 'real-internal-message-1', 'message', $2, 'Track this safely', 'text')
            ",
            &[&conversation.id, &human.id],
        )
        .await
        .expect("seed real internal source message");

    let (internal_assignee_status, internal_assignee_body) = api_json_payload_request(
        router.clone(),
        &human,
        Method::POST,
        format!("/v1/conversations/{}/tasks/from-message", conversation.id),
        json!({
            "message_id": "real-internal-message-1",
            "title": "Internal delegate must not own board work",
            "assignee_principal_id": internal_agent.id.clone(),
            "idempotency_key": "real-internal-rejected-1"
        }),
    )
    .await;
    assert_eq!(internal_assignee_status, StatusCode::BAD_REQUEST);
    assert_safe_detail_excludes(
        &internal_assignee_body,
        &internal_agent.id,
        &[internal_agent_name],
        "real internal assignee rejection",
    );

    let (visible_assignee_status, visible_task) = api_json_payload_request(
        router.clone(),
        &human,
        Method::POST,
        format!("/v1/conversations/{}/tasks/from-message", conversation.id),
        json!({
            "message_id": "real-internal-message-1",
            "title": "Visible task still works",
            "assignee_principal_id": visible_agent.id.clone(),
            "idempotency_key": "real-internal-visible-1"
        }),
    )
    .await;
    assert_eq!(visible_assignee_status, StatusCode::CREATED);
    assert_eq!(visible_task["assignee_principal_id"], visible_agent.id);

    client
        .execute(
            "
            INSERT INTO group_workflow_task
              (id, conversation_id, task_key, title, status, assignee_principal_id,
               source_kind, source_message_id, created_by, version)
            VALUES
              ('real-internal-polluted-task', $1, 'INT-1', 'Polluted internal assignment',
               'todo', $2, 'agent', NULL, $2, 4)
            ",
            &[&conversation.id, &internal_agent.id],
        )
        .await
        .expect("seed task with real internal assignee");

    let (list_status, list_body) = api_json_request(
        router.clone(),
        &human,
        Method::GET,
        format!("/v1/conversations/{}/tasks", conversation.id),
    )
    .await;
    assert_eq!(list_status, StatusCode::OK);
    let list_serialized = serde_json::to_string(&list_body).expect("serialize task list");
    assert!(
        !list_serialized.contains(&internal_agent.id),
        "safe task list projection leaked real internal agent id"
    );
    assert!(
        !list_serialized.contains(internal_agent_name),
        "safe task list projection leaked real internal agent name"
    );

    let (patch_status, patched) = api_json_payload_request(
        router.clone(),
        &human,
        Method::PATCH,
        "/v1/tasks/real-internal-polluted-task".to_string(),
        json!({
            "status": "in_progress"
        }),
    )
    .await;
    assert_eq!(patch_status, StatusCode::OK);
    assert_eq!(patched.get("assignee_principal_id"), None);
    assert_eq!(patched.get("created_by"), None);

    let fanout_metadata: Value = client
        .query_one(
            "
            SELECT metadata
            FROM conversation_events
            WHERE conversation_id = $1
              AND event_type = 'channel_task.updated'
              AND metadata->>'task_id' = 'real-internal-polluted-task'
            ORDER BY seq DESC
            LIMIT 1
            ",
            &[&conversation.id],
        )
        .await
        .expect("load real internal fanout metadata")
        .get("metadata");
    let fanout_serialized =
        serde_json::to_string(&fanout_metadata).expect("serialize fanout metadata");
    assert!(
        !fanout_serialized.contains(&internal_agent.id),
        "fanout safe projection leaked real internal agent id"
    );
    assert!(
        !fanout_serialized.contains(internal_agent_name),
        "fanout safe projection leaked real internal agent name"
    );
    assert_eq!(
        fanout_metadata["task"].get("assignee_principal_id"),
        None,
        "fanout task snapshot must omit internal assignee"
    );

    let (export_status, export_body) = api_json_request(
        router,
        &human,
        Method::GET,
        format!(
            "/v1/export/conversations/{}?actor_id={}",
            conversation.id, human.id
        ),
    )
    .await;
    assert_eq!(export_status, StatusCode::OK);
    let export_serialized = serde_json::to_string(&export_body["channel_tasks"])
        .expect("serialize channel task export");
    assert!(
        !export_serialized.contains(&internal_agent.id),
        "conversation export leaked real internal agent id"
    );
    assert!(
        !export_serialized.contains(internal_agent_name),
        "conversation export leaked real internal agent name"
    );
}

#[tokio::test]
async fn metrics_endpoint_reports_channel_task_mutation_counters() {
    let _guard = ChannelTaskEnvGuard::enabled();
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let actor = LocalAuthConfig::from_env()
        .ensure_operator_sync(&app)
        .unwrap();
    let assignee = app
        .create_agent(CreateAgentRequest {
            actor_id: actor.id.clone(),
            name: "Counter Agent".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let conversation = app
        .create_group(CreateGroupRequest {
            actor_id: actor.id.clone(),
            name: "Counter Group".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![assignee.id.clone()],
            workspace_id: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &actor).await;
    seed_principal_to_db(&database.database_url, &assignee).await;
    seed_conversation_to_db(&database.database_url, &conversation).await;

    let router = router_with_db(app, &database.database_url);
    let before = metrics_text(router.clone()).await;
    let creates_before = prometheus_metric_value(&before, "choruz_channel_task_creates_total");
    let updates_before = prometheus_metric_value(&before, "choruz_channel_task_updates_total");
    let errors_before =
        prometheus_metric_value(&before, "choruz_channel_task_mutation_errors_total");

    let create_response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/v1/conversations/{}/tasks", conversation.id))
                .header(
                    "authorization",
                    format!("Bearer {}", session_token(&assignee)),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_vec(&CreateChannelTaskRequest {
                        task_key: Some("CTR-1".into()),
                        title: "Counter task".into(),
                        assignee_principal_id: None,
                        status: None,
                        context_label: None,
                        idempotency_key: "counter-create-1".into(),
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_response.status(), StatusCode::CREATED);
    let body = to_bytes(create_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let task: choruz_application::ChannelTaskSnapshot = serde_json::from_slice(&body).unwrap();

    let patch_response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PATCH)
                .uri(format!("/v1/tasks/{}", task.task_id))
                .header(
                    "authorization",
                    format!("Bearer {}", session_token(&assignee)),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_vec(&PatchChannelTaskRequest {
                        status: Some(ChannelTaskStatus::InProgress),
                        assignee_principal_id: None,
                        blocked_reason: NullablePatch::Unchanged,
                        context_label: NullablePatch::Unchanged,
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(patch_response.status(), StatusCode::OK);

    let invalid_patch = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PATCH)
                .uri(format!("/v1/tasks/{}", task.task_id))
                .header(
                    "authorization",
                    format!("Bearer {}", session_token(&assignee)),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_vec(&PatchChannelTaskRequest {
                        status: None,
                        assignee_principal_id: Some(actor.id.clone()),
                        blocked_reason: NullablePatch::Unchanged,
                        context_label: NullablePatch::Unchanged,
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(invalid_patch.status(), StatusCode::OK);

    let after = metrics_text(router).await;
    assert!(prometheus_metric_value(&after, "choruz_channel_task_creates_total") > creates_before);
    assert!(prometheus_metric_value(&after, "choruz_channel_task_updates_total") > updates_before);
    assert!(
        prometheus_metric_value(&after, "choruz_channel_task_mutation_errors_total")
            > errors_before
    );
}

#[tokio::test]
async fn channel_task_read_apis_enforce_membership_and_safe_projection() {
    let _env = ChannelTaskEnvGuard::enabled();
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let operator = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-channel-task-read".into(),
            principal_type: PrincipalType::Human,
            name: "Operator".into(),
            avatar_url: None,
        })
        .unwrap();
    let member = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: operator.workspace_id.clone(),
            principal_type: PrincipalType::Human,
            name: "Member".into(),
            avatar_url: None,
        })
        .unwrap();
    let non_member = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: operator.workspace_id.clone(),
            principal_type: PrincipalType::Human,
            name: "Non Member".into(),
            avatar_url: None,
        })
        .unwrap();
    let visible_agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Visible Agent".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let internal_agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Internal Agent".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: Some(choruz_domain::ChannelVisibility::Internal),
        })
        .unwrap()
        .principal;
    let conversation = app
        .create_group(CreateGroupRequest {
            actor_id: operator.id.clone(),
            name: "Channel Task Reads".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![
                member.id.clone(),
                visible_agent.id.clone(),
                internal_agent.id.clone(),
            ],
            workspace_id: None,
        })
        .unwrap();
    let other_conversation = app
        .create_group(CreateGroupRequest {
            actor_id: operator.id.clone(),
            name: "Other Channel".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![non_member.id.clone()],
            workspace_id: None,
        })
        .unwrap();
    for principal in [
        &operator,
        &member,
        &non_member,
        &visible_agent,
        &internal_agent,
    ] {
        seed_principal_to_db(&database.database_url, principal).await;
    }
    seed_conversation_to_db(&database.database_url, &conversation).await;
    seed_conversation_to_db(&database.database_url, &other_conversation).await;

    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect for channel task read seed");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "
            INSERT INTO conversation_events
              (conversation_id, seq, event_id, event_type, sender_id, content, content_type)
            VALUES
              ($1, 1, 'message-1', 'message', $2, 'Source message', 'text'),
              ($1, 2, 'system-event', 'system', $2, 'System event', 'text'),
              ($3, 1, 'other-message', 'message', $4, 'Other source message', 'text')
            ",
            &[
                &conversation.id,
                &member.id,
                &other_conversation.id,
                &non_member.id,
            ],
        )
        .await
        .expect("seed visible source message");
    client
        .execute(
            "
            INSERT INTO group_workflow_task
              (id, conversation_id, task_key, title, status, assignee_principal_id,
               source_kind, source_message_id, created_by, version)
            VALUES
              ('visible-read-task', $1, 'READ-1', 'Visible task', 'in_progress', $2,
               'message', 'message-1', $3, 2),
              ('internal-read-task', $1, 'READ-2', 'Internal assignee task', 'blocked', $4,
               'agent', NULL, $4, 1),
              ('stale-principal-task', $1, 'READ-3', 'Stale principal task', 'todo', $5,
               'message', 'system-event', $5, 1)
            ",
            &[
                &conversation.id,
                &visible_agent.id,
                &member.id,
                &internal_agent.id,
                &non_member.id,
            ],
        )
        .await
        .expect("seed channel tasks");
    client
        .execute(
            "
            INSERT INTO group_workflow_event
              (id, conversation_id, task_id, actor_principal_id, kind, payload,
               resulting_version, created_at)
            SELECT
              'visible-read-event-' || LPAD(n::TEXT, 2, '0'),
              $1,
              'visible-read-task',
              $2,
              'task.started',
              jsonb_build_object(
                'previous', jsonb_build_object(
                  'status', 'todo',
                  'assignee_principal_id', $2::TEXT,
                  'source_message_id', 'system-event'
                ),
                'new', jsonb_build_object(
                  'status', 'in_progress',
                  'assignee_principal_id', $2::TEXT,
                  'source_message_id', 'message-1'
                ),
                'reason_code', 'private_runtime_label',
                'tool_diagnostics', jsonb_build_object('secret', true)
              ),
              n,
              NOW() + (n || ' seconds')::INTERVAL
            FROM generate_series(1, 55) AS n
            ",
            &[&conversation.id, &non_member.id],
        )
        .await
        .expect("seed channel task event");
    client
        .execute(
            "
            INSERT INTO group_workflow_event
              (id, conversation_id, task_id, actor_principal_id, kind, payload,
               resulting_version, created_at)
            VALUES
              ('mismatched-conversation-event', $1, 'visible-read-task', $2, 'task.started',
               jsonb_build_object(
                 'new', jsonb_build_object(
                   'status', 'done',
                   'source_message_id', 'other-message',
                   'assignee_principal_id', $2::TEXT
                 )
               ),
               999, NOW() + INTERVAL '999 seconds')
            ",
            &[&other_conversation.id, &non_member.id],
        )
        .await
        .expect("seed mismatched event conversation");

    let router = router_with_db(app, &database.database_url);
    let (list_status, list_body) = api_json_request(
        router.clone(),
        &member,
        Method::GET,
        format!("/v1/conversations/{}/tasks", conversation.id),
    )
    .await;
    assert_eq!(list_status, StatusCode::OK);
    let tasks = list_body.as_array().expect("task list body");
    assert_eq!(tasks.len(), 3);
    assert_eq!(tasks[0]["task_key"], "READ-1");
    assert_eq!(tasks[0]["assignee_principal_id"], visible_agent.id);
    assert_eq!(tasks[0]["assignee_name"], "Visible Agent");
    assert_eq!(tasks[0]["source_message_id"], "message-1");
    assert_eq!(tasks[1]["task_key"], "READ-2");
    assert_eq!(
        tasks[1].get("assignee_principal_id"),
        None,
        "internal assignee principal ids are redacted from public projection"
    );
    assert_eq!(tasks[1].get("created_by"), None);
    assert_eq!(tasks[2]["task_key"], "READ-3");
    assert_eq!(
        tasks[2].get("assignee_principal_id"),
        None,
        "non-member assignee principal ids are redacted from public projection"
    );
    assert_eq!(
        tasks[2].get("created_by"),
        None,
        "non-member creators are redacted from public projection"
    );
    assert_eq!(
        tasks[2].get("source_message_id"),
        None,
        "source message ids are redacted unless they resolve in the same conversation"
    );

    let (detail_status, detail_body) = api_json_request(
        router.clone(),
        &member,
        Method::GET,
        "/v1/tasks/visible-read-task".to_string(),
    )
    .await;
    assert_eq!(detail_status, StatusCode::OK);
    assert_eq!(detail_body["task"]["task_key"], "READ-1");
    let events = detail_body["events"].as_array().expect("event list");
    assert_eq!(
        events.len(),
        50,
        "task detail returns bounded recent history"
    );
    assert_eq!(events[0]["resulting_version"], 6);
    assert_eq!(events[49]["resulting_version"], 55);
    assert_eq!(
        events[0].get("actor_principal_id"),
        None,
        "non-member event actor must be redacted"
    );
    assert_eq!(events[0]["new"]["status"], "in_progress");
    assert_eq!(
        events[0]["new"].get("assignee_principal_id"),
        None,
        "non-member assignee ids in event deltas must be redacted"
    );
    assert_eq!(
        events[0]["previous"].get("assignee_principal_id"),
        None,
        "non-member previous assignee ids in event deltas must be redacted"
    );
    assert_eq!(
        events[0]["new"]["source_message_id"], "message-1",
        "same-conversation source message ids in event deltas remain visible"
    );
    assert_eq!(
        events[0]["previous"].get("source_message_id"),
        None,
        "event delta source message ids are redacted when they do not resolve in the same conversation"
    );
    assert_eq!(
        events[0].get("tool_diagnostics"),
        None,
        "raw event metadata must not be projected"
    );
    assert_eq!(
        events[0].get("reason_code"),
        None,
        "raw event reason codes must not be projected"
    );

    let (non_member_status, _) = api_json_request(
        router.clone(),
        &non_member,
        Method::GET,
        format!("/v1/conversations/{}/tasks", conversation.id),
    )
    .await;
    assert_eq!(non_member_status, StatusCode::FORBIDDEN);

    let (non_member_detail_status, _) = api_json_request(
        router,
        &non_member,
        Method::GET,
        "/v1/tasks/visible-read-task".to_string(),
    )
    .await;
    assert_eq!(non_member_detail_status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn channel_task_human_create_from_message_and_patch_are_authorized() {
    let _env = ChannelTaskEnvGuard::enabled();
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let operator = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-channel-task-write".into(),
            principal_type: PrincipalType::Human,
            name: "Operator".into(),
            avatar_url: None,
        })
        .unwrap();
    let human = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: operator.workspace_id.clone(),
            principal_type: PrincipalType::Human,
            name: "Human".into(),
            avatar_url: None,
        })
        .unwrap();
    let peer_human = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: operator.workspace_id.clone(),
            principal_type: PrincipalType::Human,
            name: "Peer Human".into(),
            avatar_url: None,
        })
        .unwrap();
    let non_member = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: operator.workspace_id.clone(),
            principal_type: PrincipalType::Human,
            name: "Non Member".into(),
            avatar_url: None,
        })
        .unwrap();
    let visible_agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Visible Agent".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let internal_agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Internal Agent".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: Some(choruz_domain::ChannelVisibility::Internal),
        })
        .unwrap()
        .principal;
    let conversation = app
        .create_group(CreateGroupRequest {
            actor_id: operator.id.clone(),
            name: "Channel Task Writes".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![
                human.id.clone(),
                peer_human.id.clone(),
                visible_agent.id.clone(),
                internal_agent.id.clone(),
            ],
            workspace_id: None,
        })
        .unwrap();
    let human_direct = app
        .create_direct_conversation(CreateDirectConversationRequest {
            actor_id: human.id.clone(),
            peer_principal_id: peer_human.id.clone(),
            workspace_id: None,
        })
        .unwrap();
    let agent_direct = app
        .create_direct_conversation(CreateDirectConversationRequest {
            actor_id: human.id.clone(),
            peer_principal_id: visible_agent.id.clone(),
            workspace_id: None,
        })
        .unwrap();
    let internal_agent_direct = app
        .create_direct_conversation(CreateDirectConversationRequest {
            actor_id: human.id.clone(),
            peer_principal_id: internal_agent.id.clone(),
            workspace_id: None,
        })
        .unwrap();
    for principal in [
        &operator,
        &human,
        &peer_human,
        &non_member,
        &visible_agent,
        &internal_agent,
    ] {
        seed_principal_to_db(&database.database_url, principal).await;
    }
    seed_conversation_to_db(&database.database_url, &conversation).await;
    seed_conversation_to_db(&database.database_url, &human_direct).await;
    seed_conversation_to_db(&database.database_url, &agent_direct).await;
    seed_conversation_to_db(&database.database_url, &internal_agent_direct).await;

    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect for channel task write seed");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "
            INSERT INTO conversation_events
              (conversation_id, seq, event_id, event_type, sender_id, content, content_type)
            VALUES
              ($1, 1, 'write-message-1', 'message', $2, 'Please track this', 'text'),
              ($1, 2, 'write-message-2', 'message', $2, 'Please also track this', 'text'),
              ($3, 1, 'human-direct-message-1', 'message', $2, 'No board here', 'text'),
              ($4, 1, 'agent-direct-message-1', 'message', $2, 'Please track this DM follow-up', 'text'),
              ($4, 2, 'agent-direct-message-2', 'message', $5, 'Agent-visible DM source', 'text'),
              ($6, 1, 'internal-agent-direct-message-1', 'message', $2, 'Internal agent DM should not have a board', 'text')
            ",
            &[
                &conversation.id,
                &human.id,
                &human_direct.id,
                &agent_direct.id,
                &visible_agent.id,
                &internal_agent_direct.id,
            ],
        )
        .await
        .expect("seed write source messages");

    let router = router_with_db(app, &database.database_url);
    let (create_status, created) = api_json_payload_request(
        router.clone(),
        &human,
        Method::POST,
        format!("/v1/conversations/{}/tasks/from-message", conversation.id),
        json!({
            "message_id": "write-message-1",
            "title": "Track launch follow-up",
            "assignee_principal_id": visible_agent.id,
            "context_label": "Launch",
            "idempotency_key": "human-click-1"
        }),
    )
    .await;
    assert_eq!(create_status, StatusCode::CREATED);
    assert_eq!(created["task_key"], "TASK-1");
    assert_eq!(created["status"], "todo");
    assert_eq!(created["source_kind"], "message");
    assert_eq!(created["source_message_id"], "write-message-1");
    assert_eq!(created["context_label"], "Launch");
    let task_id = created["task_id"].as_str().unwrap().to_string();

    let (dedupe_status, deduped) = api_json_payload_request(
        router.clone(),
        &human,
        Method::POST,
        format!("/v1/conversations/{}/tasks/from-message", conversation.id),
        json!({
            "message_id": "write-message-1",
            "title": "Changed title should not mutate",
            "assignee_principal_id": peer_human.id,
            "context_label": "Changed",
            "idempotency_key": "human-click-1"
        }),
    )
    .await;
    assert_eq!(dedupe_status, StatusCode::OK);
    assert_eq!(deduped["task_id"], task_id);
    assert_eq!(deduped["title"], "Track launch follow-up");
    assert_eq!(deduped["assignee_principal_id"], visible_agent.id);
    assert_eq!(deduped["context_label"], "Launch");
    assert_eq!(deduped["version"], 1);

    let (changed_key_dedupe_status, changed_key_deduped) = api_json_payload_request(
        router.clone(),
        &human,
        Method::POST,
        format!("/v1/conversations/{}/tasks/from-message", conversation.id),
        json!({
            "message_id": "write-message-1",
            "title": "Changed key should still not mutate",
            "assignee_principal_id": peer_human.id,
            "context_label": "Changed again",
            "idempotency_key": "human-click-different-key"
        }),
    )
    .await;
    assert_eq!(changed_key_dedupe_status, StatusCode::OK);
    assert_eq!(changed_key_deduped["task_id"], task_id);
    assert_eq!(changed_key_deduped["title"], "Track launch follow-up");
    assert_eq!(
        changed_key_deduped["assignee_principal_id"],
        visible_agent.id
    );
    assert_eq!(changed_key_deduped["context_label"], "Launch");
    assert_eq!(changed_key_deduped["version"], 1);

    let (different_source_same_key_status, _) = api_json_payload_request(
        router.clone(),
        &human,
        Method::POST,
        format!("/v1/conversations/{}/tasks/from-message", conversation.id),
        json!({
            "message_id": "write-message-2",
            "title": "Same idempotency key for another message conflicts",
            "assignee_principal_id": visible_agent.id,
            "context_label": "Other source",
            "idempotency_key": "human-click-1"
        }),
    )
    .await;
    assert_eq!(different_source_same_key_status, StatusCode::CONFLICT);

    let message_task_row_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM group_workflow_task
             WHERE conversation_id = $1
               AND source_kind = 'message'
               AND source_message_id = 'write-message-1'
               AND created_by = $2",
            &[&conversation.id, &human.id],
        )
        .await
        .expect("count message-derived task rows")
        .get::<_, i64>("count");
    assert_eq!(message_task_row_count, 1);
    let other_message_task_row_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM group_workflow_task
             WHERE conversation_id = $1
               AND source_kind = 'message'
               AND source_message_id = 'write-message-2'
               AND created_by = $2",
            &[&conversation.id, &human.id],
        )
        .await
        .expect("count rejected message-derived task rows")
        .get::<_, i64>("count");
    assert_eq!(other_message_task_row_count, 0);
    let message_owner_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM group_workflow_task_participant
             WHERE task_id = $1 AND principal_id = $2 AND role_key = 'owner'",
            &[&task_id, &visible_agent.id],
        )
        .await
        .expect("count message owner participant")
        .get::<_, i64>("count");
    assert_eq!(message_owner_count, 1);
    let changed_assignee_owner_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM group_workflow_task_participant
             WHERE task_id = $1 AND principal_id = $2 AND role_key = 'owner'",
            &[&task_id, &peer_human.id],
        )
        .await
        .expect("count non-mutated replay owner participant")
        .get::<_, i64>("count");
    assert_eq!(changed_assignee_owner_count, 0);
    let message_workflow_create_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM group_workflow_event
             WHERE task_id = $1 AND kind = 'channel_task.created'",
            &[&task_id],
        )
        .await
        .expect("count message workflow create events")
        .get::<_, i64>("count");
    assert_eq!(message_workflow_create_count, 1);
    let message_fanout_create_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM conversation_events
             WHERE conversation_id = $1
               AND event_type = 'channel_task.created'
               AND metadata->>'task_id' = $2",
            &[&conversation.id, &task_id],
        )
        .await
        .expect("count message fanout create events")
        .get::<_, i64>("count");
    assert_eq!(message_fanout_create_count, 1);

    let (patch_status, patched) = api_json_payload_request(
        router.clone(),
        &human,
        Method::PATCH,
        format!("/v1/tasks/{task_id}"),
        json!({
            "status": "blocked",
            "assignee_principal_id": peer_human.id,
            "blocked_reason": "Waiting on customer",
            "context_label": null
        }),
    )
    .await;
    assert_eq!(patch_status, StatusCode::OK);
    assert_eq!(patched["status"], "blocked");
    assert_eq!(patched["assignee_principal_id"], peer_human.id);
    assert_eq!(patched["blocked_reason"], "Waiting on customer");
    assert_eq!(patched.get("context_label"), None);
    assert_eq!(patched["version"], 2);

    let fanout_rows = client
        .query(
            "
            SELECT event_type, sender_id, content, content_type, metadata
            FROM conversation_events
            WHERE conversation_id = $1
              AND event_type IN ('channel_task.created', 'channel_task.updated')
            ORDER BY seq ASC
            ",
            &[&conversation.id],
        )
        .await
        .expect("load channel task fanout events");
    assert_eq!(fanout_rows.len(), 2);
    let created_fanout_type: String = fanout_rows[0].get("event_type");
    let updated_fanout_type: String = fanout_rows[1].get("event_type");
    assert_eq!(created_fanout_type, "channel_task.created");
    assert_eq!(updated_fanout_type, "channel_task.updated");
    assert_eq!(fanout_rows[0].get::<_, String>("sender_id"), human.id);
    assert_eq!(fanout_rows[0].get::<_, Option<String>>("content"), None);
    assert_eq!(
        fanout_rows[0].get::<_, String>("content_type"),
        "application/vnd.choruz.channel-task+json"
    );
    let created_metadata: Value = fanout_rows[0].get("metadata");
    assert_eq!(created_metadata["conversation_id"], conversation.id);
    assert_eq!(created_metadata["task_id"], task_id);
    assert_eq!(created_metadata["version"], 1);
    assert_eq!(created_metadata["task"]["task_id"], task_id);
    assert_eq!(
        created_metadata["task"]["source_message_id"],
        "write-message-1"
    );
    assert_eq!(created_metadata.get("workflow_diagnostic"), None);
    assert_eq!(created_metadata.get("tool_diagnostics"), None);
    assert_eq!(created_metadata["task"].get("workflow_diagnostic"), None);
    assert_eq!(created_metadata["task"].get("tool_diagnostics"), None);
    let updated_metadata: Value = fanout_rows[1].get("metadata");
    assert_eq!(updated_metadata["task_id"], task_id);
    assert_eq!(updated_metadata["version"], 2);
    assert_eq!(updated_metadata["task"]["status"], "blocked");
    assert_eq!(
        updated_metadata["task"]["updated_at"],
        updated_metadata["updated_at"]
    );

    let task_fanout_router_outbox_count = client
        .query_one(
            "
            SELECT COUNT(*)::BIGINT AS count
            FROM event_outbox
            WHERE aggregate_id = $1
              AND event_type IN ('channel_task.created', 'channel_task.updated')
            ",
            &[&conversation.id],
        )
        .await
        .expect("count channel task router outbox rows")
        .get::<_, i64>("count");
    assert_eq!(
        task_fanout_router_outbox_count, 0,
        "task fanout events must not enter the router/agent event_outbox"
    );

    let timeline_message_count = client
        .query_one(
            "
            SELECT COUNT(*)::BIGINT AS count
            FROM conversation_events
            WHERE conversation_id = $1
              AND event_type IN ('message', 'message.created', 'reply')
            ",
            &[&conversation.id],
        )
        .await
        .expect("count visible timeline messages")
        .get::<_, i64>("count");
    assert_eq!(
        timeline_message_count, 2,
        "task fanout events must not create normal timeline chat messages"
    );

    let (direct_create_status, direct_created) = api_json_payload_request(
        router.clone(),
        &human,
        Method::POST,
        format!("/v1/conversations/{}/tasks/from-message", agent_direct.id),
        json!({
            "message_id": "agent-direct-message-2",
            "title": "Track direct follow-up",
            "assignee_principal_id": visible_agent.id,
            "context_label": "Direct agent",
            "idempotency_key": "human-agent-dm-click-1"
        }),
    )
    .await;
    assert_eq!(direct_create_status, StatusCode::CREATED);
    assert_eq!(direct_created["conversation_id"], agent_direct.id);
    assert_eq!(direct_created["task_key"], "TASK-1");
    assert_eq!(direct_created["title"], "Track direct follow-up");
    assert_eq!(direct_created["status"], "todo");
    assert_eq!(direct_created["source_kind"], "message");
    assert_eq!(
        direct_created["source_message_id"],
        "agent-direct-message-2"
    );
    assert_eq!(direct_created["context_label"], "Direct agent");
    assert_eq!(direct_created["assignee_principal_id"], visible_agent.id);
    assert_eq!(direct_created["assignee_name"], "Visible Agent");
    assert_eq!(direct_created["created_by"], human.id);
    assert_eq!(direct_created["version"], 1);
    assert_eq!(direct_created.get("workflow_diagnostic"), None);
    assert_eq!(direct_created.get("tool_diagnostics"), None);
    assert_eq!(direct_created.get("description"), None);
    let direct_task_id = direct_created["task_id"].as_str().unwrap().to_string();

    let (direct_list_status, direct_list_body) = api_json_request(
        router.clone(),
        &human,
        Method::GET,
        format!("/v1/conversations/{}/tasks", agent_direct.id),
    )
    .await;
    assert_eq!(direct_list_status, StatusCode::OK);
    let direct_tasks = direct_list_body
        .as_array()
        .expect("direct-agent task list body");
    assert_eq!(direct_tasks.len(), 1);
    assert_eq!(direct_tasks[0]["task_id"], direct_task_id);
    assert_eq!(
        direct_tasks[0]["source_message_id"],
        "agent-direct-message-2"
    );
    assert_eq!(direct_tasks[0]["assignee_principal_id"], visible_agent.id);

    let (direct_detail_status, direct_detail_body) = api_json_request(
        router.clone(),
        &human,
        Method::GET,
        format!("/v1/tasks/{direct_task_id}"),
    )
    .await;
    assert_eq!(direct_detail_status, StatusCode::OK);
    assert_eq!(direct_detail_body["task"]["task_id"], direct_task_id);
    assert_eq!(
        direct_detail_body["task"]["source_message_id"],
        "agent-direct-message-2"
    );
    assert_eq!(
        direct_detail_body["task"]["assignee_principal_id"],
        visible_agent.id
    );
    assert_eq!(direct_detail_body["events"].as_array().unwrap().len(), 1);
    assert_eq!(
        direct_detail_body["events"][0]["kind"],
        "channel_task.workflow_event"
    );
    assert_eq!(
        direct_detail_body["events"][0]["workflow_kind"],
        "channel_task.created"
    );

    let direct_owner_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM group_workflow_task_participant
             WHERE task_id = $1 AND principal_id = $2 AND role_key = 'owner'",
            &[&direct_task_id, &visible_agent.id],
        )
        .await
        .expect("count direct-agent owner participant")
        .get::<_, i64>("count");
    assert_eq!(direct_owner_count, 1);
    let direct_workflow_create_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM group_workflow_event
             WHERE task_id = $1 AND kind = 'channel_task.created'",
            &[&direct_task_id],
        )
        .await
        .expect("count direct-agent workflow create events")
        .get::<_, i64>("count");
    assert_eq!(direct_workflow_create_count, 1);
    let direct_fanout_rows = client
        .query(
            "
            SELECT event_type, sender_id, content, content_type, metadata
            FROM conversation_events
            WHERE conversation_id = $1
              AND event_type = 'channel_task.created'
            ORDER BY seq ASC
            ",
            &[&agent_direct.id],
        )
        .await
        .expect("load direct-agent task fanout");
    assert_eq!(direct_fanout_rows.len(), 1);
    assert_eq!(
        direct_fanout_rows[0].get::<_, String>("sender_id"),
        human.id
    );
    assert_eq!(
        direct_fanout_rows[0].get::<_, Option<String>>("content"),
        None
    );
    assert_eq!(
        direct_fanout_rows[0].get::<_, String>("content_type"),
        "application/vnd.choruz.channel-task+json"
    );
    let direct_fanout_metadata: Value = direct_fanout_rows[0].get("metadata");
    assert_eq!(direct_fanout_metadata["conversation_id"], agent_direct.id);
    assert_eq!(direct_fanout_metadata["task_id"], direct_task_id);
    assert_eq!(direct_fanout_metadata["version"], 1);
    assert_eq!(
        direct_fanout_metadata["task"]["source_message_id"],
        "agent-direct-message-2"
    );
    assert_eq!(
        direct_fanout_metadata["task"]["assignee_principal_id"],
        visible_agent.id
    );
    assert_eq!(
        direct_fanout_metadata["task"].get("workflow_diagnostic"),
        None
    );
    assert_eq!(direct_fanout_metadata["task"].get("tool_diagnostics"), None);

    let direct_timeline_message_count = client
        .query_one(
            "
            SELECT COUNT(*)::BIGINT AS count
            FROM conversation_events
            WHERE conversation_id = $1
              AND event_type IN ('message', 'message.created', 'reply')
            ",
            &[&agent_direct.id],
        )
        .await
        .expect("count direct-agent visible timeline messages")
        .get::<_, i64>("count");
    assert_eq!(
        direct_timeline_message_count, 2,
        "direct-agent task creation must not create normal timeline chat messages"
    );

    let (direct_agent_command_status, _) = api_json_payload_request(
        router.clone(),
        &visible_agent,
        Method::POST,
        format!("/v1/conversations/{}/tasks", agent_direct.id),
        json!({
            "task_key": "DM-2",
            "title": "Direct agent commands remain unsupported",
            "idempotency_key": "direct-agent-command-still-forbidden"
        }),
    )
    .await;
    assert_eq!(direct_agent_command_status, StatusCode::FORBIDDEN);
    let direct_agent_command_task_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM group_workflow_task
             WHERE conversation_id = $1 AND source_kind = 'agent'",
            &[&agent_direct.id],
        )
        .await
        .expect("count rejected direct-agent command tasks")
        .get::<_, i64>("count");
    assert_eq!(direct_agent_command_task_count, 0);

    let (direct_agent_from_message_status, _) = api_json_payload_request(
        router.clone(),
        &visible_agent,
        Method::POST,
        format!("/v1/conversations/{}/tasks/from-message", agent_direct.id),
        json!({
            "message_id": "agent-direct-message-1",
            "title": "Direct agents cannot create from message",
            "assignee_principal_id": visible_agent.id,
            "idempotency_key": "direct-agent-from-message-forbidden"
        }),
    )
    .await;
    assert_eq!(direct_agent_from_message_status, StatusCode::FORBIDDEN);
    let direct_task_count_after_agent_rejection = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM group_workflow_task
             WHERE conversation_id = $1",
            &[&agent_direct.id],
        )
        .await
        .expect("count direct-agent tasks after agent from-message rejection")
        .get::<_, i64>("count");
    assert_eq!(direct_task_count_after_agent_rejection, 1);
    let direct_owner_count_after_agent_rejection = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM group_workflow_task_participant participant
             JOIN group_workflow_task task ON task.id = participant.task_id
             WHERE task.conversation_id = $1",
            &[&agent_direct.id],
        )
        .await
        .expect("count direct-agent owner rows after agent from-message rejection")
        .get::<_, i64>("count");
    assert_eq!(direct_owner_count_after_agent_rejection, 1);
    let direct_workflow_event_count_after_agent_rejection = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM group_workflow_event
             WHERE conversation_id = $1
               AND kind LIKE 'channel_task.%'",
            &[&agent_direct.id],
        )
        .await
        .expect("count direct-agent workflow events after agent from-message rejection")
        .get::<_, i64>("count");
    assert_eq!(direct_workflow_event_count_after_agent_rejection, 1);
    let direct_fanout_count_after_agent_rejection = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM conversation_events
             WHERE conversation_id = $1
               AND event_type IN ('channel_task.created', 'channel_task.updated')",
            &[&agent_direct.id],
        )
        .await
        .expect("count direct-agent fanout after agent from-message rejection")
        .get::<_, i64>("count");
    assert_eq!(direct_fanout_count_after_agent_rejection, 1);
    let direct_timeline_count_after_agent_rejection = client
        .query_one(
            "
            SELECT COUNT(*)::BIGINT AS count
            FROM conversation_events
            WHERE conversation_id = $1
              AND event_type IN ('message', 'message.created', 'reply')
            ",
            &[&agent_direct.id],
        )
        .await
        .expect("count direct-agent chat messages after agent from-message rejection")
        .get::<_, i64>("count");
    assert_eq!(direct_timeline_count_after_agent_rejection, 2);

    let (internal_direct_status, _) = api_json_payload_request(
        router.clone(),
        &human,
        Method::POST,
        format!(
            "/v1/conversations/{}/tasks/from-message",
            internal_agent_direct.id
        ),
        json!({
            "message_id": "internal-agent-direct-message-1",
            "title": "Internal direct agent should be ineligible",
            "assignee_principal_id": human.id,
            "idempotency_key": "internal-agent-direct-forbidden"
        }),
    )
    .await;
    assert_eq!(internal_direct_status, StatusCode::FORBIDDEN);
    let internal_direct_task_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM group_workflow_task
             WHERE conversation_id = $1",
            &[&internal_agent_direct.id],
        )
        .await
        .expect("count rejected internal-agent-direct task rows")
        .get::<_, i64>("count");
    assert_eq!(internal_direct_task_count, 0);
    let internal_direct_owner_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM group_workflow_task_participant participant
             JOIN group_workflow_task task ON task.id = participant.task_id
             WHERE task.conversation_id = $1",
            &[&internal_agent_direct.id],
        )
        .await
        .expect("count rejected internal-agent-direct owner rows")
        .get::<_, i64>("count");
    assert_eq!(internal_direct_owner_count, 0);
    let internal_direct_workflow_event_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM group_workflow_event
             WHERE conversation_id = $1
               AND kind LIKE 'channel_task.%'",
            &[&internal_agent_direct.id],
        )
        .await
        .expect("count rejected internal-agent-direct workflow events")
        .get::<_, i64>("count");
    assert_eq!(internal_direct_workflow_event_count, 0);
    let internal_direct_fanout_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM conversation_events
             WHERE conversation_id = $1
               AND event_type IN ('channel_task.created', 'channel_task.updated')",
            &[&internal_agent_direct.id],
        )
        .await
        .expect("count rejected internal-agent-direct fanout events")
        .get::<_, i64>("count");
    assert_eq!(internal_direct_fanout_count, 0);
    let internal_direct_timeline_message_count = client
        .query_one(
            "
            SELECT COUNT(*)::BIGINT AS count
            FROM conversation_events
            WHERE conversation_id = $1
              AND event_type IN ('message', 'message.created', 'reply')
            ",
            &[&internal_agent_direct.id],
        )
        .await
        .expect("count rejected internal-agent-direct visible timeline messages")
        .get::<_, i64>("count");
    assert_eq!(
        internal_direct_timeline_message_count, 1,
        "internal-agent direct rejection must not create normal timeline chat messages"
    );

    let (internal_assignee_status, _) = api_json_payload_request(
        router.clone(),
        &human,
        Method::PATCH,
        format!("/v1/tasks/{task_id}"),
        json!({
            "assignee_principal_id": internal_agent.id
        }),
    )
    .await;
    assert_eq!(internal_assignee_status, StatusCode::BAD_REQUEST);

    let (non_member_status, _) = api_json_payload_request(
        router.clone(),
        &non_member,
        Method::PATCH,
        format!("/v1/tasks/{task_id}"),
        json!({
            "status": "done"
        }),
    )
    .await;
    assert_eq!(non_member_status, StatusCode::FORBIDDEN);

    let (missing_patch_status, _) = api_json_payload_request(
        router.clone(),
        &human,
        Method::PATCH,
        "/v1/tasks/missing-channel-task".to_string(),
        json!({
            "status": "done"
        }),
    )
    .await;
    assert_eq!(missing_patch_status, StatusCode::NOT_FOUND);

    let (human_direct_status, _) = api_json_payload_request(
        router,
        &human,
        Method::POST,
        format!("/v1/conversations/{}/tasks/from-message", human_direct.id),
        json!({
            "message_id": "human-direct-message-1",
            "title": "Should be rejected",
            "assignee_principal_id": human.id
        }),
    )
    .await;
    assert_eq!(human_direct_status, StatusCode::FORBIDDEN);
    let human_direct_task_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM group_workflow_task
             WHERE conversation_id = $1",
            &[&human_direct.id],
        )
        .await
        .expect("count rejected human-direct task rows")
        .get::<_, i64>("count");
    assert_eq!(human_direct_task_count, 0);
    let human_direct_owner_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM group_workflow_task_participant participant
             JOIN group_workflow_task task ON task.id = participant.task_id
             WHERE task.conversation_id = $1",
            &[&human_direct.id],
        )
        .await
        .expect("count rejected human-direct owner rows")
        .get::<_, i64>("count");
    assert_eq!(human_direct_owner_count, 0);
    let human_direct_workflow_event_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM group_workflow_event
             WHERE conversation_id = $1
               AND kind LIKE 'channel_task.%'",
            &[&human_direct.id],
        )
        .await
        .expect("count rejected human-direct workflow events")
        .get::<_, i64>("count");
    assert_eq!(human_direct_workflow_event_count, 0);
    let human_direct_fanout_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM conversation_events
             WHERE conversation_id = $1
               AND event_type IN ('channel_task.created', 'channel_task.updated')",
            &[&human_direct.id],
        )
        .await
        .expect("count rejected human-direct fanout events")
        .get::<_, i64>("count");
    assert_eq!(human_direct_fanout_count, 0);
    let human_direct_timeline_message_count = client
        .query_one(
            "
            SELECT COUNT(*)::BIGINT AS count
            FROM conversation_events
            WHERE conversation_id = $1
              AND event_type IN ('message', 'message.created', 'reply')
            ",
            &[&human_direct.id],
        )
        .await
        .expect("count rejected human-direct visible timeline messages")
        .get::<_, i64>("count");
    assert_eq!(
        human_direct_timeline_message_count, 1,
        "human-to-human direct rejection must not create normal timeline chat messages"
    );
}

#[tokio::test]
async fn channel_task_fanout_payload_shape_redacts_internal_metadata_and_orders_versions() {
    let _env = ChannelTaskEnvGuard::enabled();
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let operator = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-channel-task-fanout-shape".into(),
            principal_type: PrincipalType::Human,
            name: "Operator".into(),
            avatar_url: None,
        })
        .unwrap();
    let human = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: operator.workspace_id.clone(),
            principal_type: PrincipalType::Human,
            name: "Human".into(),
            avatar_url: None,
        })
        .unwrap();
    let peer_human = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: operator.workspace_id.clone(),
            principal_type: PrincipalType::Human,
            name: "Peer Human".into(),
            avatar_url: None,
        })
        .unwrap();
    let non_member = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: operator.workspace_id.clone(),
            principal_type: PrincipalType::Human,
            name: "Non Member".into(),
            avatar_url: None,
        })
        .unwrap();
    let visible_agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Visible Agent".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let internal_agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Internal Agent".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: Some(choruz_domain::ChannelVisibility::Internal),
        })
        .unwrap()
        .principal;
    let conversation = app
        .create_group(CreateGroupRequest {
            actor_id: operator.id.clone(),
            name: "Fanout Shape".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![
                human.id.clone(),
                peer_human.id.clone(),
                visible_agent.id.clone(),
                internal_agent.id.clone(),
            ],
            workspace_id: None,
        })
        .unwrap();
    for principal in [
        &operator,
        &human,
        &peer_human,
        &non_member,
        &visible_agent,
        &internal_agent,
    ] {
        seed_principal_to_db(&database.database_url, principal).await;
    }
    seed_conversation_to_db(&database.database_url, &conversation).await;

    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect for fanout shape seed");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "
            INSERT INTO conversation_events
              (conversation_id, seq, event_id, event_type, sender_id, content, content_type)
            VALUES
              ($1, 1, 'fanout-shape-source-1', 'message', $2, 'Track this', 'text')
            ",
            &[&conversation.id, &human.id],
        )
        .await
        .expect("seed source message");

    let router = router_with_db(app, &database.database_url);

    // v1: human creates a task from a message (assignee = visible_agent).
    let (create_status, created) = api_json_payload_request(
        router.clone(),
        &human,
        Method::POST,
        format!("/v1/conversations/{}/tasks/from-message", conversation.id),
        json!({
            "message_id": "fanout-shape-source-1",
            "title": "Track follow-up",
            "assignee_principal_id": visible_agent.id,
            "context_label": "Launch",
            "idempotency_key": "fanout-shape-create-1"
        }),
    )
    .await;
    assert_eq!(create_status, StatusCode::CREATED);
    let task_id = created["task_id"].as_str().unwrap().to_string();

    // v2: human reassigns to peer_human and moves to in_progress.
    let (patch_one_status, patch_one) = api_json_payload_request(
        router.clone(),
        &human,
        Method::PATCH,
        format!("/v1/tasks/{task_id}"),
        json!({
            "assignee_principal_id": peer_human.id,
            "status": "in_progress"
        }),
    )
    .await;
    assert_eq!(patch_one_status, StatusCode::OK);
    assert_eq!(patch_one["version"], 2);

    // v3: human marks the task blocked.
    let (patch_two_status, patch_two) = api_json_payload_request(
        router.clone(),
        &human,
        Method::PATCH,
        format!("/v1/tasks/{task_id}"),
        json!({
            "status": "blocked",
            "blocked_reason": "Waiting on customer"
        }),
    )
    .await;
    assert_eq!(patch_two_status, StatusCode::OK);
    assert_eq!(patch_two["version"], 3);

    // Direct-insert a second task whose assignee is an internal agent and whose
    // recorded creator is a non-member. The API rejects assigning either of
    // these via the public mutation surface, so we seed it via SQL to prove
    // that the fanout safe projection redacts them on the next mutation event.
    let polluted_task_id = "polluted-fanout-task";
    client
        .execute(
            "
            INSERT INTO group_workflow_task
              (id, conversation_id, task_key, title, status,
               assignee_principal_id, source_kind, source_message_id,
               created_by, version)
            VALUES
              ($1, $2, 'TASK-POLLUTED', 'Polluted seed', 'todo',
               $3, 'agent', NULL,
               $4, 7)
            ",
            &[
                &polluted_task_id,
                &conversation.id,
                &internal_agent.id,
                &non_member.id,
            ],
        )
        .await
        .expect("seed polluted task row");

    // Trigger a fanout snapshot on the polluted task by patching its status
    // only (assignee unchanged, so we do not re-validate the seeded assignee).
    let (polluted_patch_status, polluted_patch) = api_json_payload_request(
        router,
        &human,
        Method::PATCH,
        format!("/v1/tasks/{polluted_task_id}"),
        json!({
            "status": "in_progress"
        }),
    )
    .await;
    assert_eq!(polluted_patch_status, StatusCode::OK);
    assert_eq!(polluted_patch["version"], 8);
    // The HTTP response itself must already be the safe projection.
    assert_eq!(polluted_patch.get("assignee_principal_id"), None);
    assert_eq!(polluted_patch.get("assignee_name"), None);
    assert_eq!(polluted_patch.get("assignee_type"), None);
    assert_eq!(polluted_patch.get("created_by"), None);

    let fanout_rows = client
        .query(
            "
            SELECT seq, event_type, sender_id, content, content_type, metadata
            FROM conversation_events
            WHERE conversation_id = $1
              AND event_type IN ('channel_task.created', 'channel_task.updated')
            ORDER BY seq ASC
            ",
            &[&conversation.id],
        )
        .await
        .expect("load channel task fanout events");
    assert_eq!(
        fanout_rows.len(),
        4,
        "expected one created + three updated fanout rows"
    );

    let allowed_top_level: std::collections::HashSet<&str> = [
        "event_type",
        "conversation_id",
        "task_id",
        "version",
        "updated_at",
        "task",
    ]
    .into_iter()
    .collect();
    let allowed_task_keys: std::collections::HashSet<&str> = [
        "task_id",
        "conversation_id",
        "task_key",
        "title",
        "status",
        "assignee_principal_id",
        "assignee_type",
        "assignee_name",
        "blocked_reason",
        "context_label",
        "source_kind",
        "source_message_id",
        "created_by",
        "created_by_type",
        "updated_by",
        "updated_by_type",
        "version",
        "created_at",
        "updated_at",
    ]
    .into_iter()
    .collect();

    let mut last_seq: i64 = 0;
    let mut versions_for_task: std::collections::HashMap<String, i64> =
        std::collections::HashMap::new();
    let mut saw_task_one_created = false;
    let mut saw_polluted_redaction = false;

    for row in &fanout_rows {
        let seq: i64 = row.get("seq");
        assert!(
            seq > last_seq,
            "fanout seq must be strictly increasing across channel task events: got {seq} after {last_seq}"
        );
        last_seq = seq;

        let event_type: String = row.get("event_type");
        assert!(
            event_type == "channel_task.created" || event_type == "channel_task.updated",
            "unexpected channel task event_type: {event_type}"
        );
        assert_eq!(
            row.get::<_, Option<String>>("content"),
            None,
            "channel task fanout rows must not carry text content"
        );
        assert_eq!(
            row.get::<_, String>("content_type"),
            "application/vnd.choruz.channel-task+json"
        );

        let metadata: Value = row.get("metadata");
        let metadata_obj = metadata
            .as_object()
            .expect("channel task fanout metadata is an object");
        for key in metadata_obj.keys() {
            assert!(
                allowed_top_level.contains(key.as_str()),
                "unexpected top-level fanout metadata key: {key}"
            );
        }
        assert_eq!(metadata["event_type"], event_type);
        assert_eq!(metadata["conversation_id"], conversation.id);

        let row_task_id = metadata["task_id"]
            .as_str()
            .expect("metadata.task_id is a string")
            .to_string();
        let envelope_version = metadata["version"]
            .as_i64()
            .expect("metadata.version is i64");
        let prev = versions_for_task.insert(row_task_id.clone(), envelope_version);
        if let Some(prev_version) = prev {
            assert!(
                envelope_version > prev_version,
                "task {row_task_id} fanout version must be strictly increasing: {envelope_version} after {prev_version}"
            );
        }

        let envelope_updated_at = metadata["updated_at"]
            .as_str()
            .expect("metadata.updated_at is a string")
            .to_string();
        let task = metadata["task"]
            .as_object()
            .expect("metadata.task is an object");
        for key in task.keys() {
            assert!(
                allowed_task_keys.contains(key.as_str()),
                "unexpected key inside fanout snapshot task: {key}"
            );
        }
        assert_eq!(task["task_id"].as_str(), Some(row_task_id.as_str()));
        assert_eq!(task["conversation_id"], conversation.id);
        assert_eq!(task["version"].as_i64(), Some(envelope_version));
        assert_eq!(
            task["updated_at"].as_str(),
            Some(envelope_updated_at.as_str()),
            "snapshot.updated_at must equal envelope updated_at"
        );

        if row_task_id == task_id && event_type == "channel_task.created" {
            saw_task_one_created = true;
            assert_eq!(task["status"], "todo");
            assert_eq!(task["assignee_principal_id"], visible_agent.id);
            assert_eq!(task["source_message_id"], "fanout-shape-source-1");
        }

        if row_task_id == polluted_task_id {
            saw_polluted_redaction = true;
            assert_eq!(task["status"], "in_progress");
            assert_eq!(
                task.get("assignee_principal_id"),
                None,
                "internal-agent assignee must be redacted from fanout snapshot"
            );
            assert_eq!(
                task.get("assignee_name"),
                None,
                "internal-agent assignee_name must be redacted from fanout snapshot"
            );
            assert_eq!(
                task.get("assignee_type"),
                None,
                "internal-agent assignee_type must be redacted from fanout snapshot"
            );
            assert_eq!(
                task.get("created_by"),
                None,
                "non-member created_by must be redacted from fanout snapshot"
            );
            assert_eq!(
                task.get("created_by_type"),
                None,
                "non-member created_by_type must be redacted from fanout snapshot"
            );
        }

        // Forbidden token sweep: ensure no raw workflow metadata, diagnostics,
        // role maps, lineage, idempotency, internal IDs, or non-member IDs
        // ever surface inside the serialized fanout payload.
        let serialized = serde_json::to_string(&metadata).expect("serialize fanout metadata");
        for forbidden in [
            "\"previous\"",
            "\"new\"",
            "\"kind\"",
            "\"payload\"",
            "\"workflow_diagnostic\"",
            "\"tool_diagnostics\"",
            "\"routing_role_map\"",
            "\"subagent_lineage\"",
            "\"idempotency_payload_hash\"",
            "\"reason_code\"",
            "\"status_effect\"",
            "\"workflow_kind\"",
            "\"actor_principal_id\"",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "fanout metadata must not contain {forbidden}: {serialized}"
            );
        }
        assert!(
            !serialized.contains(internal_agent.id.as_str()),
            "fanout metadata must not contain internal agent principal id: {serialized}"
        );
        assert!(
            !serialized.contains(non_member.id.as_str()),
            "fanout metadata must not contain non-member principal id: {serialized}"
        );
    }

    assert!(
        saw_task_one_created,
        "missing channel_task.created envelope for primary task"
    );
    assert!(
        saw_polluted_redaction,
        "missing channel_task.updated envelope that exercises redaction"
    );

    let primary_version = versions_for_task
        .get(&task_id)
        .copied()
        .expect("primary task fanout versions present");
    assert_eq!(
        primary_version, 3,
        "primary task last fanout version must match latest applied mutation"
    );
    let polluted_version = versions_for_task
        .get(polluted_task_id)
        .copied()
        .expect("polluted task fanout version present");
    assert_eq!(
        polluted_version, 8,
        "polluted task fanout version must reflect direct-seed base version + 1"
    );

    let task_router_outbox_count = client
        .query_one(
            "
            SELECT COUNT(*)::BIGINT AS count
            FROM event_outbox
            WHERE aggregate_id = $1
              AND event_type IN ('channel_task.created', 'channel_task.updated')
            ",
            &[&conversation.id],
        )
        .await
        .expect("count channel task router outbox rows")
        .get::<_, i64>("count");
    assert_eq!(
        task_router_outbox_count, 0,
        "channel task fanout events must not enter the router/agent event_outbox"
    );
}

#[tokio::test]
async fn channel_task_generic_create_is_group_agent_only_and_idempotent() {
    let _env = ChannelTaskEnvGuard::enabled();
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let operator = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-channel-task-agent-create".into(),
            principal_type: PrincipalType::Human,
            name: "Operator".into(),
            avatar_url: None,
        })
        .unwrap();
    let human = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: operator.workspace_id.clone(),
            principal_type: PrincipalType::Human,
            name: "Human".into(),
            avatar_url: None,
        })
        .unwrap();
    let visible_agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Visible Agent".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let peer_agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Peer Agent".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let non_member_agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Non Member Agent".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let internal_agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Internal Agent".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: Some(choruz_domain::ChannelVisibility::Internal),
        })
        .unwrap()
        .principal;
    let conversation = app
        .create_group(CreateGroupRequest {
            actor_id: operator.id.clone(),
            name: "Agent Task Creates".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![
                human.id.clone(),
                visible_agent.id.clone(),
                peer_agent.id.clone(),
                internal_agent.id.clone(),
            ],
            workspace_id: None,
        })
        .unwrap();
    let direct_conversation = app
        .create_direct_conversation(CreateDirectConversationRequest {
            actor_id: human.id.clone(),
            peer_principal_id: visible_agent.id.clone(),
            workspace_id: None,
        })
        .unwrap();
    for principal in [
        &operator,
        &human,
        &visible_agent,
        &peer_agent,
        &non_member_agent,
        &internal_agent,
    ] {
        seed_principal_to_db(&database.database_url, principal).await;
    }
    seed_conversation_to_db(&database.database_url, &conversation).await;
    seed_conversation_to_db(&database.database_url, &direct_conversation).await;

    let router = router_with_db(app, &database.database_url);
    let create_uri = format!("/v1/conversations/{}/tasks", conversation.id);

    let (human_status, _) = api_json_payload_request(
        router.clone(),
        &human,
        Method::POST,
        create_uri.clone(),
        json!({
            "task_key": "AG-0",
            "title": "Human direct create is not in MVP",
            "idempotency_key": "human-turn-1"
        }),
    )
    .await;
    assert_eq!(human_status, StatusCode::FORBIDDEN);

    let (internal_actor_status, _) = api_json_payload_request(
        router.clone(),
        &internal_agent,
        Method::POST,
        create_uri.clone(),
        json!({
            "task_key": "AG-0",
            "title": "Internal actor cannot create visible tasks",
            "idempotency_key": "internal-agent-turn-1"
        }),
    )
    .await;
    assert_eq!(internal_actor_status, StatusCode::FORBIDDEN);

    let (non_member_actor_status, _) = api_json_payload_request(
        router.clone(),
        &non_member_agent,
        Method::POST,
        create_uri.clone(),
        json!({
            "task_key": "AG-0",
            "title": "Non-member actor cannot create visible tasks",
            "idempotency_key": "non-member-agent-turn-1"
        }),
    )
    .await;
    assert_eq!(non_member_actor_status, StatusCode::FORBIDDEN);

    let (missing_idempotency_status, _) = api_json_payload_request(
        router.clone(),
        &visible_agent,
        Method::POST,
        create_uri.clone(),
        json!({
            "task_key": "AG-1",
            "title": "Missing idempotency",
            "idempotency_key": " "
        }),
    )
    .await;
    assert_eq!(missing_idempotency_status, StatusCode::BAD_REQUEST);

    let (blank_title_status, _) = api_json_payload_request(
        router.clone(),
        &visible_agent,
        Method::POST,
        create_uri.clone(),
        json!({
            "task_key": "AG-1",
            "title": " ",
            "idempotency_key": "agent-turn-blank-title"
        }),
    )
    .await;
    assert_eq!(blank_title_status, StatusCode::BAD_REQUEST);

    let (punctuation_title_status, _) = api_json_payload_request(
        router.clone(),
        &visible_agent,
        Method::POST,
        create_uri.clone(),
        json!({
            "task_key": "AG-1",
            "title": "...",
            "idempotency_key": "agent-turn-punctuation-title"
        }),
    )
    .await;
    assert_eq!(punctuation_title_status, StatusCode::BAD_REQUEST);

    let (omitted_idempotency_status, _) = api_json_payload_request(
        router.clone(),
        &visible_agent,
        Method::POST,
        create_uri.clone(),
        json!({
            "task_key": "AG-1",
            "title": "Missing idempotency field"
        }),
    )
    .await;
    assert_eq!(omitted_idempotency_status, StatusCode::BAD_REQUEST);

    let agent_payload = json!({
        "task_key": "AG-1",
        "title": "Design backend task command path",
        "idempotency_key": "agent-turn-1"
    });
    let (create_status, created) = api_json_payload_request(
        router.clone(),
        &visible_agent,
        Method::POST,
        create_uri.clone(),
        agent_payload.clone(),
    )
    .await;
    assert_eq!(create_status, StatusCode::CREATED);
    assert_eq!(created["task_key"], "AG-1");
    assert_eq!(created["source_kind"], "agent");
    assert_eq!(created["status"], "todo");
    assert_eq!(created["assignee_principal_id"], visible_agent.id);
    assert_eq!(created["created_by"], visible_agent.id);
    assert_eq!(created["version"], 1);
    let task_id = created["task_id"].as_str().unwrap().to_string();

    let (repeat_status, repeated) = api_json_payload_request(
        router.clone(),
        &visible_agent,
        Method::POST,
        create_uri.clone(),
        agent_payload,
    )
    .await;
    assert_eq!(repeat_status, StatusCode::OK);
    assert_eq!(repeated["task_id"], task_id);
    assert_eq!(repeated["task_key"], "AG-1");
    assert_eq!(repeated["title"], "Design backend task command path");
    assert_eq!(repeated["version"], 1);

    let (conflict_status, _) = api_json_payload_request(
        router.clone(),
        &visible_agent,
        Method::POST,
        create_uri.clone(),
        json!({
            "task_key": "AG-1-CHANGED",
            "title": "Changed payload conflicts",
            "idempotency_key": "agent-turn-1"
        }),
    )
    .await;
    assert_eq!(conflict_status, StatusCode::CONFLICT);

    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect for generic channel task assertions");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let task_row_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM group_workflow_task
             WHERE conversation_id = $1
               AND created_by = $2
               AND idempotency_key = 'agent-turn-1'",
            &[&conversation.id, &visible_agent.id],
        )
        .await
        .expect("count idempotent generic task rows")
        .get::<_, i64>("count");
    assert_eq!(task_row_count, 1);
    let task_event_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM group_workflow_event
             WHERE task_id = $1 AND kind = 'channel_task.created'",
            &[&task_id],
        )
        .await
        .expect("count channel task create events")
        .get::<_, i64>("count");
    assert_eq!(task_event_count, 1);
    let owner_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM group_workflow_task_participant
             WHERE task_id = $1 AND principal_id = $2 AND role_key = 'owner'",
            &[&task_id, &visible_agent.id],
        )
        .await
        .expect("count owner participant")
        .get::<_, i64>("count");
    assert_eq!(owner_count, 1);
    let task_fanout_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM conversation_events
             WHERE conversation_id = $1
               AND event_type = 'channel_task.created'
               AND metadata->>'task_id' = $2",
            &[&conversation.id, &task_id],
        )
        .await
        .expect("count generic channel task fanout events")
        .get::<_, i64>("count");
    assert_eq!(task_fanout_count, 1);
    let chat_event_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM conversation_events
             WHERE conversation_id = $1
               AND event_type IN ('message', 'message.created', 'reply')",
            &[&conversation.id],
        )
        .await
        .expect("count chat events")
        .get::<_, i64>("count");
    assert_eq!(chat_event_count, 0);

    // Agents may omit `task_key`; the server synthesizes a TASK-{N} key.
    let (auto_key_status, auto_key_created) = api_json_payload_request(
        router.clone(),
        &visible_agent,
        Method::POST,
        create_uri.clone(),
        json!({
            "title": "Server allocates task key when agent omits it",
            "idempotency_key": "agent-turn-auto-key"
        }),
    )
    .await;
    assert_eq!(auto_key_status, StatusCode::CREATED);
    let auto_key_task_id = auto_key_created["task_id"].as_str().unwrap().to_string();
    let auto_key = auto_key_created["task_key"]
        .as_str()
        .expect("task_key returned");
    assert!(
        auto_key.starts_with("TASK-"),
        "expected server-generated TASK- prefix, got {auto_key}"
    );
    assert_eq!(auto_key_created["source_kind"], "agent");

    // Replaying the same idempotency_key without a task_key returns the original task,
    // not a freshly-allocated key.
    let (auto_key_repeat_status, auto_key_repeat) = api_json_payload_request(
        router.clone(),
        &visible_agent,
        Method::POST,
        create_uri.clone(),
        json!({
            "title": "Server allocates task key when agent omits it",
            "idempotency_key": "agent-turn-auto-key"
        }),
    )
    .await;
    assert_eq!(auto_key_repeat_status, StatusCode::OK);
    assert_eq!(auto_key_repeat["task_id"], auto_key_created["task_id"]);
    assert_eq!(auto_key_repeat["task_key"], auto_key_created["task_key"]);
    assert_eq!(
        auto_key_repeat["title"],
        "Server allocates task key when agent omits it"
    );
    assert_eq!(auto_key_repeat["version"], 1);

    let auto_key_task_row_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM group_workflow_task
             WHERE conversation_id = $1
               AND created_by = $2
               AND idempotency_key = 'agent-turn-auto-key'",
            &[&conversation.id, &visible_agent.id],
        )
        .await
        .expect("count auto-key idempotent task rows")
        .get::<_, i64>("count");
    assert_eq!(auto_key_task_row_count, 1);
    let auto_key_owner_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM group_workflow_task_participant
             WHERE task_id = $1 AND principal_id = $2 AND role_key = 'owner'",
            &[&auto_key_task_id, &visible_agent.id],
        )
        .await
        .expect("count auto-key owner participant")
        .get::<_, i64>("count");
    assert_eq!(auto_key_owner_count, 1);
    let auto_key_workflow_event_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM group_workflow_event
             WHERE task_id = $1 AND kind = 'channel_task.created'",
            &[&auto_key_task_id],
        )
        .await
        .expect("count auto-key workflow create events")
        .get::<_, i64>("count");
    assert_eq!(auto_key_workflow_event_count, 1);
    let auto_key_fanout_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM conversation_events
             WHERE conversation_id = $1
               AND event_type = 'channel_task.created'
               AND metadata->>'task_id' = $2",
            &[&conversation.id, &auto_key_task_id],
        )
        .await
        .expect("count auto-key channel task fanout events")
        .get::<_, i64>("count");
    assert_eq!(auto_key_fanout_count, 1);

    // Seed an explicit `TASK-{next}` card so the next auto-allocation request
    // collides with the sequence value the allocator would otherwise pick.
    // The skip-loop must advance past the conflict instead of wedging.
    let auto_key_number: i64 = auto_key
        .strip_prefix("TASK-")
        .and_then(|n| n.parse().ok())
        .expect("auto key has TASK-{N} shape");
    let blocker_key = format!("TASK-{}", auto_key_number + 1);
    client
        .execute(
            "INSERT INTO group_workflow_task
                (id, conversation_id, task_key, title, status, assignee_principal_id,
                 source_kind, created_by, created_at, updated_at)
             VALUES ($1, $2, $3, 'Manually keyed collider', 'todo', $4,
                     'agent', $4, NOW(), NOW())",
            &[
                &Uuid::now_v7().to_string(),
                &conversation.id,
                &blocker_key,
                &visible_agent.id,
            ],
        )
        .await
        .expect("seed colliding explicit TASK key");

    let (skip_status, skip_created) = api_json_payload_request(
        router.clone(),
        &visible_agent,
        Method::POST,
        create_uri.clone(),
        json!({
            "title": "Auto allocator skips collisions",
            "idempotency_key": "agent-turn-auto-key-skip"
        }),
    )
    .await;
    assert_eq!(skip_status, StatusCode::CREATED);
    let skip_key = skip_created["task_key"].as_str().expect("skip key");
    assert_ne!(skip_key, blocker_key, "must skip past explicit collider");
    assert!(
        skip_key.starts_with("TASK-"),
        "expected TASK- prefix on skip allocation, got {skip_key}"
    );

    let (human_assignee_status, _) = api_json_payload_request(
        router.clone(),
        &visible_agent,
        Method::POST,
        create_uri.clone(),
        json!({
            "task_key": "AG-2",
            "title": "Agents cannot assign humans",
            "assignee_principal_id": human.id,
            "idempotency_key": "agent-turn-2"
        }),
    )
    .await;
    assert_eq!(human_assignee_status, StatusCode::BAD_REQUEST);

    let (internal_assignee_status, _) = api_json_payload_request(
        router.clone(),
        &visible_agent,
        Method::POST,
        create_uri.clone(),
        json!({
            "task_key": "AG-3",
            "title": "Internal assignee stays hidden",
            "assignee_principal_id": internal_agent.id,
            "idempotency_key": "agent-turn-3"
        }),
    )
    .await;
    assert_eq!(internal_assignee_status, StatusCode::BAD_REQUEST);

    let (peer_create_status, peer_created) = api_json_payload_request(
        router.clone(),
        &visible_agent,
        Method::POST,
        create_uri,
        json!({
            "task_key": "AG-4",
            "title": "Peer agent can own visible work",
            "assignee_principal_id": peer_agent.id,
            "status": "in_progress",
            "context_label": "Agent commands",
            "idempotency_key": "agent-turn-4"
        }),
    )
    .await;
    assert_eq!(peer_create_status, StatusCode::CREATED);
    assert_eq!(peer_created["assignee_principal_id"], peer_agent.id);
    assert_eq!(peer_created["status"], "in_progress");
    assert_eq!(peer_created["context_label"], "Agent commands");
    let peer_task_id = peer_created["task_id"].as_str().unwrap().to_string();

    client
        .execute(
            "UPDATE conversation_member
             SET removed_at = NOW()
             WHERE conv_id = $1 AND principal_id = $2",
            &[&conversation.id, &peer_agent.id],
        )
        .await
        .expect("remove peer assignee");
    let (stale_assignee_retry_status, stale_assignee_retry) = api_json_payload_request(
        router.clone(),
        &visible_agent,
        Method::POST,
        format!("/v1/conversations/{}/tasks", conversation.id),
        json!({
            "task_key": "AG-4",
            "title": "Peer agent can own visible work",
            "assignee_principal_id": peer_agent.id,
            "status": "in_progress",
            "context_label": "Agent commands",
            "idempotency_key": "agent-turn-4"
        }),
    )
    .await;
    assert_eq!(stale_assignee_retry_status, StatusCode::OK);
    assert_eq!(stale_assignee_retry["task_id"], peer_task_id);

    let (direct_status, _) = api_json_payload_request(
        router,
        &visible_agent,
        Method::POST,
        format!("/v1/conversations/{}/tasks", direct_conversation.id),
        json!({
            "task_key": "DM-1",
            "title": "Direct agent commands are unsupported",
            "idempotency_key": "direct-agent-turn-1"
        }),
    )
    .await;
    assert_eq!(direct_status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn channel_task_agent_patch_rules_enforce_owner_and_coordinator_scope() {
    let _env = ChannelTaskEnvGuard::enabled();
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let operator = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-channel-task-agent-patch".into(),
            principal_type: PrincipalType::Human,
            name: "Operator".into(),
            avatar_url: None,
        })
        .unwrap();
    let human = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: operator.workspace_id.clone(),
            principal_type: PrincipalType::Human,
            name: "Human".into(),
            avatar_url: None,
        })
        .unwrap();
    let owner_agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Owner Agent".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let peer_agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Peer Agent".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let coordinator_agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Coordinator Agent".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let conversation = app
        .create_group(CreateGroupRequest {
            actor_id: operator.id.clone(),
            name: "Agent Patch Rules".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![
                human.id.clone(),
                owner_agent.id.clone(),
                peer_agent.id.clone(),
                coordinator_agent.id.clone(),
            ],
            workspace_id: None,
        })
        .unwrap();
    let direct_conversation = app
        .create_direct_conversation(CreateDirectConversationRequest {
            actor_id: human.id.clone(),
            peer_principal_id: owner_agent.id.clone(),
            workspace_id: None,
        })
        .unwrap();
    for principal in [
        &operator,
        &human,
        &owner_agent,
        &peer_agent,
        &coordinator_agent,
    ] {
        seed_principal_to_db(&database.database_url, principal).await;
    }
    seed_conversation_to_db(&database.database_url, &conversation).await;
    seed_conversation_to_db(&database.database_url, &direct_conversation).await;

    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect for channel task agent patch seed");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "
            INSERT INTO conversation_runtime_policies
              (conversation_id, default_coordinator_agent_id)
            VALUES ($1, $2)
            ",
            &[&conversation.id, &coordinator_agent.id],
        )
        .await
        .expect("seed coordinator policy");
    client
        .execute(
            "
            INSERT INTO group_workflow_task
              (id, conversation_id, task_key, title, status, assignee_principal_id,
               source_kind, created_by, version)
            VALUES
              ('agent-owned-task', $1, 'AGP-1', 'Owned task', 'todo', $2, 'agent', $2, 1),
              ('agent-human-assign-task', $1, 'AGP-2', 'Human assignment blocked', 'todo', $2, 'agent', $2, 1),
              ('agent-unowned-task', $1, 'AGP-3', 'Unowned task', 'todo', $3, 'agent', $3, 1),
              ('direct-agent-task', $4, 'DM-1', 'Direct task', 'todo', $2, 'agent', $2, 1)
            ",
            &[
                &conversation.id,
                &owner_agent.id,
                &peer_agent.id,
                &direct_conversation.id,
            ],
        )
        .await
        .expect("seed channel tasks");

    let router = router_with_db(app, &database.database_url);
    let (owner_status, owner_patched) = api_json_payload_request(
        router.clone(),
        &owner_agent,
        Method::PATCH,
        "/v1/tasks/agent-owned-task".to_string(),
        json!({
            "status": "in_progress",
            "blocked_reason": "Waiting for smoke"
        }),
    )
    .await;
    assert_eq!(owner_status, StatusCode::OK);
    assert_eq!(owner_patched["status"], "in_progress");
    assert_eq!(owner_patched["blocked_reason"], "Waiting for smoke");
    assert_eq!(owner_patched["version"], 2);

    let (owner_transfer_status, owner_transferred) = api_json_payload_request(
        router.clone(),
        &owner_agent,
        Method::PATCH,
        "/v1/tasks/agent-owned-task".to_string(),
        json!({
            "assignee_principal_id": peer_agent.id
        }),
    )
    .await;
    assert_eq!(owner_transfer_status, StatusCode::OK);
    assert_eq!(owner_transferred["assignee_principal_id"], peer_agent.id);
    assert_eq!(owner_transferred["version"], 3);

    let (human_assign_status, _) = api_json_payload_request(
        router.clone(),
        &owner_agent,
        Method::PATCH,
        "/v1/tasks/agent-human-assign-task".to_string(),
        json!({
            "assignee_principal_id": human.id
        }),
    )
    .await;
    assert_eq!(human_assign_status, StatusCode::BAD_REQUEST);

    let (unowned_status, _) = api_json_payload_request(
        router.clone(),
        &owner_agent,
        Method::PATCH,
        "/v1/tasks/agent-unowned-task".to_string(),
        json!({
            "status": "done"
        }),
    )
    .await;
    assert_eq!(unowned_status, StatusCode::FORBIDDEN);

    let (coordinator_status, coordinator_patched) = api_json_payload_request(
        router.clone(),
        &coordinator_agent,
        Method::PATCH,
        "/v1/tasks/agent-unowned-task".to_string(),
        json!({
            "status": "blocked",
            "assignee_principal_id": owner_agent.id,
            "blocked_reason": "Coordinator intervention"
        }),
    )
    .await;
    assert_eq!(coordinator_status, StatusCode::OK);
    assert_eq!(coordinator_patched["status"], "blocked");
    assert_eq!(coordinator_patched["assignee_principal_id"], owner_agent.id);
    assert_eq!(
        coordinator_patched["blocked_reason"],
        "Coordinator intervention"
    );

    let (coordinator_human_assign_status, _) = api_json_payload_request(
        router.clone(),
        &coordinator_agent,
        Method::PATCH,
        "/v1/tasks/agent-unowned-task".to_string(),
        json!({
            "assignee_principal_id": human.id
        }),
    )
    .await;
    assert_eq!(coordinator_human_assign_status, StatusCode::BAD_REQUEST);

    let (direct_agent_status, _) = api_json_payload_request(
        router,
        &owner_agent,
        Method::PATCH,
        "/v1/tasks/direct-agent-task".to_string(),
        json!({
            "status": "done"
        }),
    )
    .await;
    assert_eq!(direct_agent_status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn channel_task_mutation_contract_rejects_title_and_description_fields() {
    let _env = ChannelTaskEnvGuard::enabled();
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let operator = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-channel-task-contract".into(),
            principal_type: PrincipalType::Human,
            name: "Operator".into(),
            avatar_url: None,
        })
        .unwrap();
    let human = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: operator.workspace_id.clone(),
            principal_type: PrincipalType::Human,
            name: "Human".into(),
            avatar_url: None,
        })
        .unwrap();
    let visible_agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Visible Agent".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let conversation = app
        .create_group(CreateGroupRequest {
            actor_id: operator.id.clone(),
            name: "Channel Task Contract".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![human.id.clone(), visible_agent.id.clone()],
            workspace_id: None,
        })
        .unwrap();
    for principal in [&operator, &human, &visible_agent] {
        seed_principal_to_db(&database.database_url, principal).await;
    }
    seed_conversation_to_db(&database.database_url, &conversation).await;

    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect for mutation contract seed");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "
            INSERT INTO conversation_events
              (conversation_id, seq, event_id, event_type, sender_id, content, content_type)
            VALUES
              ($1, 1, 'contract-message-1', 'message', $2, 'Please track this', 'text')
            ",
            &[&conversation.id, &human.id],
        )
        .await
        .expect("seed mutation contract message");

    let router = router_with_db(app, &database.database_url);
    let from_message_uri = format!("/v1/conversations/{}/tasks/from-message", conversation.id);
    let generic_create_uri = format!("/v1/conversations/{}/tasks", conversation.id);

    // Seed a task we can patch.
    let (create_status, created) = api_json_payload_request(
        router.clone(),
        &human,
        Method::POST,
        from_message_uri.clone(),
        json!({
            "message_id": "contract-message-1",
            "title": "Track launch follow-up",
            "assignee_principal_id": visible_agent.id,
            "idempotency_key": "contract-1"
        }),
    )
    .await;
    assert_eq!(create_status, StatusCode::CREATED);
    let task_id = created["task_id"].as_str().unwrap().to_string();
    let initial_version = created["version"].as_i64().unwrap();
    let initial_title = created["title"].as_str().unwrap().to_string();

    // Post-creation title edits are not in MVP; PATCH must reject the field.
    let (title_patch_status, title_patch_body) = api_json_payload_request(
        router.clone(),
        &human,
        Method::PATCH,
        format!("/v1/tasks/{task_id}"),
        json!({
            "status": "in_progress",
            "title": "Renamed after creation"
        }),
    )
    .await;
    assert_eq!(title_patch_status, StatusCode::BAD_REQUEST);
    assert!(
        title_patch_body["error"]["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("invalid JSON body"),
        "title patch should be rejected as an unknown field, got {title_patch_body:?}"
    );

    // PATCH must also reject the legacy description field.
    let (description_patch_status, _) = api_json_payload_request(
        router.clone(),
        &human,
        Method::PATCH,
        format!("/v1/tasks/{task_id}"),
        json!({
            "status": "in_progress",
            "description": "Long form descriptions are not in MVP"
        }),
    )
    .await;
    assert_eq!(description_patch_status, StatusCode::BAD_REQUEST);

    // The rejected patches must not have mutated the task or appended new events.
    let (after_status, after_body) = api_json_request(
        router.clone(),
        &human,
        Method::GET,
        format!("/v1/tasks/{task_id}"),
    )
    .await;
    assert_eq!(after_status, StatusCode::OK);
    assert_eq!(after_body["task"]["version"], initial_version);
    assert_eq!(after_body["task"]["status"], "todo");
    assert_eq!(after_body["task"]["title"], initial_title);

    // POST /tasks/from-message must reject description payloads.
    let (from_message_desc_status, _) = api_json_payload_request(
        router.clone(),
        &human,
        Method::POST,
        from_message_uri,
        json!({
            "message_id": "contract-message-1",
            "title": "Another title",
            "assignee_principal_id": visible_agent.id,
            "description": "Long form descriptions are not in MVP",
            "idempotency_key": "contract-2"
        }),
    )
    .await;
    assert_eq!(from_message_desc_status, StatusCode::BAD_REQUEST);

    // Generic POST /tasks must reject description payloads too (agent path).
    let (generic_desc_status, _) = api_json_payload_request(
        router,
        &visible_agent,
        Method::POST,
        generic_create_uri,
        json!({
            "task_key": "CONTRACT-1",
            "title": "Agent path also rejects description",
            "idempotency_key": "agent-contract-1",
            "description": "Long form descriptions are not in MVP"
        }),
    )
    .await;
    assert_eq!(generic_desc_status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn channel_task_assignee_validation_rejects_missing_invalid_and_cross_workspace() {
    let _env = ChannelTaskEnvGuard::enabled();
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();

    let admin_a = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-channel-task-assignee-a".into(),
            principal_type: PrincipalType::Human,
            name: "Operator A".into(),
            avatar_url: None,
        })
        .unwrap();
    let human_a = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: admin_a.workspace_id.clone(),
            principal_type: PrincipalType::Human,
            name: "Human A".into(),
            avatar_url: None,
        })
        .unwrap();
    let peer_human_a = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: admin_a.workspace_id.clone(),
            principal_type: PrincipalType::Human,
            name: "Peer Human A".into(),
            avatar_url: None,
        })
        .unwrap();
    let visible_agent_a = app
        .create_agent(CreateAgentRequest {
            actor_id: admin_a.id.clone(),
            name: "Visible Agent A".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let non_member_human_a = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: admin_a.workspace_id.clone(),
            principal_type: PrincipalType::Human,
            name: "Non-Member Human A".into(),
            avatar_url: None,
        })
        .unwrap();
    let removed_agent_a = app
        .create_agent(CreateAgentRequest {
            actor_id: admin_a.id.clone(),
            name: "Removed Agent A".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let disabled_agent_a = app
        .create_agent(CreateAgentRequest {
            actor_id: admin_a.id.clone(),
            name: "Disabled Agent A".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let deleted_agent_a = app
        .create_agent(CreateAgentRequest {
            actor_id: admin_a.id.clone(),
            name: "Deleted Agent A".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let internal_agent_a = app
        .create_agent(CreateAgentRequest {
            actor_id: admin_a.id.clone(),
            name: "Internal Agent A".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: Some(choruz_domain::ChannelVisibility::Internal),
        })
        .unwrap()
        .principal;

    let admin_b = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-channel-task-assignee-b".into(),
            principal_type: PrincipalType::Human,
            name: "Operator B".into(),
            avatar_url: None,
        })
        .unwrap();
    let cross_workspace_agent = app
        .create_agent(CreateAgentRequest {
            actor_id: admin_b.id.clone(),
            name: "Cross-Workspace Agent".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    assert_ne!(
        cross_workspace_agent.workspace_id, admin_a.workspace_id,
        "cross-workspace assignee must actually be in a different workspace"
    );

    let conversation = app
        .create_group(CreateGroupRequest {
            actor_id: admin_a.id.clone(),
            name: "Assignee Validation".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![
                human_a.id.clone(),
                peer_human_a.id.clone(),
                visible_agent_a.id.clone(),
                removed_agent_a.id.clone(),
                disabled_agent_a.id.clone(),
                deleted_agent_a.id.clone(),
                internal_agent_a.id.clone(),
            ],
            workspace_id: None,
        })
        .unwrap();

    for principal in [
        &admin_a,
        &human_a,
        &peer_human_a,
        &visible_agent_a,
        &non_member_human_a,
        &removed_agent_a,
        &disabled_agent_a,
        &deleted_agent_a,
        &internal_agent_a,
        &admin_b,
        &cross_workspace_agent,
    ] {
        seed_principal_to_db(&database.database_url, principal).await;
    }
    seed_conversation_to_db(&database.database_url, &conversation).await;

    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect for assignee validation seed");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    // Remove this assignee AFTER it has been added as a conversation member.
    // This isolates the active-membership predicate (`removed_at IS NULL`).
    let removed_rows = client
        .execute(
            "UPDATE conversation_member SET removed_at = NOW() WHERE conv_id = $1 AND principal_id = $2",
            &[&conversation.id, &removed_agent_a.id],
        )
        .await
        .expect("remove removed_agent_a from conversation");
    assert_eq!(
        removed_rows, 1,
        "expected exactly one conversation membership row to be removed for the fixture"
    );
    // Disable the would-be assignee AFTER it has been added as a conversation member.
    // This isolates the disabled-principal predicate (`p.disabled = FALSE`) from the
    // membership predicate, so a rejection here can only be attributed to `disabled`.
    let disabled_rows = client
        .execute(
            "UPDATE principal SET disabled = TRUE WHERE id = $1",
            &[&disabled_agent_a.id],
        )
        .await
        .expect("disable disabled_agent_a principal");
    assert_eq!(
        disabled_rows, 1,
        "expected exactly one principal row to be disabled for the fixture"
    );
    // Soft-delete this assignee AFTER membership seeding to exercise deleted_at,
    // not absence from the conversation.
    let deleted_rows = client
        .execute(
            "UPDATE principal SET deleted_at = NOW() WHERE id = $1",
            &[&deleted_agent_a.id],
        )
        .await
        .expect("soft-delete deleted_agent_a principal");
    assert_eq!(
        deleted_rows, 1,
        "expected exactly one principal row to be deleted for the fixture"
    );
    client
        .execute(
            "
            INSERT INTO conversation_events
              (conversation_id, seq, event_id, event_type, sender_id, content, content_type)
            VALUES
              ($1, 1, 'assignee-validation-message-1', 'message', $2, 'Please track this', 'text'),
              ($1, 2, 'assignee-validation-message-2', 'message', $2, 'Please track this too', 'text')
            ",
            &[&conversation.id, &human_a.id],
        )
        .await
        .expect("seed assignee validation message");

    let router = router_with_db(app, &database.database_url);
    let from_message_uri = format!("/v1/conversations/{}/tasks/from-message", conversation.id);
    let generic_create_uri = format!("/v1/conversations/{}/tasks", conversation.id);

    let nonexistent_principal_id = "prn_nonexistent_principal_for_assignee_validation";

    // --- from-message: visible human assignee is valid for human-created tasks ---
    let (visible_human_status, visible_human_body) = api_json_payload_request(
        router.clone(),
        &human_a,
        Method::POST,
        from_message_uri.clone(),
        json!({
            "message_id": "assignee-validation-message-2",
            "title": "Visible human assignee should pass",
            "assignee_principal_id": peer_human_a.id,
            "idempotency_key": "assignee-visible-human-1"
        }),
    )
    .await;
    assert_eq!(visible_human_status, StatusCode::CREATED);
    assert_eq!(visible_human_body["assignee_principal_id"], peer_human_a.id);
    assert_eq!(visible_human_body["assignee_type"], "human");

    // --- from-message: blank assignee ---
    let blank_assignee_value = "   ";
    let (blank_assignee_status, blank_assignee_body) = api_json_payload_request(
        router.clone(),
        &human_a,
        Method::POST,
        from_message_uri.clone(),
        json!({
            "message_id": "assignee-validation-message-1",
            "title": "Blank assignee should fail",
            "assignee_principal_id": blank_assignee_value,
            "idempotency_key": "assignee-blank-1"
        }),
    )
    .await;
    assert_eq!(blank_assignee_status, StatusCode::BAD_REQUEST);
    let blank_detail = validation_detail(&blank_assignee_body);
    assert!(
        !blank_detail.is_empty(),
        "blank assignee response must carry a validation detail, got {blank_assignee_body:?}"
    );
    assert!(
        blank_detail.contains("assignee_principal_id"),
        "blank assignee detail must name the rejected field, got {blank_detail:?}"
    );
    assert_safe_detail(
        &blank_assignee_body,
        blank_assignee_value,
        "from-message blank assignee",
    );

    // --- from-message: non-existent assignee ---
    let (missing_assignee_status, missing_assignee_body) = api_json_payload_request(
        router.clone(),
        &human_a,
        Method::POST,
        from_message_uri.clone(),
        json!({
            "message_id": "assignee-validation-message-1",
            "title": "Non-existent assignee should fail",
            "assignee_principal_id": nonexistent_principal_id,
            "idempotency_key": "assignee-missing-1"
        }),
    )
    .await;
    assert_eq!(missing_assignee_status, StatusCode::BAD_REQUEST);
    assert_safe_detail(
        &missing_assignee_body,
        nonexistent_principal_id,
        "from-message non-existent assignee",
    );

    // --- from-message: cross-workspace assignee ---
    let (cross_workspace_from_message_status, cross_workspace_from_message_body) =
        api_json_payload_request(
            router.clone(),
            &human_a,
            Method::POST,
            from_message_uri.clone(),
            json!({
                "message_id": "assignee-validation-message-1",
                "title": "Cross-workspace assignee should fail",
                "assignee_principal_id": cross_workspace_agent.id,
                "idempotency_key": "assignee-cross-workspace-1"
            }),
        )
        .await;
    assert_eq!(cross_workspace_from_message_status, StatusCode::BAD_REQUEST);
    assert_safe_detail(
        &cross_workspace_from_message_body,
        &cross_workspace_agent.id,
        "from-message cross-workspace assignee",
    );

    // --- generic create (agent path): cross-workspace assignee ---
    let (cross_workspace_generic_status, cross_workspace_generic_body) = api_json_payload_request(
        router.clone(),
        &visible_agent_a,
        Method::POST,
        generic_create_uri.clone(),
        json!({
            "task_key": "AV-1",
            "title": "Agent cannot assign cross-workspace peer",
            "assignee_principal_id": cross_workspace_agent.id,
            "idempotency_key": "assignee-cross-workspace-agent-1"
        }),
    )
    .await;
    assert_eq!(cross_workspace_generic_status, StatusCode::BAD_REQUEST);
    assert_safe_detail(
        &cross_workspace_generic_body,
        &cross_workspace_agent.id,
        "generic create cross-workspace assignee",
    );

    for (case, task_key, assignee_principal_id, idempotency_key) in [
        (
            "generic create removed member assignee",
            "AV-2",
            removed_agent_a.id.as_str(),
            "assignee-removed-agent-generic-1",
        ),
        (
            "generic create disabled assignee",
            "AV-3",
            disabled_agent_a.id.as_str(),
            "assignee-disabled-agent-generic-1",
        ),
        (
            "generic create deleted assignee",
            "AV-4",
            deleted_agent_a.id.as_str(),
            "assignee-deleted-agent-generic-1",
        ),
        (
            "generic create internal assignee",
            "AV-5",
            internal_agent_a.id.as_str(),
            "assignee-internal-agent-generic-1",
        ),
    ] {
        let (generic_status, generic_body) = api_json_payload_request(
            router.clone(),
            &visible_agent_a,
            Method::POST,
            generic_create_uri.clone(),
            json!({
                "task_key": task_key,
                "title": case,
                "assignee_principal_id": assignee_principal_id,
                "idempotency_key": idempotency_key
            }),
        )
        .await;
        assert_eq!(generic_status, StatusCode::BAD_REQUEST, "{case}");
        assert_safe_detail(&generic_body, assignee_principal_id, case);
    }

    // --- from-message: removed member assignee ---
    let (removed_assignee_status, removed_assignee_body) = api_json_payload_request(
        router.clone(),
        &human_a,
        Method::POST,
        from_message_uri.clone(),
        json!({
            "message_id": "assignee-validation-message-1",
            "title": "Removed member assignee should fail",
            "assignee_principal_id": removed_agent_a.id,
            "idempotency_key": "assignee-removed-1"
        }),
    )
    .await;
    assert_eq!(removed_assignee_status, StatusCode::BAD_REQUEST);
    assert_safe_detail(
        &removed_assignee_body,
        &removed_agent_a.id,
        "from-message removed member assignee",
    );

    // --- from-message: non-member actor must be forbidden, not silently accepted ---
    // `non_member_human_a` is in the same workspace but never joined the conversation,
    // so `require_channel_task_member_access_tx` must reject the mutation before any
    // assignee work runs. This complements the assignee-side validations.
    let (non_member_status, non_member_body) = api_json_payload_request(
        router.clone(),
        &non_member_human_a,
        Method::POST,
        from_message_uri.clone(),
        json!({
            "message_id": "assignee-validation-message-1",
            "title": "Non-member actor must be forbidden",
            "assignee_principal_id": visible_agent_a.id,
            "idempotency_key": "assignee-non-member-actor-1"
        }),
    )
    .await;
    assert_eq!(non_member_status, StatusCode::FORBIDDEN);
    let non_member_detail = validation_detail(&non_member_body);
    assert!(
        !non_member_detail.is_empty(),
        "non-member actor response must carry a non-empty error.detail, got {non_member_body:?}"
    );
    assert!(
        non_member_detail.contains("member"),
        "non-member actor detail must name the rejected concept (\"member\"), got {non_member_detail:?}"
    );
    assert!(
        !non_member_detail.contains(&non_member_human_a.id),
        "non-member actor detail must not echo the actor principal id, got {non_member_detail:?}"
    );

    // --- from-message: disabled assignee must be rejected ---
    // `disabled_agent_a` IS a conversation member, but its principal row has
    // `disabled = TRUE`, so `validate_visible_channel_task_assignee_tx`'s
    // `p.disabled = FALSE` predicate must reject it.
    let (disabled_assignee_status, disabled_assignee_body) = api_json_payload_request(
        router.clone(),
        &human_a,
        Method::POST,
        from_message_uri.clone(),
        json!({
            "message_id": "assignee-validation-message-1",
            "title": "Disabled assignee should fail",
            "assignee_principal_id": disabled_agent_a.id,
            "idempotency_key": "assignee-disabled-1"
        }),
    )
    .await;
    assert_eq!(disabled_assignee_status, StatusCode::BAD_REQUEST);
    assert_safe_detail(
        &disabled_assignee_body,
        &disabled_agent_a.id,
        "from-message disabled assignee",
    );

    // --- from-message: soft-deleted assignee must be rejected ---
    let (deleted_assignee_status, deleted_assignee_body) = api_json_payload_request(
        router.clone(),
        &human_a,
        Method::POST,
        from_message_uri.clone(),
        json!({
            "message_id": "assignee-validation-message-1",
            "title": "Deleted assignee should fail",
            "assignee_principal_id": deleted_agent_a.id,
            "idempotency_key": "assignee-deleted-1"
        }),
    )
    .await;
    assert_eq!(deleted_assignee_status, StatusCode::BAD_REQUEST);
    assert_safe_detail(
        &deleted_assignee_body,
        &deleted_agent_a.id,
        "from-message deleted assignee",
    );

    // --- from-message: internal agent assignee must be rejected ---
    let (internal_assignee_status, internal_assignee_body) = api_json_payload_request(
        router.clone(),
        &human_a,
        Method::POST,
        from_message_uri.clone(),
        json!({
            "message_id": "assignee-validation-message-1",
            "title": "Internal agent assignee should fail",
            "assignee_principal_id": internal_agent_a.id,
            "idempotency_key": "assignee-internal-1"
        }),
    )
    .await;
    assert_eq!(internal_assignee_status, StatusCode::BAD_REQUEST);
    assert_safe_detail(
        &internal_assignee_body,
        &internal_agent_a.id,
        "from-message internal agent assignee",
    );

    // Seed a real task we can attempt to PATCH below.
    let seed_payload = json!({
        "message_id": "assignee-validation-message-1",
        "title": "Track this work",
        "assignee_principal_id": visible_agent_a.id,
        "idempotency_key": "assignee-seed-1"
    });
    let (seed_status, seed_body) = api_json_payload_request(
        router.clone(),
        &human_a,
        Method::POST,
        from_message_uri,
        seed_payload,
    )
    .await;
    assert_eq!(seed_status, StatusCode::CREATED);
    let task_id = seed_body["task_id"].as_str().unwrap().to_string();
    let baseline_version = seed_body["version"].as_i64().unwrap();
    let baseline_assignee = seed_body["assignee_principal_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(baseline_assignee, visible_agent_a.id);
    assert_eq!(seed_body["assignee_type"], "agent");

    // --- PATCH (human path): cross-workspace assignee ---
    let (cross_workspace_patch_status, cross_workspace_patch_body) = api_json_payload_request(
        router.clone(),
        &human_a,
        Method::PATCH,
        format!("/v1/tasks/{task_id}"),
        json!({
            "assignee_principal_id": cross_workspace_agent.id
        }),
    )
    .await;
    assert_eq!(cross_workspace_patch_status, StatusCode::BAD_REQUEST);
    assert_safe_detail(
        &cross_workspace_patch_body,
        &cross_workspace_agent.id,
        "patch cross-workspace assignee",
    );

    // Ensure the rejected mutations did NOT change the seed task.
    let (after_status, after_body) = api_json_request(
        router,
        &human_a,
        Method::GET,
        format!("/v1/tasks/{task_id}"),
    )
    .await;
    assert_eq!(after_status, StatusCode::OK);
    assert_eq!(after_body["task"]["version"], baseline_version);
    assert_eq!(
        after_body["task"]["assignee_principal_id"],
        baseline_assignee
    );

    // Ensure no failed validation produced a chat-like event in the conversation.
    // Use a deny-list (everything NOT in the channel_task.* family) so a future
    // chat-like event_type cannot silently slip past an allowlist.
    let non_task_event_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM conversation_events
             WHERE conversation_id = $1
               AND event_type NOT LIKE 'channel_task.%'",
            &[&conversation.id],
        )
        .await
        .expect("count non-task events")
        .get::<_, i64>("count");
    assert_eq!(
        non_task_event_count, 2,
        "only the seeded source messages should be present; failed validations must not emit any chat or non-task events"
    );

    // Ensure only accepted valid-assignee mutations persisted task rows: the
    // visible-human task plus the visible-agent seed task.
    let task_row_count = client
        .query_one(
            "SELECT COUNT(*)::BIGINT AS count
             FROM group_workflow_task
             WHERE conversation_id = $1",
            &[&conversation.id],
        )
        .await
        .expect("count workflow tasks")
        .get::<_, i64>("count");
    assert_eq!(
        task_row_count, 2,
        "failed assignee validation must not persist any extra task rows"
    );
}
