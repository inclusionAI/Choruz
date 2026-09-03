use super::*;

#[tokio::test]
async fn bootstrap_is_bounded_stably_paginated_and_uses_activity_preview() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let router = router_with_db(app.clone(), &database.database_url);
    let alice = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-bootstrap".into(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &alice).await;

    let mut conversations = Vec::new();
    for index in 0..4 {
        let conversation = app
            .create_group(CreateGroupRequest {
                actor_id: alice.id.clone(),
                name: format!("Bootstrap {index}"),
                description: None,
                avatar_url: None,
                member_ids: vec![],
                workspace_id: Some(alice.workspace_id.clone()),
            })
            .unwrap();
        seed_conversation_to_db(&database.database_url, &conversation).await;
        assert_eq!(
            api_send_text_message(
                router.clone(),
                &alice,
                &conversation.id,
                &format!("bootstrap-message-{index}"),
                &format!("message {index}"),
            )
            .await,
            StatusCode::CREATED
        );
        conversations.push(conversation);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    assert_eq!(
        api_pin_conversation(router.clone(), &alice, &conversations[3].id).await,
        StatusCode::NO_CONTENT
    );

    let (first_status, first) = api_bootstrap(router.clone(), &alice, "limit=2").await;
    assert_eq!(first_status, StatusCode::OK);
    let first_items = first["conversations"]["items"].as_array().unwrap();
    assert_eq!(first_items.len(), 2);
    assert_eq!(first["conversations"]["has_more"], true);
    assert_eq!(
        first_items[0]["conversation"]["id"].as_str(),
        Some(conversations[3].id.as_str())
    );
    assert_eq!(first_items[0]["last_message"]["content"], "message 3");
    assert!(first_items[0]["pinned_at"].is_string());
    assert!(first["messages_by_conversation"].is_null());

    let cursor = first["conversations"]["next_cursor"]
        .as_str()
        .expect("first page cursor");
    let (second_status, second) =
        api_bootstrap(router.clone(), &alice, &format!("limit=2&after={cursor}")).await;
    assert_eq!(second_status, StatusCode::OK);
    let second_items = second["conversations"]["items"].as_array().unwrap();
    assert_eq!(second_items.len(), 2);
    assert_eq!(second["conversations"]["has_more"], false);

    let first_ids: std::collections::HashSet<_> = first_items
        .iter()
        .filter_map(|item| item["conversation"]["id"].as_str())
        .collect();
    let second_ids: std::collections::HashSet<_> = second_items
        .iter()
        .filter_map(|item| item["conversation"]["id"].as_str())
        .collect();
    assert!(first_ids.is_disjoint(&second_ids));
    assert_eq!(first_ids.len() + second_ids.len(), 4);

    let (invalid_status, _) = api_bootstrap(router, &alice, "after=not-a-cursor").await;
    assert_eq!(invalid_status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn bootstrap_caps_page_size_and_does_not_expose_foreign_conversations() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let router = router_with_db(app.clone(), &database.database_url);
    let alice = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-bootstrap-visible".into(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    let mallory = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-bootstrap-foreign".into(),
            principal_type: PrincipalType::Human,
            name: "Mallory".into(),
            avatar_url: None,
        })
        .unwrap();
    for principal in [&alice, &mallory] {
        seed_principal_to_db(&database.database_url, principal).await;
    }

    for index in 0..105 {
        let conversation = app
            .create_group(CreateGroupRequest {
                actor_id: alice.id.clone(),
                name: format!("Visible {index}"),
                description: None,
                avatar_url: None,
                member_ids: vec![],
                workspace_id: Some(alice.workspace_id.clone()),
            })
            .unwrap();
        seed_conversation_to_db(&database.database_url, &conversation).await;
    }
    let foreign = app
        .create_group(CreateGroupRequest {
            actor_id: mallory.id.clone(),
            name: "Foreign".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![],
            workspace_id: Some(mallory.workspace_id.clone()),
        })
        .unwrap();
    seed_conversation_to_db(&database.database_url, &foreign).await;

    let (status, body) = api_bootstrap(router, &alice, "limit=10000").await;
    assert_eq!(status, StatusCode::OK);
    let items = body["conversations"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 100, "server must cap bootstrap pages");
    assert_eq!(body["conversations"]["has_more"], true);
    assert!(
        items
            .iter()
            .all(|item| { item["conversation"]["id"].as_str() != Some(foreign.id.as_str()) })
    );
}

