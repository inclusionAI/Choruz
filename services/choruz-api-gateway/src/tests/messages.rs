use super::*;

#[tokio::test]
async fn health_and_direct_message_flow_work() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let test_router = router_with_db(app.clone(), &database.database_url);

    let response = test_router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let health: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(health["service"], "choruz-api-gateway");
    assert_eq!(
        health["protocol_version"],
        choruz_common::HOST_SERVICE_PROTOCOL_VERSION
    );
    assert_eq!(health["status"], "ok");

    let response = test_router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/readyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let readiness: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(readiness["service"], "choruz-api-gateway");
    assert_eq!(readiness["status"], "ready");
    assert_eq!(readiness["database"], true);

    let alice = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-acme".into(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    let bob = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-acme".into(),
            principal_type: PrincipalType::Human,
            name: "Bob".into(),
            avatar_url: None,
        })
        .unwrap();
    let conversation = app
        .create_direct_conversation(CreateDirectConversationRequest {
            actor_id: alice.id.clone(),
            peer_principal_id: bob.id.clone(),
            workspace_id: None,
        })
        .unwrap();

    // Seed data to DB for DbService lookups
    seed_principal_to_db(&database.database_url, &alice).await;
    seed_principal_to_db(&database.database_url, &bob).await;
    seed_conversation_to_db(&database.database_url, &conversation).await;

    let response = test_router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/messages")
                .header("authorization", format!("Bearer {}", session_token(&alice)))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&SendMessageRequest {
                        actor_id: alice.id.clone(),
                        conversation_id: conversation.id.clone(),
                        idempotency_key: "m-1".into(),
                        content: "hello".into(),
                        content_type: "text".into(),
                        metadata: json!({}),
                        trace_id: None,
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed["content"], "hello");
}

