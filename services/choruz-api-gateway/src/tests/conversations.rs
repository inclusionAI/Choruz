use super::*;

#[tokio::test]
async fn agent_created_conversation_registers_its_workspace_owner() {
    let database = TestDatabase::create().await;
    let db =
        choruz_application::DbService::new(choruz_store::EventStore::new(&database.database_url));
    let human = db
        .create_human_user("local-user", "password-123")
        .await
        .unwrap();
    let other_human = db
        .create_human_user("other-user", "password-456")
        .await
        .unwrap();
    let company = db
        .create_company(CreateCompanyRequest {
            actor_id: human.id.clone(),
            name: "Agent Team".into(),
            slug: Some("agent-team".into()),
            description: None,
            folder_path: None,
        })
        .await
        .unwrap();
    let agent = db
        .create_agent(CreateAgentRequest {
            actor_id: human.id.clone(),
            name: "Coordinator".into(),
            scopes: vec!["messages:read".into(), "messages:write".into()],
            workspace_id: Some(company.id.clone()),
            channel_visibility: None,
        })
        .await
        .unwrap()
        .principal;

    let conversation = db
        .create_group(CreateGroupRequest {
            actor_id: agent.id.clone(),
            name: "Agent-created group".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![],
            workspace_id: Some(company.id),
        })
        .await
        .unwrap();

    assert!(conversation.members.contains_key(&agent.id));
    assert!(conversation.members.contains_key(&human.id));
    assert!(!conversation.members.contains_key(&other_human.id));
    let error = db
        .remove_group_member(&conversation.id, &agent.id, &human.id)
        .await
        .expect_err("the workspace owner must remain a member");
    assert!(matches!(error, AppError::Forbidden(_)));
}

#[tokio::test]
async fn agent_created_group_uses_eligible_human_when_company_owner_is_inaccessible() {
    let database = TestDatabase::create().await;
    let db =
        choruz_application::DbService::new(choruz_store::EventStore::new(&database.database_url));
    let stale_owner = db
        .create_human_user("stale-owner", "password-123")
        .await
        .unwrap();
    let eligible_human = db
        .create_human_user("eligible-human", "password-456")
        .await
        .unwrap();
    let company = db
        .create_company(CreateCompanyRequest {
            actor_id: eligible_human.id.clone(),
            name: "Agent Team".into(),
            slug: Some("agent-team".into()),
            description: None,
            folder_path: None,
        })
        .await
        .unwrap();
    let agent = db
        .create_agent(CreateAgentRequest {
            actor_id: eligible_human.id.clone(),
            name: "Coordinator".into(),
            scopes: vec!["messages:read".into(), "messages:write".into()],
            workspace_id: Some(company.id.clone()),
            channel_visibility: None,
        })
        .await
        .unwrap()
        .principal;

    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "UPDATE company SET owner_id = $2 WHERE id = $1",
            &[&company.id, &stale_owner.id],
        )
        .await
        .unwrap();

    let group = db
        .create_group(CreateGroupRequest {
            actor_id: agent.id.clone(),
            name: "Agent-created group".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![],
            workspace_id: Some(company.id),
        })
        .await
        .unwrap();

    assert!(group.members.contains_key(&eligible_human.id));
    assert!(!group.members.contains_key(&stale_owner.id));
}

#[tokio::test]
async fn batch_disable_cannot_delete_a_nonmember_conversation() {
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
    let foreign_human = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-foreign".into(),
            principal_type: PrincipalType::Human,
            name: "Foreign Human".into(),
            avatar_url: None,
        })
        .unwrap();
    let foreign_conversation = app
        .create_group(CreateGroupRequest {
            actor_id: foreign_human.id.clone(),
            name: "Foreign Conversation".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![],
            workspace_id: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &operator).await;
    seed_principal_to_db(&database.database_url, &foreign_human).await;
    seed_conversation_to_db(&database.database_url, &foreign_conversation).await;
    let test_router = router_with_db(app, &database.database_url);

    let (status, _) = api_json_payload_request(
        test_router,
        &operator,
        Method::POST,
        "/v1/agents/batch-disable".into(),
        json!({
            "actor_id": operator.id,
            "agent_ids": [],
            "conversation_ids": [foreign_conversation.id]
        }),
    )
    .await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect to verify protected conversation");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let still_exists = client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM conversation WHERE id = $1)",
            &[&foreign_conversation.id],
        )
        .await
        .expect("verify protected conversation")
        .get::<_, bool>(0);
    assert!(still_exists);
}

#[tokio::test]
async fn conversation_pin_put_is_idempotent_and_preserves_pinned_at() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let router = router_with_db(app.clone(), &database.database_url);

    let alice = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-pins".into(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    let bob = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-pins".into(),
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

    seed_principal_to_db(&database.database_url, &alice).await;
    seed_principal_to_db(&database.database_url, &bob).await;
    seed_conversation_to_db(&database.database_url, &conversation).await;

    assert_eq!(
        api_pin_conversation(router.clone(), &alice, &conversation.id).await,
        StatusCode::NO_CONTENT
    );
    let first_pinned_at = db_pinned_at(&database.database_url, &alice.id, &conversation.id)
        .await
        .expect("pin row after first PUT");

    tokio::time::sleep(Duration::from_millis(20)).await;

    assert_eq!(
        api_pin_conversation(router.clone(), &alice, &conversation.id).await,
        StatusCode::NO_CONTENT
    );
    let second_pinned_at = db_pinned_at(&database.database_url, &alice.id, &conversation.id)
        .await
        .expect("pin row after repeated PUT");

    assert_eq!(first_pinned_at, second_pinned_at);
    assert_eq!(
        db_pin_count(&database.database_url, &alice.id, &conversation.id).await,
        1
    );

    let (snapshot_status, snapshot) = api_console_snapshot(router, &alice).await;
    assert_eq!(snapshot_status, StatusCode::OK);
    let pins = snapshot["pinned_conversations"].as_array().unwrap();
    assert_eq!(pins.len(), 1);
    assert_eq!(
        pins[0]["conversation_id"].as_str(),
        Some(conversation.id.as_str())
    );
}