#[tokio::test]
async fn sync_feed_is_gapless_private_and_independent_from_outbox_ack() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let router = router_with_db(app.clone(), &database.database_url);
    let alice = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-sync".into(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    let bob = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-sync".into(),
            principal_type: PrincipalType::Human,
            name: "Bob".into(),
            avatar_url: None,
        })
        .unwrap();
    let mallory = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-foreign".into(),
            principal_type: PrincipalType::Human,
            name: "Mallory".into(),
            avatar_url: None,
        })
        .unwrap();
    for principal in [&alice, &bob, &mallory] {
        seed_principal_to_db(&database.database_url, principal).await;
    }
    let conversation = app
        .create_group(CreateGroupRequest {
            actor_id: alice.id.clone(),
            name: "Sync group".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![bob.id.clone()],
            workspace_id: Some(alice.workspace_id.clone()),
        })
        .unwrap();
    seed_conversation_to_db(&database.database_url, &conversation).await;

    let (_, alice_bootstrap) = api_bootstrap(router.clone(), &alice, "").await;
    let (_, bob_bootstrap) = api_bootstrap(router.clone(), &bob, "").await;
    let (_, mallory_bootstrap) = api_bootstrap(router.clone(), &mallory, "").await;
    let alice_start = alice_bootstrap["sync_cursor"].as_u64().unwrap();
    let bob_start = bob_bootstrap["sync_cursor"].as_u64().unwrap();
    let mallory_start = mallory_bootstrap["sync_cursor"].as_u64().unwrap();

    for index in 0..5 {
        assert_eq!(
            api_send_text_message(
                router.clone(),
                &alice,
                &conversation.id,
                &format!("sync-message-{index}"),
                &format!("message {index}"),
            )
            .await,
            StatusCode::CREATED
        );
    }
    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect for reply sync regression");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "INSERT INTO conversation_events
               (conversation_id, seq, event_id, event_type, sender_id, content,
                content_type, metadata, turn_id, reply_event_id)
             VALUES ($1, 6, 'reply-sync-regression', 'reply', $2,
                     'agent reply reaches sync', 'text', '{}',
                     'reply-sync-turn', 'reply-sync-regression')",
            &[&conversation.id, &bob.id],
        )
        .await
        .expect("insert pipeline-style reply event");
    assert_eq!(
        api_pin_conversation(router.clone(), &alice, &conversation.id).await,
        StatusCode::NO_CONTENT
    );
    let (update_status, _) = api_json_payload_request(
        router.clone(),
        &alice,
        Method::PATCH,
        format!("/v1/groups/{}", conversation.id),
        json!({
            "actor_id": alice.id,
            "name": "Renamed sync group",
            "description": "one durable metadata mutation"
        }),
    )
    .await;
    assert_eq!(update_status, StatusCode::OK);

    let mut cursor = alice_start;
    let mut alice_changes = Vec::new();
    loop {
        let (status, page) =
            api_sync(router.clone(), &alice, &format!("cursor={cursor}&limit=2")).await;
        assert_eq!(status, StatusCode::OK);
        let changes = page["changes"].as_array().unwrap();
        alice_changes.extend(changes.iter().cloned());
        let next = page["next_cursor"].as_u64().unwrap();
        assert!(next >= cursor);
        cursor = next;
        if !page["has_more"].as_bool().unwrap() {
            break;
        }
    }
    assert_eq!(
        alice_changes
            .iter()
            .filter(|change| change["event_type"] == "message.created")
            .count(),
        6
    );
    assert!(alice_changes.iter().any(|change| {
        change["payload"]["event_id"] == "reply-sync-regression"
            && change["payload"]["content"] == "agent reply reaches sync"
            && change["payload"]["sender_id"] == bob.id
    }));
    assert!(
        alice_changes
            .iter()
            .any(|change| change["event_type"] == "conversation.pin_set")
    );
    assert!(
        alice_changes
            .iter()
            .any(|change| change["event_type"] == "conversation.read_state_changed")
    );
    assert!(
        alice_changes
            .iter()
            .any(|change| change["event_type"] == "conversation.updated")
    );
    assert!(alice_changes.windows(2).all(|window| {
        window[0]["cursor"].as_u64().unwrap() < window[1]["cursor"].as_u64().unwrap()
    }));

    let (bob_status, bob_page) = api_sync(
        router.clone(),
        &bob,
        &format!("cursor={bob_start}&limit=100"),
    )
    .await;
    assert_eq!(bob_status, StatusCode::OK);
    assert_eq!(
        bob_page["changes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|change| change["event_type"] == "message.created")
            .count(),
        6
    );
    assert!(
        !bob_page["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|change| change["event_type"] == "conversation.pin_set")
    );
    assert!(
        bob_page["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|change| change["event_type"] == "conversation.updated")
    );

    let (foreign_status, foreign_page) =
        api_sync(router.clone(), &mallory, &format!("cursor={mallory_start}")).await;
    assert_eq!(foreign_status, StatusCode::OK);
    assert!(foreign_page["changes"].as_array().unwrap().is_empty());

    let (future_status, future_body) =
        api_sync(router.clone(), &mallory, "cursor=9223372036854775807").await;
    assert_eq!(future_status, StatusCode::BAD_REQUEST);
    assert!(validation_detail(&future_body).contains("ahead"));

    let (events_status, events) = api_json_request(
        router.clone(),
        &alice,
        Method::GET,
        format!("/v1/principals/{}/events", alice.id),
    )
    .await;
    assert_eq!(events_status, StatusCode::OK);
    let last_delivery_seq = events.as_array().unwrap().last().unwrap()["delivery_seq"]
        .as_u64()
        .unwrap();
    let (ack_status, _) = api_json_payload_request(
        router.clone(),
        &alice,
        Method::POST,
        format!("/v1/principals/{}/events/ack", alice.id),
        json!({"upto_delivery_seq": last_delivery_seq}),
    )
    .await;
    assert_eq!(ack_status, StatusCode::OK);

    let (_, after_ack) = api_sync(router, &alice, &format!("cursor={alice_start}&limit=100")).await;
    assert_eq!(
        after_ack["changes"].as_array().unwrap().len(),
        alice_changes.len()
    );
}

#[tokio::test]
async fn sync_websocket_requires_auth_replays_unacked_and_isolates_device_acks() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let alice = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-sync-socket".into(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &alice).await;
    let conversation = app
        .create_group(CreateGroupRequest {
            actor_id: alice.id.clone(),
            name: "Socket sync".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![],
            workspace_id: Some(alice.workspace_id.clone()),
        })
        .unwrap();
    seed_conversation_to_db(&database.database_url, &conversation).await;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let test_router = router_with_db(app.clone(), &database.database_url);
    let http_router = test_router.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, test_router).await.unwrap();
    });

    let unauthenticated = connect_async(format!(
        "ws://{address}/v1/ws/sync?device_id=browser-a&cursor=0"
    ))
    .await
    .expect_err("sync socket must require authentication");
    match unauthenticated {
        tokio_tungstenite::tungstenite::Error::Http(response) => {
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        error => panic!("expected HTTP auth response, got {error}"),
    }

    let (_, bootstrap) = api_bootstrap(http_router.clone(), &alice, "").await;
    let start = bootstrap["sync_cursor"].as_u64().unwrap();
    let mut first = connect_sync_socket(address, &alice, "browser-a", start).await;
    let ready = next_ws_json(&mut first).await;
    assert_eq!(ready["type"], "sync_ready");
    assert_eq!(ready["cursor"], start);

    assert_eq!(
        api_send_text_message(
            http_router.clone(),
            &alice,
            &conversation.id,
            "socket-live-message",
            "arrives without polling",
        )
        .await,
        StatusCode::CREATED
    );
    let live = next_ws_json(&mut first).await;
    assert_eq!(live["type"], "sync_changes");
    assert!(
        live["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|change| change["event_type"] == "message.created")
    );
    let delivered_cursor = live["next_cursor"].as_u64().unwrap();
    first.close(None).await.unwrap();

    // No ACK: reconnecting the same device must replay the delivered page.
    let mut replay = connect_sync_socket(address, &alice, "browser-a", start).await;
    assert_eq!(next_ws_json(&mut replay).await["type"], "sync_ready");
    let replayed = next_ws_json(&mut replay).await;
    let replay_cursor = replayed["next_cursor"].as_u64().unwrap();
    assert!(replay_cursor >= delivered_cursor);
    replay
        .send(Message::Text(
            json!({"type":"sync_ack","cursor":replay_cursor})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let acked = next_ws_json(&mut replay).await;
    assert_eq!(acked, json!({"type":"sync_acked","cursor":replay_cursor}));
    replay.close(None).await.unwrap();

    // Device A resumes after its ACK, while independent device B still sees
    // the same durable history from its own cursor.
    let mut resumed = connect_sync_socket(address, &alice, "browser-a", start).await;
    let resumed_ready = next_ws_json(&mut resumed).await;
    assert_eq!(resumed_ready["cursor"], replay_cursor);
    let mut second = connect_sync_socket(address, &alice, "browser-b", start).await;
    assert_eq!(next_ws_json(&mut second).await["cursor"], start);
    assert_eq!(
        next_ws_json(&mut second).await["next_cursor"],
        replay_cursor
    );

    // A client cannot ACK data that this connection was never sent.
    resumed
        .send(Message::Text(
            json!({"type":"sync_ack","cursor":replay_cursor + 1})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let rejected = next_ws_json(&mut resumed).await;
    assert_eq!(rejected["type"], "sync_error");
    assert!(rejected["detail"].as_str().unwrap().contains("unsent"));

    second.close(None).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn message_pages_cover_history_and_incremental_bursts_without_gaps() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let router = router_with_db(app.clone(), &database.database_url);
    let alice = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-message-pages".into(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    let mallory = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-message-pages-foreign".into(),
            principal_type: PrincipalType::Human,
            name: "Mallory".into(),
            avatar_url: None,
        })
        .unwrap();
    for principal in [&alice, &mallory] {
        seed_principal_to_db(&database.database_url, principal).await;
    }
    let conversation = app
        .create_group(CreateGroupRequest {
            actor_id: alice.id.clone(),
            name: "Paged history".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![],
            workspace_id: Some(alice.workspace_id.clone()),
        })
        .unwrap();
    seed_conversation_to_db(&database.database_url, &conversation).await;

    for index in 1..=12 {
        assert_eq!(
            api_send_text_message(
                router.clone(),
                &alice,
                &conversation.id,
                &format!("paged-{index}"),
                &format!("message {index}"),
            )
            .await,
            StatusCode::CREATED
        );
    }

    let page_sequences = |body: &Value| {
        body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| message["server_seq"].as_u64().unwrap())
            .collect::<Vec<_>>()
    };

    let (latest_status, latest) =
        api_message_page(router.clone(), &alice, &conversation.id, "limit=4").await;
    assert_eq!(latest_status, StatusCode::OK);
    assert_eq!(page_sequences(&latest), vec![9, 10, 11, 12]);
    assert_eq!(latest["direction"], "latest");
    assert_eq!(latest["has_more"], true);
    assert_eq!(latest["next_cursor"], 9);

    let (older_status, older) = api_message_page(
        router.clone(),
        &alice,
        &conversation.id,
        "limit=4&before_seq=9",
    )
    .await;
    assert_eq!(older_status, StatusCode::OK);
    assert_eq!(page_sequences(&older), vec![5, 6, 7, 8]);
    assert_eq!(older["next_cursor"], 5);

    let mut incremental = Vec::new();
    let mut cursor = 0;
    loop {
        let (_, page) = api_message_page(
            router.clone(),
            &alice,
            &conversation.id,
            &format!("limit=4&after_seq={cursor}"),
        )
        .await;
        let sequences = page_sequences(&page);
        cursor = *sequences.last().expect("incremental page is non-empty");
        incremental.extend(sequences);
        if !page["has_more"].as_bool().unwrap() {
            break;
        }
    }
    assert_eq!(incremental, (1..=12).collect::<Vec<_>>());

    for view in ["", "&view=timeline"] {
        let (status, body) = api_json_request(
            router.clone(),
            &alice,
            Method::GET,
            format!(
                "/v1/conversations/{}/messages?principal_id={}&since_seq=0&limit=4{view}",
                conversation.id, alice.id
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let messages = if view.is_empty() {
            body.as_array().unwrap()
        } else {
            body["messages"].as_array().unwrap()
        };
        assert_eq!(
            messages
                .iter()
                .map(|message| message["server_seq"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4],
            "legacy incremental reads must return the oldest unseen page"
        );
    }

    let (ambiguous_status, _) = api_message_page(
        router.clone(),
        &alice,
        &conversation.id,
        "before_seq=9&after_seq=1",
    )
    .await;
    assert_eq!(ambiguous_status, StatusCode::BAD_REQUEST);
    let (foreign_status, _) = api_message_page(router, &mallory, &conversation.id, "limit=4").await;
    assert_eq!(foreign_status, StatusCode::FORBIDDEN);
}