#[tokio::test]
async fn attachments_upload_download_and_enforce_workspace_boundaries() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let alice = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-acme".into(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    let bob = app
        .create_agent(CreateAgentRequest {
            actor_id: alice.id.clone(),
            name: "Bob agent".into(),
            scopes: vec!["messages:read".into(), "messages:write".into()],
            workspace_id: Some("ws-acme".into()),
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let mallory = choruz_domain::Principal {
        id: choruz_common::new_id(),
        workspace_id: "ws-labs".into(),
        principal_type: PrincipalType::Agent,
        name: "Mallory agent".into(),
        avatar_url: None,
        scopes: vec!["messages:read".into(), "messages:write".into()],
        secret_hash: Some(choruz_auth::hash_secret("mallory-agent-secret")),
        disabled: false,
        deleted_at: None,
        channel_visibility: choruz_domain::ChannelVisibility::Visible,
        created_at: choruz_common::now(),
        updated_at: choruz_common::now(),
        user_id: None,
    };
    seed_principal_to_db(&database.database_url, &alice).await;
    seed_principal_to_db(&database.database_url, &bob).await;
    seed_principal_to_db(&database.database_url, &mallory).await;
    let attachment_root =
        std::env::temp_dir().join(format!("choruz-attachment-{}", choruz_common::new_id()));
    let router = router_with_runtime(
        app.clone(),
        &attachment_root,
        LocalAuthConfig::from_env(),
        RuntimeStore::new(&database.database_url),
        PgSessionStore::new(&database.database_url),
        choruz_store::EventStore::new(&database.database_url),
    );

    let upload = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/attachments")
                .header("authorization", format!("Bearer {}", session_token(&alice)))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&UploadAttachmentRequest {
                        actor_id: alice.id.clone(),
                        filename: "brief.txt".into(),
                        content_type: "text/plain".into(),
                        data_base64: "YXR0YWNobWVudCBib2R5".into(),
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(upload.status(), StatusCode::CREATED);
    let upload_body = to_bytes(upload.into_body(), usize::MAX).await.unwrap();
    let attachment: AttachmentRecord = serde_json::from_slice(&upload_body).unwrap();
    assert_eq!(attachment.workspace_id, "ws-acme");
    assert_eq!(
        attachment.download_path,
        format!("/v1/attachments/{}", attachment.id)
    );

    // macOS reports Python files as `text/x-python-script`, which is not a
    // safe browser-rendering MIME. Source files are accepted but normalized to
    // text/plain before storage and download.
    let python_upload = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/attachments")
                .header("authorization", format!("Bearer {}", session_token(&alice)))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&UploadAttachmentRequest {
                        actor_id: alice.id.clone(),
                        filename: "script.py".into(),
                        content_type: "text/x-python-script".into(),
                        data_base64: "cHJpbnQoJ2hlbGxvJykK".into(),
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(python_upload.status(), StatusCode::CREATED);
    let python_attachment: AttachmentRecord = serde_json::from_slice(
        &to_bytes(python_upload.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(python_attachment.content_type, "text/plain");

    let download = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/attachments/{}?actor_id={}",
                    attachment.id, alice.id
                ))
                .header("authorization", format!("Bearer {}", session_token(&alice)))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(download.status(), StatusCode::OK);
    assert_eq!(download.headers()[CONTENT_TYPE], "text/plain");
    let bytes = to_bytes(download.into_body(), usize::MAX).await.unwrap();
    assert_eq!(bytes.as_ref(), b"attachment body");

    let denied = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/attachments/{}?actor_id={}",
                    attachment.id, mallory.id
                ))
                .header(
                    "authorization",
                    format!("Bearer {}", session_token(&mallory)),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);

    let same_workspace_denied = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/attachments/{}?actor_id={}",
                    attachment.id, bob.id
                ))
                .header("authorization", format!("Bearer {}", session_token(&bob)))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(same_workspace_denied.status(), StatusCode::FORBIDDEN);

    let shared_group = app
        .create_group(CreateGroupRequest {
            actor_id: alice.id.clone(),
            name: "Shared Attachment Group".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![],
            workspace_id: Some("ws-acme".into()),
        })
        .unwrap();
    seed_conversation_to_db(&database.database_url, &shared_group).await;
    {
        let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
            .await
            .expect("connect for attachment membership seed");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        client
            .execute(
                "INSERT INTO conversation_member (conv_id, principal_id, joined_at)
                 VALUES ($1, $2, NOW())",
                &[&shared_group.id, &mallory.id],
            )
            .await
            .expect("seed cross-workspace attachment member");
    }

    let shared_msg = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/messages")
                .header("authorization", format!("Bearer {}", session_token(&alice)))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&SendMessageRequest {
                        actor_id: alice.id.clone(),
                        conversation_id: shared_group.id.clone(),
                        idempotency_key: "attachment-share-msg".into(),
                        content: "Attachment: brief.txt".into(),
                        content_type: "attachment".into(),
                        metadata: json!({
                            "attachment_id": attachment.id,
                            "filename": "brief.txt",
                            "mime_type": "text/plain",
                            "size_bytes": 15,
                            "download_path": format!("/v1/attachments/{}", attachment.id),
                        }),
                        trace_id: None,
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(shared_msg.status(), StatusCode::CREATED);

    let cross_workspace_allowed = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/attachments/{}?actor_id={}",
                    attachment.id, mallory.id
                ))
                .header(
                    "authorization",
                    format!("Bearer {}", session_token(&mallory)),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cross_workspace_allowed.status(), StatusCode::OK);

    let referenced_delete = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(format!(
                    "/v1/attachments/{}?actor_id={}",
                    attachment.id, alice.id
                ))
                .header("authorization", format!("Bearer {}", session_token(&alice)))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(referenced_delete.status(), StatusCode::CONFLICT);

    let orphan_upload = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/attachments")
                .header("authorization", format!("Bearer {}", session_token(&alice)))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&UploadAttachmentRequest {
                        actor_id: alice.id.clone(),
                        filename: "cleanup.txt".into(),
                        content_type: "text/plain".into(),
                        data_base64: "Y2xlYW51cCBib2R5".into(),
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(orphan_upload.status(), StatusCode::CREATED);
    let orphan_body = to_bytes(orphan_upload.into_body(), usize::MAX)
        .await
        .unwrap();
    let orphan_attachment: AttachmentRecord = serde_json::from_slice(&orphan_body).unwrap();

    let deleted = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(format!(
                    "/v1/attachments/{}?actor_id={}",
                    orphan_attachment.id, alice.id
                ))
                .header("authorization", format!("Bearer {}", session_token(&alice)))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);

    let missing = router
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/attachments/{}?actor_id={}",
                    orphan_attachment.id, alice.id
                ))
                .header("authorization", format!("Bearer {}", session_token(&alice)))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    tokio::fs::remove_dir_all(&attachment_root).await.unwrap();
}