#[tokio::test]
async fn conversation_pins_are_scoped_per_user() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let router = router_with_db(app.clone(), &database.database_url);

    let alice = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-pin-isolation".into(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    let bob = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-pin-isolation".into(),
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

    seed_principal_to_db(&database.database_url, &alice).await;
    seed_principal_to_db(&database.database_url, &bob).await;
    seed_conversation_to_db(&database.database_url, &conversation).await;

    assert_eq!(
        api_pin_conversation(router.clone(), &alice, &conversation.id).await,
        StatusCode::NO_CONTENT
    );

    let (alice_snapshot_status, alice_snapshot) =
        api_console_snapshot(router.clone(), &alice).await;
    assert_eq!(alice_snapshot_status, StatusCode::OK);
    assert!(
        alice_snapshot["pinned_conversations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|pin| pin["conversation_id"].as_str() == Some(conversation.id.as_str()))
    );

    let (bob_snapshot_status, bob_snapshot) = api_console_snapshot(router.clone(), &bob).await;
    assert_eq!(bob_snapshot_status, StatusCode::OK);
    assert!(
        bob_snapshot["pinned_conversations"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    assert_eq!(
        api_pin_conversation(router.clone(), &bob, &conversation.id).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        api_unpin_conversation(router.clone(), &alice, &conversation.id).await,
        StatusCode::NO_CONTENT
    );

    assert_eq!(
        db_pin_count(&database.database_url, &alice.id, &conversation.id).await,
        0
    );
    assert_eq!(
        db_pin_count(&database.database_url, &bob.id, &conversation.id).await,
        1
    );
}

#[tokio::test]
async fn conversation_archive_is_recoverable_user_scoped_and_removes_the_users_pin() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let router = router_with_db(app.clone(), &database.database_url);

    let alice = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-archive".into(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    let bob = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-archive".into(),
            principal_type: PrincipalType::Human,
            name: "Bob".into(),
            avatar_url: None,
        })
        .unwrap();
    let agent = app
        .create_agent(CreateAgentRequest {
            actor_id: alice.id.clone(),
            name: "Archive Bot".into(),
            scopes: vec!["messages:read".into(), "messages:write".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let mallory = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-elsewhere".into(),
            principal_type: PrincipalType::Human,
            name: "Mallory".into(),
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

    seed_principal_to_db(&database.database_url, &alice).await;
    seed_principal_to_db(&database.database_url, &bob).await;
    seed_principal_to_db(&database.database_url, &agent).await;
    seed_principal_to_db(&database.database_url, &mallory).await;
    seed_conversation_to_db(&database.database_url, &conversation).await;

    assert_eq!(
        api_archive_conversation(router.clone(), &agent, &conversation.id).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        api_archive_conversation(router.clone(), &mallory, &conversation.id).await,
        StatusCode::FORBIDDEN
    );

    assert_eq!(
        api_pin_conversation(router.clone(), &alice, &conversation.id).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        api_archive_conversation(router.clone(), &alice, &conversation.id).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        api_archive_conversation(router.clone(), &alice, &conversation.id).await,
        StatusCode::NO_CONTENT
    );

    let (alice_status, alice_snapshot) = api_console_snapshot(router.clone(), &alice).await;
    assert_eq!(alice_status, StatusCode::OK);
    assert_eq!(
        alice_snapshot["archived_conversations"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        alice_snapshot["pinned_conversations"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let (bob_status, bob_snapshot) = api_console_snapshot(router.clone(), &bob).await;
    assert_eq!(bob_status, StatusCode::OK);
    assert!(
        bob_snapshot["archived_conversations"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    assert_eq!(
        api_unarchive_conversation(router.clone(), &alice, &conversation.id).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        api_unarchive_conversation(router.clone(), &alice, &conversation.id).await,
        StatusCode::NO_CONTENT
    );
    let (restored_status, restored_snapshot) = api_console_snapshot(router, &alice).await;
    assert_eq!(restored_status, StatusCode::OK);
    assert!(
        restored_snapshot["archived_conversations"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        restored_snapshot["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"].as_str() == Some(conversation.id.as_str()))
    );
}

#[tokio::test]
async fn hide_is_user_scoped_and_removes_an_agent_session_from_normal_markers() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let router = router_with_db(app.clone(), &database.database_url);
    let alice = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-hide".into(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    let bob = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-hide".into(),
            principal_type: PrincipalType::Human,
            name: "Bob".into(),
            avatar_url: None,
        })
        .unwrap();
    let agent = app
        .create_agent(CreateAgentRequest {
            actor_id: alice.id.clone(),
            name: "Hidden Bot".into(),
            scopes: vec!["messages:read".into(), "messages:write".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let agent_direct = app
        .create_direct_conversation(CreateDirectConversationRequest {
            actor_id: alice.id.clone(),
            peer_principal_id: agent.id.clone(),
            workspace_id: None,
        })
        .unwrap();
    let human_direct = app
        .create_direct_conversation(CreateDirectConversationRequest {
            actor_id: alice.id.clone(),
            peer_principal_id: bob.id.clone(),
            workspace_id: None,
        })
        .unwrap();

    for principal in [&alice, &bob, &agent] {
        seed_principal_to_db(&database.database_url, principal).await;
    }
    for conversation in [&agent_direct, &human_direct] {
        seed_conversation_to_db(&database.database_url, conversation).await;
    }

    assert_eq!(
        api_pin_conversation(router.clone(), &alice, &agent_direct.id).await,
        StatusCode::NO_CONTENT,
    );
    assert_eq!(
        api_archive_conversation(router.clone(), &alice, &agent_direct.id).await,
        StatusCode::NO_CONTENT,
    );
    assert_eq!(
        api_hide_conversation(router.clone(), &alice, &human_direct.id).await,
        StatusCode::BAD_REQUEST,
    );
    assert_eq!(
        api_hide_conversation(router.clone(), &alice, &agent_direct.id).await,
        StatusCode::NO_CONTENT,
    );
    assert_eq!(
        api_hide_conversation(router.clone(), &alice, &agent_direct.id).await,
        StatusCode::NO_CONTENT,
    );

    let (alice_status, alice_snapshot) = api_console_snapshot(router.clone(), &alice).await;
    assert_eq!(alice_status, StatusCode::OK);
    assert_eq!(
        alice_snapshot["hidden_conversations"]
            .as_array()
            .unwrap()
            .len(),
        1,
    );
    assert!(
        alice_snapshot["pinned_conversations"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        alice_snapshot["archived_conversations"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        alice_snapshot["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|conversation| { conversation["id"].as_str() == Some(agent_direct.id.as_str()) })
    );
    let (bootstrap_status, bootstrap) = api_bootstrap(router.clone(), &alice, "limit=1").await;
    assert_eq!(bootstrap_status, StatusCode::OK);
    assert_eq!(
        bootstrap["hidden_conversations"].as_array().unwrap().len(),
        1,
        "the recovery UI must receive hidden sessions even when the conversation page is bounded",
    );

    assert_eq!(
        api_restore_hidden_conversation(router.clone(), &alice, &agent_direct.id).await,
        StatusCode::NO_CONTENT,
    );
    let (restored_status, restored_snapshot) = api_console_snapshot(router, &alice).await;
    assert_eq!(restored_status, StatusCode::OK);
    assert!(
        restored_snapshot["hidden_conversations"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn conversation_pin_put_denies_inaccessible_and_agent_principals() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let router = router_with_db(app.clone(), &database.database_url);

    let operator = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-pin-denials".into(),
            principal_type: PrincipalType::Human,
            name: "Operator".into(),
            avatar_url: None,
        })
        .unwrap();
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
    let mallory = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: operator.workspace_id.clone(),
            principal_type: PrincipalType::Human,
            name: "Mallory".into(),
            avatar_url: None,
        })
        .unwrap();
    let agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Pin Agent".into(),
            scopes: vec!["messages:read".into(), "messages:write".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let conversation = app
        .create_group(CreateGroupRequest {
            actor_id: operator.id.clone(),
            name: "Pin Denials".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![alice.id.clone(), bob.id.clone(), agent.id.clone()],
            workspace_id: None,
        })
        .unwrap();

    for principal in [&operator, &alice, &bob, &mallory, &agent] {
        seed_principal_to_db(&database.database_url, principal).await;
    }
    seed_conversation_to_db(&database.database_url, &conversation).await;

    assert_eq!(
        api_pin_conversation(router.clone(), &mallory, &conversation.id).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        api_pin_conversation(router, &agent, &conversation.id).await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn conversation_unpin_is_own_row_noop_and_humans_cannot_be_removed() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let router = router_with_db(app.clone(), &database.database_url);

    let operator = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-pin-delete".into(),
            principal_type: PrincipalType::Human,
            name: "Operator".into(),
            avatar_url: None,
        })
        .unwrap();
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
    let conversation = app
        .create_group(CreateGroupRequest {
            actor_id: operator.id.clone(),
            name: "Pin Delete".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![alice.id.clone(), bob.id.clone()],
            workspace_id: None,
        })
        .unwrap();

    seed_principal_to_db(&database.database_url, &operator).await;
    seed_principal_to_db(&database.database_url, &alice).await;
    seed_principal_to_db(&database.database_url, &bob).await;
    seed_conversation_to_db(&database.database_url, &conversation).await;

    assert_eq!(
        api_pin_conversation(router.clone(), &alice, &conversation.id).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        api_pin_conversation(router.clone(), &bob, &conversation.id).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        api_remove_group_member(router.clone(), &operator, &conversation.id, &bob.id).await,
        StatusCode::FORBIDDEN
    );

    let (bob_conversations_status, bob_conversations) =
        api_list_conversations(router.clone(), &bob).await;
    assert_eq!(bob_conversations_status, StatusCode::OK);
    assert!(
        bob_conversations
            .as_array()
            .unwrap()
            .iter()
            .any(|conversation_row| conversation_row["id"].as_str()
                == Some(conversation.id.as_str()))
    );

    assert_eq!(
        api_unpin_conversation(router.clone(), &bob, &conversation.id).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        api_unpin_conversation(router.clone(), &bob, &conversation.id).await,
        StatusCode::NO_CONTENT
    );

    assert_eq!(
        db_pin_count(&database.database_url, &bob.id, &conversation.id).await,
        0
    );
    assert_eq!(
        db_pin_count(&database.database_url, &alice.id, &conversation.id).await,
        1
    );
}

#[tokio::test]
async fn console_snapshot_omits_deleted_pinned_conversations() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let router = router_with_db(app.clone(), &database.database_url);

    let alice = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-deleted-pins".into(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    let bob = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-deleted-pins".into(),
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

    seed_principal_to_db(&database.database_url, &alice).await;
    seed_principal_to_db(&database.database_url, &bob).await;
    seed_conversation_to_db(&database.database_url, &conversation).await;

    assert_eq!(
        api_pin_conversation(router.clone(), &alice, &conversation.id).await,
        StatusCode::NO_CONTENT
    );

    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect for conversation delete");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "DELETE FROM conversation WHERE id = $1",
            &[&conversation.id],
        )
        .await
        .expect("delete conversation");

    let (snapshot_status, snapshot) = api_console_snapshot(router, &alice).await;
    assert_eq!(snapshot_status, StatusCode::OK);
    assert!(
        snapshot["pinned_conversations"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn company_multi_harness_accounts_is_off_until_a_member_turns_it_on() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let test_router = router_with_db(app.clone(), &database.database_url);

    let owner = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-owner".into(),
            principal_type: PrincipalType::Human,
            name: "Owner".into(),
            avatar_url: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &owner).await;

    let (created_status, created) = api_json_payload_request(
        test_router.clone(),
        &owner,
        Method::POST,
        "/v1/companies".into(),
        json!({
            "actor_id": owner.id.clone(),
            "name": "Single Login Company",
            "description": null,
        }),
    )
    .await;
    assert_eq!(created_status, StatusCode::CREATED);
    assert_eq!(created["multi_harness_accounts"], json!(false));
    let company_id = created["id"].as_str().unwrap();

    let (updated_status, updated) = api_json_payload_request(
        test_router.clone(),
        &owner,
        Method::PATCH,
        format!("/v1/companies/{company_id}"),
        json!({
            "actor_id": owner.id.clone(),
            "multi_harness_accounts": true,
        }),
    )
    .await;
    assert_eq!(updated_status, StatusCode::OK);
    assert_eq!(updated["multi_harness_accounts"], json!(true));

    let (listed_status, listed) =
        api_json_request(test_router, &owner, Method::GET, "/v1/companies".into()).await;
    assert_eq!(listed_status, StatusCode::OK);
    let listed_company = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|company| company["id"] == json!(company_id))
        .unwrap();
    assert_eq!(listed_company["multi_harness_accounts"], json!(true));
}

#[tokio::test]
async fn company_workspace_authorization_guards_hold() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let test_router = router_with_db(app.clone(), &database.database_url);

    let alice = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-alice".into(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    let bob = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-bob".into(),
            principal_type: PrincipalType::Human,
            name: "Bob".into(),
            avatar_url: None,
        })
        .unwrap();
    let operator = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-operator".into(),
            principal_type: PrincipalType::Human,
            name: "Operator".into(),
            avatar_url: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &alice).await;
    seed_principal_to_db(&database.database_url, &bob).await;
    seed_principal_to_db(&database.database_url, &operator).await;

    let (alice_company_status, alice_company) = api_json_payload_request(
        test_router.clone(),
        &alice,
        Method::POST,
        "/v1/companies".into(),
        json!({
            "actor_id": alice.id.clone(),
            "name": "Alice Company",
            "description": null,
        }),
    )
    .await;
    assert_eq!(alice_company_status, StatusCode::CREATED);
    let alice_company_id = alice_company["id"].as_str().unwrap();

    let (bob_company_status, bob_company) = api_json_payload_request(
        test_router.clone(),
        &bob,
        Method::POST,
        "/v1/companies".into(),
        json!({
            "actor_id": bob.id.clone(),
            "name": "Bob Company",
            "description": null,
        }),
    )
    .await;
    assert_eq!(bob_company_status, StatusCode::CREATED);
    let bob_company_id = bob_company["id"].as_str().unwrap();

    let (alice_reads_bob_company, _) = api_json_request(
        test_router.clone(),
        &alice,
        Method::GET,
        format!("/v1/companies/{bob_company_id}"),
    )
    .await;
    assert_eq!(alice_reads_bob_company, StatusCode::FORBIDDEN);

    let (alice_reads_bob_members, _) = api_json_request(
        test_router.clone(),
        &alice,
        Method::GET,
        format!("/v1/companies/{bob_company_id}/members"),
    )
    .await;
    assert_eq!(alice_reads_bob_members, StatusCode::FORBIDDEN);

    let stale_personal_conversation_id = "stale-personal-workspace-conv";
    let mut stale_personal_members = std::collections::BTreeMap::new();
    stale_personal_members.insert(
        alice.id.clone(),
        choruz_domain::ConversationMember {
            principal_id: alice.id.clone(),
            joined_at: choruz_common::now(),
        },
    );
    seed_conversation_to_db(
        &database.database_url,
        &choruz_domain::Conversation {
            id: stale_personal_conversation_id.into(),
            workspace_id: bob.workspace_id.clone(),
            conversation_type: choruz_domain::ConversationType::Group,
            name: Some("stale personal workspace".into()),
            description: None,
            avatar_url: None,
            creator_id: bob.id.clone(),
            created_at: choruz_common::now(),
            updated_at: choruz_common::now(),
            members: stale_personal_members,
        },
    )
    .await;
    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect for stale personal event seed");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "INSERT INTO conversation_events
                (conversation_id, seq, event_id, event_type, sender_id, content,
                 content_type, metadata, client_msg_id, created_at)
             VALUES ($1, 1, 'stale-personal-event', 'message', $2,
                     'stale personal content', 'text/plain', '{}'::jsonb,
                     'stale-personal-client', NOW())",
            &[&stale_personal_conversation_id, &bob.id],
        )
        .await
        .expect("seed stale personal event");

    let (stale_personal_list_status, stale_personal_list) =
        api_list_conversations(test_router.clone(), &alice).await;
    assert_eq!(stale_personal_list_status, StatusCode::OK);
    assert!(
        stale_personal_list
            .as_array()
            .unwrap()
            .iter()
            .all(|conversation| conversation["id"] != stale_personal_conversation_id)
    );

    let (stale_personal_messages_status, _) =
        api_list_messages(test_router.clone(), &alice, stale_personal_conversation_id).await;
    assert_eq!(stale_personal_messages_status, StatusCode::FORBIDDEN);

    let (stale_personal_search_status, stale_personal_search) =
        api_search_messages(test_router.clone(), &alice, "stale%20personal", None).await;
    assert_eq!(stale_personal_search_status, StatusCode::OK);
    assert!(stale_personal_search.as_array().unwrap().is_empty());

    let (stale_personal_ingest_status, _) = api_json_payload_request(
        test_router.clone(),
        &alice,
        Method::POST,
        "/v2/ingest".into(),
        json!({
            "conversation_id": stale_personal_conversation_id,
            "content": "stale personal ingest should fail",
            "content_type": "text/plain",
            "client_msg_id": "stale-personal-ingest",
            "metadata": {},
        }),
    )
    .await;
    assert_eq!(stale_personal_ingest_status, StatusCode::FORBIDDEN);

    let (stale_personal_export_status, _) = api_json_request(
        test_router.clone(),
        &alice,
        Method::GET,
        format!(
            "/v1/export/conversations/{stale_personal_conversation_id}?actor_id={}",
            alice.id
        ),
    )
    .await;
    assert_eq!(stale_personal_export_status, StatusCode::FORBIDDEN);

    let company_test_agent = |id: &str, name: &str| choruz_domain::Principal {
        id: id.into(),
        workspace_id: alice_company_id.to_string(),
        principal_type: PrincipalType::Agent,
        name: name.into(),
        avatar_url: None,
        scopes: vec!["messages:read".into(), "messages:write".into()],
        secret_hash: None,
        disabled: false,
        deleted_at: None,
        channel_visibility: choruz_domain::ChannelVisibility::Visible,
        created_at: choruz_common::now(),
        updated_at: choruz_common::now(),
        user_id: None,
    };
    let company_member = company_test_agent("company-member-agent", "Company Member Agent");
    let company_stale_owner = company_test_agent("company-stale-agent", "Company Stale Agent");
    let company_remove_target = company_test_agent("company-remove-agent", "Company Remove Agent");
    let company_agent = company_test_agent("company-scoped-agent", "Company Scoped Agent");
    seed_principal_to_db(&database.database_url, &company_member).await;
    seed_principal_to_db(&database.database_url, &company_stale_owner).await;
    seed_principal_to_db(&database.database_url, &company_remove_target).await;
    seed_principal_to_db(&database.database_url, &company_agent).await;
    for member_id in [
        company_member.id.clone(),
        company_stale_owner.id.clone(),
        company_remove_target.id.clone(),
    ] {
        let (add_company_member_status, _) = api_json_payload_request(
            test_router.clone(),
            &alice,
            Method::POST,
            format!("/v1/companies/{alice_company_id}/members"),
            json!({
                "actor_id": alice.id.clone(),
                "principal_id": member_id,
                "role": "member",
            }),
        )
        .await;
        assert_eq!(add_company_member_status, StatusCode::CREATED);
    }

    let forbidden_group_name = "forbidden-bob-workspace-group";
    let (forbidden_group_status, _) = api_json_payload_request(
        test_router.clone(),
        &alice,
        Method::POST,
        "/v1/groups".into(),
        json!({
            "actor_id": alice.id.clone(),
            "name": forbidden_group_name,
            "description": null,
            "avatar_url": null,
            "member_ids": [],
            "workspace_id": bob_company_id,
        }),
    )
    .await;
    assert_eq!(forbidden_group_status, StatusCode::FORBIDDEN);

    let (alice_conversations_status, alice_conversations) =
        api_list_conversations(test_router.clone(), &alice).await;
    assert_eq!(alice_conversations_status, StatusCode::OK);
    assert!(
        alice_conversations
            .as_array()
            .unwrap()
            .iter()
            .all(|conversation| conversation["name"] != forbidden_group_name)
    );

    let (company_group_status, company_group) = api_json_payload_request(
        test_router.clone(),
        &alice,
        Method::POST,
        "/v1/groups".into(),
        json!({
            "actor_id": alice.id.clone(),
            "name": "alice-company-group",
            "description": null,
            "avatar_url": null,
            "member_ids": [
                company_member.id.clone(),
                company_remove_target.id.clone(),
                company_agent.id.clone()
            ],
            "workspace_id": alice_company_id,
        }),
    )
    .await;
    assert_eq!(company_group_status, StatusCode::CREATED);
    assert_eq!(
        company_group["workspace_id"].as_str().unwrap(),
        alice_company_id
    );

    let company_conversation_id = company_group["id"].as_str().unwrap();
    let send_status = api_send_text_message(
        test_router.clone(),
        &alice,
        company_conversation_id,
        "company-workspace-message",
        "hello company workspace",
    )
    .await;
    assert_eq!(send_status, StatusCode::CREATED);

    let (member_messages_status, member_messages) = api_list_messages(
        test_router.clone(),
        &company_member,
        company_conversation_id,
    )
    .await;
    assert_eq!(member_messages_status, StatusCode::OK);
    assert_eq!(member_messages.as_array().unwrap().len(), 1);

    let (member_export_status, member_export) = api_json_request(
        test_router.clone(),
        &company_member,
        Method::GET,
        format!(
            "/v1/export/conversations/{company_conversation_id}?actor_id={}",
            company_member.id
        ),
    )
    .await;
    assert_eq!(member_export_status, StatusCode::OK);
    assert_eq!(
        member_export["conversation"]["id"].as_str().unwrap(),
        company_conversation_id
    );
    assert_eq!(member_export["messages"].as_array().unwrap().len(), 1);

    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect for export event seed");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    for idx in 0..205_i64 {
        client
            .execute(
                "INSERT INTO conversation_events
                    (conversation_id, seq, event_id, event_type, sender_id, content,
                     content_type, metadata, client_msg_id, created_at)
                 VALUES ($1, $2, $3, 'message', $4, $5, 'text/plain', '{}'::jsonb, $6, NOW())",
                &[
                    &company_conversation_id,
                    &(idx + 2),
                    &format!("export-extra-event-{idx}"),
                    &alice.id,
                    &format!("export extra {idx}"),
                    &format!("export-extra-client-{idx}"),
                ],
            )
            .await
            .expect("seed export event");
    }
    let (large_export_status, large_export) = api_json_request(
        test_router.clone(),
        &company_member,
        Method::GET,
        format!(
            "/v1/export/conversations/{company_conversation_id}?actor_id={}",
            company_member.id
        ),
    )
    .await;
    assert_eq!(large_export_status, StatusCode::OK);
    assert_eq!(large_export["messages"].as_array().unwrap().len(), 206);

    let (agent_conversations_status, agent_conversations) =
        api_list_conversations(test_router.clone(), &company_agent).await;
    assert_eq!(agent_conversations_status, StatusCode::OK);
    assert!(
        agent_conversations
            .as_array()
            .unwrap()
            .iter()
            .any(|conversation| conversation["id"] == company_conversation_id)
    );

    let (agent_messages_status, agent_messages) =
        api_list_messages(test_router.clone(), &company_agent, company_conversation_id).await;
    assert_eq!(agent_messages_status, StatusCode::OK);
    assert_eq!(agent_messages.as_array().unwrap().len(), 200);

    let (agent_search_status, agent_search) = api_search_messages(
        test_router.clone(),
        &company_agent,
        "hello%20company%20workspace",
        None,
    )
    .await;
    assert_eq!(agent_search_status, StatusCode::OK);
    assert_eq!(agent_search.as_array().unwrap().len(), 1);

    let (agent_unreads_status, agent_unreads) = api_json_request(
        test_router.clone(),
        &company_agent,
        Method::GET,
        "/v1/unreads".into(),
    )
    .await;
    assert_eq!(agent_unreads_status, StatusCode::OK);
    assert!(
        agent_unreads
            .as_array()
            .unwrap()
            .iter()
            .any(|unread| unread["conversation_id"] == company_conversation_id)
    );

    assert_eq!(
        api_remove_group_member(
            test_router.clone(),
            &alice,
            company_conversation_id,
            &company_remove_target.id,
        )
        .await,
        StatusCode::OK
    );

    let (stale_owner_group_status, stale_owner_group) = api_json_payload_request(
        test_router.clone(),
        &company_stale_owner,
        Method::POST,
        "/v1/groups".into(),
        json!({
            "actor_id": company_stale_owner.id.clone(),
            "name": "stale-owner-company-group",
            "description": null,
            "avatar_url": null,
            "member_ids": [],
            "workspace_id": alice_company_id,
        }),
    )
    .await;
    assert_eq!(stale_owner_group_status, StatusCode::CREATED);
    let stale_owner_conversation_id = stale_owner_group["id"].as_str().unwrap();

    let (remove_stale_owner_company_status, _) = api_json_request(
        test_router.clone(),
        &alice,
        Method::DELETE,
        format!(
            "/v1/companies/{alice_company_id}/members/{}",
            company_stale_owner.id
        ),
    )
    .await;
    assert_eq!(remove_stale_owner_company_status, StatusCode::NO_CONTENT);

    let (stale_owner_update_status, _) = api_json_payload_request(
        test_router.clone(),
        &company_stale_owner,
        Method::PATCH,
        format!("/v1/groups/{stale_owner_conversation_id}"),
        json!({
            "actor_id": company_stale_owner.id.clone(),
            "name": "stale-owner-rename",
            "description": null,
            "avatar_url": null,
        }),
    )
    .await;
    assert_eq!(stale_owner_update_status, StatusCode::OK);

    let (stale_owner_add_member_status, _) = api_json_payload_request(
        test_router.clone(),
        &company_stale_owner,
        Method::POST,
        format!("/v1/groups/{stale_owner_conversation_id}/members"),
        json!({
            "actor_id": company_stale_owner.id.clone(),
            "member_ids": [company_member.id.clone()],
        }),
    )
    .await;
    assert_eq!(stale_owner_add_member_status, StatusCode::OK);

    let (remove_company_member_status, _) = api_json_request(
        test_router.clone(),
        &alice,
        Method::DELETE,
        format!(
            "/v1/companies/{alice_company_id}/members/{}",
            company_member.id
        ),
    )
    .await;
    assert_eq!(remove_company_member_status, StatusCode::NO_CONTENT);

    let (removed_member_conversations_status, removed_member_conversations) =
        api_list_conversations(test_router.clone(), &company_member).await;
    assert_eq!(removed_member_conversations_status, StatusCode::OK);
    assert!(
        removed_member_conversations
            .as_array()
            .unwrap()
            .iter()
            .any(|conversation| conversation["id"] == company_conversation_id)
    );

    let (removed_member_messages_status, _) = api_list_messages(
        test_router.clone(),
        &company_member,
        company_conversation_id,
    )
    .await;
    assert_eq!(removed_member_messages_status, StatusCode::OK);

    let (removed_member_search_status, removed_member_search) = api_search_messages(
        test_router.clone(),
        &company_member,
        "hello%20company%20workspace",
        None,
    )
    .await;
    assert_eq!(removed_member_search_status, StatusCode::OK);
    assert_eq!(removed_member_search.as_array().unwrap().len(), 1);

    let (removed_member_unreads_status, removed_member_unreads) = api_json_request(
        test_router.clone(),
        &company_member,
        Method::GET,
        "/v1/unreads".into(),
    )
    .await;
    assert_eq!(removed_member_unreads_status, StatusCode::OK);
    assert!(
        removed_member_unreads
            .as_array()
            .unwrap()
            .iter()
            .any(|unread| unread["conversation_id"] == company_conversation_id)
    );

    let (removed_member_ingest_status, _) = api_json_payload_request(
        test_router.clone(),
        &company_member,
        Method::POST,
        "/v2/ingest".into(),
        json!({
            "conversation_id": company_conversation_id,
            "content": "conversation member ingest",
            "content_type": "text/plain",
            "client_msg_id": "removed-member-ingest",
            "metadata": {},
        }),
    )
    .await;
    assert_eq!(removed_member_ingest_status, StatusCode::CREATED);

    assert_eq!(
        api_view_conversation(
            test_router.clone(),
            &company_member,
            company_conversation_id
        )
        .await,
        StatusCode::NO_CONTENT
    );

    let (removed_member_export_status, _) = api_json_request(
        test_router.clone(),
        &company_member,
        Method::GET,
        format!(
            "/v1/export/conversations/{company_conversation_id}?actor_id={}",
            company_member.id
        ),
    )
    .await;
    assert_eq!(removed_member_export_status, StatusCode::OK);

    let (forbidden_direct_status, _) = api_json_payload_request(
        test_router.clone(),
        &alice,
        Method::POST,
        "/v1/conversations/direct".into(),
        json!({
            "actor_id": alice.id.clone(),
            "peer_principal_id": bob.id.clone(),
            "workspace_id": bob_company_id,
        }),
    )
    .await;
    assert_eq!(forbidden_direct_status, StatusCode::FORBIDDEN);

    let (deleted_status, _) = api_json_request(
        test_router.clone(),
        &alice,
        Method::DELETE,
        format!("/v1/companies/{alice_company_id}"),
    )
    .await;
    assert_eq!(deleted_status, StatusCode::NO_CONTENT);

    let (delete_again_status, _) = api_json_request(
        test_router.clone(),
        &alice,
        Method::DELETE,
        format!("/v1/companies/{alice_company_id}"),
    )
    .await;
    assert_eq!(delete_again_status, StatusCode::NO_CONTENT);

    let (update_deleted_company_status, _) = api_json_payload_request(
        test_router.clone(),
        &alice,
        Method::PATCH,
        format!("/v1/companies/{alice_company_id}"),
        json!({
            "actor_id": alice.id.clone(),
            "name": "deleted company rename",
            "description": null,
            "avatar_url": null,
            "agents_active": null,
            "folder_path": null,
        }),
    )
    .await;
    assert_eq!(update_deleted_company_status, StatusCode::NOT_FOUND);

    let (archive_deleted_company_status, _) = api_json_payload_request(
        test_router.clone(),
        &alice,
        Method::POST,
        format!("/v1/companies/{alice_company_id}/archive"),
        json!({ "actor_id": alice.id.clone() }),
    )
    .await;
    assert_eq!(archive_deleted_company_status, StatusCode::NOT_FOUND);

    let (unarchive_deleted_company_status, _) = api_json_payload_request(
        test_router.clone(),
        &alice,
        Method::POST,
        format!("/v1/companies/{alice_company_id}/unarchive"),
        json!({ "actor_id": alice.id.clone() }),
    )
    .await;
    assert_eq!(unarchive_deleted_company_status, StatusCode::NOT_FOUND);

    let (add_deleted_company_member_status, _) = api_json_payload_request(
        test_router.clone(),
        &alice,
        Method::POST,
        format!("/v1/companies/{alice_company_id}/members"),
        json!({
            "actor_id": alice.id.clone(),
            "principal_id": company_agent.id.clone(),
            "role": "member",
        }),
    )
    .await;
    assert_eq!(add_deleted_company_member_status, StatusCode::NOT_FOUND);

    let (remove_deleted_company_member_status, _) = api_json_request(
        test_router.clone(),
        &alice,
        Method::DELETE,
        format!(
            "/v1/companies/{alice_company_id}/members/{}",
            company_remove_target.id
        ),
    )
    .await;
    assert_eq!(remove_deleted_company_member_status, StatusCode::NOT_FOUND);

    let (deleted_company_conversations_status, deleted_company_conversations) =
        api_list_conversations(test_router.clone(), &alice).await;
    assert_eq!(deleted_company_conversations_status, StatusCode::OK);
    assert!(
        deleted_company_conversations
            .as_array()
            .unwrap()
            .iter()
            .all(|conversation| conversation["id"] != company_conversation_id)
    );

    let (deleted_company_messages_status, _) =
        api_list_messages(test_router.clone(), &alice, company_conversation_id).await;
    assert_eq!(deleted_company_messages_status, StatusCode::FORBIDDEN);

    let (admin_deleted_company_messages_status, _) =
        api_list_messages(test_router.clone(), &operator, company_conversation_id).await;
    assert_eq!(admin_deleted_company_messages_status, StatusCode::FORBIDDEN);

    let (deleted_company_search_status, deleted_company_search) = api_search_messages(
        test_router.clone(),
        &alice,
        "hello%20company%20workspace",
        None,
    )
    .await;
    assert_eq!(deleted_company_search_status, StatusCode::OK);
    assert!(deleted_company_search.as_array().unwrap().is_empty());

    let (deleted_company_unreads_status, deleted_company_unreads) = api_json_request(
        test_router.clone(),
        &alice,
        Method::GET,
        "/v1/unreads".into(),
    )
    .await;
    assert_eq!(deleted_company_unreads_status, StatusCode::OK);
    assert!(
        deleted_company_unreads
            .as_array()
            .unwrap()
            .iter()
            .all(|unread| unread["conversation_id"] != company_conversation_id)
    );

    let (deleted_company_ingest_status, _) = api_json_payload_request(
        test_router.clone(),
        &alice,
        Method::POST,
        "/v2/ingest".into(),
        json!({
            "conversation_id": company_conversation_id,
            "content": "deleted company ingest should fail",
            "content_type": "text/plain",
            "client_msg_id": "deleted-company-ingest",
            "metadata": {},
        }),
    )
    .await;
    assert_eq!(deleted_company_ingest_status, StatusCode::FORBIDDEN);

    assert_eq!(
        api_view_conversation(test_router.clone(), &alice, company_conversation_id).await,
        StatusCode::FORBIDDEN
    );

    let (deleted_company_export_status, _) = api_json_request(
        test_router.clone(),
        &alice,
        Method::GET,
        format!(
            "/v1/export/conversations/{company_conversation_id}?actor_id={}",
            alice.id
        ),
    )
    .await;
    assert_eq!(deleted_company_export_status, StatusCode::FORBIDDEN);

    let (read_deleted_status, _) = api_json_request(
        test_router.clone(),
        &alice,
        Method::GET,
        format!("/v1/companies/{alice_company_id}"),
    )
    .await;
    assert_eq!(read_deleted_status, StatusCode::NOT_FOUND);

    let (deleted_group_status, _) = api_json_payload_request(
        test_router,
        &alice,
        Method::POST,
        "/v1/groups".into(),
        json!({
            "actor_id": alice.id,
            "name": "deleted-company-group",
            "description": null,
            "avatar_url": null,
            "member_ids": [],
            "workspace_id": alice_company_id,
        }),
    )
    .await;
    assert_eq!(deleted_group_status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn console_snapshot_reports_enabled_host_plugins() {
    let _env = ChannelTaskEnvGuard::disabled();
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let router = router_with_db(app.clone(), &database.database_url);
    let alice = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-channel-task-flags".into(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &alice).await;

    let (snapshot_status, snapshot) = api_console_snapshot(router, &alice).await;
    assert_eq!(snapshot_status, StatusCode::OK);
    assert_eq!(snapshot["plugins"].as_array().map(Vec::len), Some(1));
    assert_eq!(snapshot["plugins"][0]["id"], "pixel-world");
}

#[tokio::test]
async fn console_snapshot_includes_visible_conversation_member_principals() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let router = router_with_db(app.clone(), &database.database_url);
    let operator = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-channel-task-members".into(),
            principal_type: PrincipalType::Human,
            name: "Operator".into(),
            avatar_url: None,
        })
        .unwrap();
    let alice = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-channel-task-members".into(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    let bob = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-channel-task-members".into(),
            principal_type: PrincipalType::Human,
            name: "Bob".into(),
            avatar_url: None,
        })
        .unwrap();
    let agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Task Agent".into(),
            scopes: vec!["messages:read".into(), "messages:write".into()],
            workspace_id: Some("ws-channel-task-members".into()),
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let internal_agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Internal Task Agent".into(),
            scopes: vec!["messages:read".into(), "messages:write".into()],
            workspace_id: Some("ws-channel-task-members".into()),
            channel_visibility: Some(choruz_domain::ChannelVisibility::Internal),
        })
        .unwrap()
        .principal;
    let conversation = app
        .create_group(CreateGroupRequest {
            actor_id: operator.id.clone(),
            name: "Task Members".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![
                alice.id.clone(),
                bob.id.clone(),
                agent.id.clone(),
                internal_agent.id.clone(),
            ],
            workspace_id: Some("ws-channel-task-members".into()),
        })
        .unwrap();

    for principal in [&operator, &alice, &bob, &agent, &internal_agent] {
        seed_principal_to_db(&database.database_url, principal).await;
    }
    seed_conversation_to_db(&database.database_url, &conversation).await;

    let (snapshot_status, snapshot) = api_console_snapshot(router, &alice).await;
    assert_eq!(snapshot_status, StatusCode::OK);
    let principal_ids: Vec<_> = snapshot["principals"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|principal| principal["id"].as_str())
        .collect();
    assert!(principal_ids.contains(&alice.id.as_str()));
    assert!(principal_ids.contains(&bob.id.as_str()));
    assert!(principal_ids.contains(&agent.id.as_str()));
    assert!(principal_ids.contains(&operator.id.as_str()));
    assert!(!principal_ids.contains(&internal_agent.id.as_str()));
    for principal in snapshot["principals"]
        .as_array()
        .unwrap()
        .iter()
        .chain(snapshot["agents"].as_array().unwrap().iter())
    {
        assert!(
            principal.get("secret_hash").is_none(),
            "console principals must be redacted"
        );
        assert!(
            principal.get("deleted_at").is_none(),
            "console principals must omit deletion metadata"
        );
    }
}