#[tokio::test]
async fn attachment_upload_cleans_up_bytes_when_metadata_insert_fails() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let alice = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-acme".into(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &alice).await;

    let attachment_root = std::env::temp_dir().join(format!(
        "choruz-attachment-cleanup-{}",
        choruz_common::new_id()
    ));
    let router = router_with_runtime(
        app.clone(),
        &attachment_root,
        LocalAuthConfig::from_env(),
        RuntimeStore::new(&database.database_url),
        PgSessionStore::new(&database.database_url),
        choruz_store::EventStore::new(&database.database_url),
    );

    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect for attachment table drop");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "ALTER TABLE attachment ADD COLUMN force_metadata_insert_failure TEXT NOT NULL",
            &[],
        )
        .await
        .expect("force attachment metadata inserts to fail");

    let upload = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/attachments")
                .header("authorization", format!("Bearer {}", session_token(&alice)))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&UploadAttachmentRequest {
                        actor_id: alice.id.clone(),
                        filename: "cleanup-fail.txt".into(),
                        content_type: "text/plain".into(),
                        data_base64: "Y2xlYW51cCBmYWls".into(),
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(upload.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let dir_exists = tokio::fs::try_exists(&attachment_root).await.unwrap();
    if dir_exists {
        let mut entries = tokio::fs::read_dir(&attachment_root).await.unwrap();
        assert!(
            entries.next_entry().await.unwrap().is_none(),
            "attachment bytes should be cleaned up when metadata insert fails"
        );
        tokio::fs::remove_dir_all(&attachment_root).await.unwrap();
    }
}

#[tokio::test]
async fn attachment_message_send_rejects_missing_attachment_reference() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let alice = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-acme".into(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    let conversation = app
        .create_group(CreateGroupRequest {
            actor_id: alice.id.clone(),
            name: "Attachment Guard".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![],
            workspace_id: Some("ws-acme".into()),
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &alice).await;
    seed_conversation_to_db(&database.database_url, &conversation).await;

    let router = router_with_db(app, &database.database_url);
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/messages")
                .header("authorization", format!("Bearer {}", session_token(&alice)))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&SendMessageRequest {
                        actor_id: alice.id.clone(),
                        conversation_id: conversation.id.clone(),
                        idempotency_key: "missing-attachment-msg".into(),
                        content: "Attachment: missing.txt".into(),
                        content_type: "attachment".into(),
                        metadata: json!({
                            "attachment_id": "att-missing",
                            "filename": "missing.txt",
                            "mime_type": "text/plain",
                        }),
                        trace_id: None,
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn attachment_message_send_allows_the_single_human_to_reuse_attachment() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let alice = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-acme".into(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    let bob = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-acme".into(),
            principal_type: PrincipalType::Human,
            name: "Bob".into(),
            avatar_url: None,
        })
        .unwrap();
    let conversation = app
        .create_group(CreateGroupRequest {
            actor_id: alice.id.clone(),
            name: "Unauthorized Attachment Guard".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![bob.id.clone()],
            workspace_id: Some("ws-acme".into()),
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &alice).await;
    seed_principal_to_db(&database.database_url, &bob).await;
    seed_conversation_to_db(&database.database_url, &conversation).await;

    let attachment_root = std::env::temp_dir().join(format!(
        "choruz-attachment-authz-{}",
        choruz_common::new_id()
    ));
    let router = router_with_runtime(
        app.clone(),
        &attachment_root,
        LocalAuthConfig::from_env(),
        RuntimeStore::new(&database.database_url),
        PgSessionStore::new(&database.database_url),
        choruz_store::EventStore::new(&database.database_url),
    );

    let upload = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/attachments")
                .header("authorization", format!("Bearer {}", session_token(&alice)))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&UploadAttachmentRequest {
                        actor_id: alice.id.clone(),
                        filename: "private.txt".into(),
                        content_type: "text/plain".into(),
                        data_base64: "cHJpdmF0ZSBib2R5".into(),
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(upload.status(), StatusCode::CREATED);
    let upload_body = to_bytes(upload.into_body(), usize::MAX).await.unwrap();
    let attachment: AttachmentRecord = serde_json::from_slice(&upload_body).unwrap();

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/messages")
                .header("authorization", format!("Bearer {}", session_token(&bob)))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&SendMessageRequest {
                        actor_id: bob.id.clone(),
                        conversation_id: conversation.id.clone(),
                        idempotency_key: "unauthorized-attachment-msg".into(),
                        content: "Attachment: private.txt".into(),
                        content_type: "attachment".into(),
                        metadata: json!({
                            "attachment_id": attachment.id,
                            "filename": "private.txt",
                            "mime_type": "text/plain",
                            "download_path": format!("/v1/attachments/{}", attachment.id),
                        }),
                        trace_id: None,
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    tokio::fs::remove_dir_all(&attachment_root).await.unwrap();
}

#[tokio::test]
async fn search_messages_isolation() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let auth = LocalAuthConfig::from_env();
    let operator = auth.ensure_operator_sync(&app).unwrap();

    let alice = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: operator.workspace_id.clone(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();

    let bob = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: operator.workspace_id.clone(),
            principal_type: PrincipalType::Human,
            name: "Bob".into(),
            avatar_url: None,
        })
        .unwrap();

    let group_a = app
        .create_group(CreateGroupRequest {
            actor_id: operator.id.clone(),
            name: "Group A".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![alice.id.clone()],
            workspace_id: None,
        })
        .unwrap();

    let group_b = app
        .create_group(CreateGroupRequest {
            actor_id: operator.id.clone(),
            name: "Group B".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![bob.id.clone()],
            workspace_id: None,
        })
        .unwrap();

    seed_principal_to_db(&database.database_url, &operator).await;
    seed_principal_to_db(&database.database_url, &alice).await;
    seed_principal_to_db(&database.database_url, &bob).await;
    seed_conversation_to_db(&database.database_url, &group_a).await;
    seed_conversation_to_db(&database.database_url, &group_b).await;

    let router = router_with_db(app.clone(), &database.database_url);

    // Alice sends a message to Group A
    let msg_alice = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/messages")
                .header("authorization", format!("Bearer {}", session_token(&alice)))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&SendMessageRequest {
                        actor_id: alice.id.clone(),
                        conversation_id: group_a.id.clone(),
                        idempotency_key: "msg-alice".into(),
                        content: "secret word from alice".into(),
                        content_type: "text".into(),
                        metadata: json!({}),
                        trace_id: None,
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(msg_alice.status(), StatusCode::CREATED);

    // Bob sends a message to Group B
    let msg_bob = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/messages")
                .header("authorization", format!("Bearer {}", session_token(&bob)))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&SendMessageRequest {
                        actor_id: bob.id.clone(),
                        conversation_id: group_b.id.clone(),
                        idempotency_key: "msg-bob".into(),
                        content: "secret word from bob".into(),
                        content_type: "text".into(),
                        metadata: json!({}),
                        trace_id: None,
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(msg_bob.status(), StatusCode::CREATED);

    // Alice searches for "secret"
    let alice_search = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/v1/messages/search?principal_id={}&q=secret",
                    alice.id
                ))
                .header("authorization", format!("Bearer {}", session_token(&alice)))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(alice_search.status(), StatusCode::OK);
    let body = to_bytes(alice_search.into_body(), usize::MAX)
        .await
        .unwrap();
    let results: Value = serde_json::from_slice(&body).unwrap();

    // Alice should only see HER message.
    // CURRENTLY (BUGGY): Alice sees BOTH.
    // EXPECTED: Alice sees 1.
    assert_eq!(
        results.as_array().unwrap().len(),
        1,
        "Alice should only see her own messages in Group A. Results: {:?}",
        results
    );
    assert_eq!(results[0]["content"], "secret word from alice");

    // Operator searches for "secret"
    let admin_search = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/v1/messages/search?principal_id={}&q=secret",
                    operator.id
                ))
                .header(
                    "authorization",
                    format!("Bearer {}", session_token(&operator)),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(admin_search.status(), StatusCode::OK);
    let body = to_bytes(admin_search.into_body(), usize::MAX)
        .await
        .unwrap();
    let results: Value = serde_json::from_slice(&body).unwrap();

    // Operator should see BOTH.
    assert_eq!(
        results.as_array().unwrap().len(),
        2,
        "Operator should see all messages. Results: {:?}",
        results
    );
}

#[tokio::test]
async fn get_message_fetches_quote_targets_with_uniform_auth() {
    // Quote-reply preview endpoint: members fetch any single message in
    // the conversation; outsiders are forbidden; nonexistent ids and ids
    // from OTHER conversations are a uniform 404 (no cross-conversation
    // existence oracle — same contract as the thread read paths).
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let router = router_with_db(app.clone(), &database.database_url);

    let alice = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-get-message".into(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    let outsider = app
        .create_agent(CreateAgentRequest {
            actor_id: alice.id.clone(),
            name: "Outsider agent".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    seed_principal_to_db(&database.database_url, &alice).await;
    seed_principal_to_db(&database.database_url, &outsider).await;

    let mk_group = |name: &str| {
        json!({
            "actor_id": alice.id.clone(),
            "name": name,
            "description": null,
            "avatar_url": null,
            "member_ids": [],
        })
    };
    let (_, group_a) = api_json_payload_request(
        router.clone(),
        &alice,
        Method::POST,
        "/v1/groups".into(),
        mk_group("get-message-a"),
    )
    .await;
    let (_, group_b) = api_json_payload_request(
        router.clone(),
        &alice,
        Method::POST,
        "/v1/groups".into(),
        mk_group("get-message-b"),
    )
    .await;
    let conv_a = group_a["id"].as_str().unwrap();
    let conv_b = group_b["id"].as_str().unwrap();

    let (_, msg) = api_send_with_metadata(
        router.clone(),
        &alice,
        conv_a,
        "the original message",
        "gm-1",
        json!({}),
    )
    .await;
    let msg_id = msg["id"].as_str().unwrap();

    // Member fetch → 200 with the full message body.
    let (status, fetched) = api_json_request(
        router.clone(),
        &alice,
        Method::GET,
        format!("/v1/conversations/{conv_a}/messages/{msg_id}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "member fetch: {fetched}");
    assert_eq!(fetched["id"], msg_id);
    assert_eq!(fetched["content"], "the original message");
    assert_eq!(fetched["sender_id"], alice.id.as_str());

    // Outsider (same workspace, not a member) → 403.
    let (outsider_status, _) = api_json_request(
        router.clone(),
        &outsider,
        Method::GET,
        format!("/v1/conversations/{conv_a}/messages/{msg_id}"),
    )
    .await;
    assert_eq!(outsider_status, StatusCode::FORBIDDEN);

    // Same id through ANOTHER conversation → uniform 404.
    let (cross_status, _) = api_json_request(
        router.clone(),
        &alice,
        Method::GET,
        format!("/v1/conversations/{conv_b}/messages/{msg_id}"),
    )
    .await;
    assert_eq!(
        cross_status,
        StatusCode::NOT_FOUND,
        "cross-conversation id must be indistinguishable from missing"
    );

    // Nonexistent id → 404.
    let (missing_status, _) = api_json_request(
        router.clone(),
        &alice,
        Method::GET,
        format!("/v1/conversations/{conv_a}/messages/no-such-message"),
    )
    .await;
    assert_eq!(missing_status, StatusCode::NOT_FOUND);
}
