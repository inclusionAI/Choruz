use super::*;

#[tokio::test]
async fn db_create_agent_allows_human_in_own_company_but_denies_other_company() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let human = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "human-workspace".into(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    let other_human = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "other-workspace".into(),
            principal_type: PrincipalType::Human,
            name: "Bob".into(),
            avatar_url: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &human).await;
    seed_principal_to_db(&database.database_url, &other_human).await;

    let db = choruz_application::DbService::new(choruz_store::EventStore::new(
        database.database_url.clone(),
    ));
    let company = db
        .create_company(CreateCompanyRequest {
            actor_id: human.id.clone(),
            name: "Alice Co".into(),
            slug: Some("alice-co".into()),
            description: None,
            folder_path: None,
        })
        .await
        .unwrap();
    let other_company = db
        .create_company(CreateCompanyRequest {
            actor_id: other_human.id,
            name: "Bob Co".into(),
            slug: Some("bob-co".into()),
            description: None,
            folder_path: None,
        })
        .await
        .unwrap();

    let agent = db
        .create_agent(CreateAgentRequest {
            actor_id: human.id.clone(),
            name: "Alice agent".into(),
            scopes: vec![],
            workspace_id: Some(company.id.clone()),
            channel_visibility: None,
        })
        .await
        .unwrap()
        .principal;
    assert_eq!(agent.workspace_id, company.id);

    let internal_agent = db
        .create_agent(CreateAgentRequest {
            actor_id: human.id.clone(),
            name: "Internal agent".into(),
            scopes: vec![],
            workspace_id: Some(company.id.clone()),
            channel_visibility: Some(choruz_domain::ChannelVisibility::Internal),
        })
        .await
        .unwrap()
        .principal;
    assert_eq!(
        internal_agent.channel_visibility,
        choruz_domain::ChannelVisibility::Internal
    );

    let err = db
        .create_agent(CreateAgentRequest {
            actor_id: human.id,
            name: "Unauthorized agent".into(),
            scopes: vec![],
            workspace_id: Some(other_company.id),
            channel_visibility: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Forbidden(_)));
}

#[tokio::test]
async fn cron_endpoints_reject_foreign_workspace_agent() {
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
    let foreign_agent = choruz_domain::Principal {
        id: "foreign-agent".into(),
        workspace_id: "ws-foreign".into(),
        principal_type: PrincipalType::Agent,
        name: "Foreign Agent".into(),
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
    app.inject_principal(foreign_agent.clone());
    seed_principal_to_db(&database.database_url, &operator).await;
    seed_principal_to_db(&database.database_url, &foreign_agent).await;

    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect for cron authorization seed");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let timestamp = choruz_common::now();
    for (workspace_id, owner_id) in [
        (operator.workspace_id.as_str(), operator.id.as_str()),
        (
            foreign_agent.workspace_id.as_str(),
            foreign_agent.id.as_str(),
        ),
    ] {
        client
            .execute(
                "INSERT INTO company
                    (id, name, slug, owner_id, created_at, updated_at)
                 VALUES ($1, $1, $1, $2, $3, $3)",
                &[&workspace_id, &owner_id, &timestamp],
            )
            .await
            .expect("seed cron authorization company");
    }
    client
        .execute(
            "INSERT INTO company_member (company_id, principal_id, joined_at)
             VALUES ($1, $2, $3)",
            &[&operator.workspace_id, &operator.id, &timestamp],
        )
        .await
        .expect("seed operator company membership");

    let test_router = router_with_db(app, &database.database_url);
    let collection_uri = format!("/v1/agents/{}/cron", foreign_agent.id);
    let item_uri = format!("{collection_uri}/foreign-job");

    let (list_status, _) = api_json_request(
        test_router.clone(),
        &operator,
        Method::GET,
        collection_uri.clone(),
    )
    .await;
    let (create_status, _) = api_json_payload_request(
        test_router.clone(),
        &operator,
        Method::POST,
        collection_uri,
        json!({
            "name": "foreign",
            "schedule_type": "every",
            "schedule_value": "60",
            "message": "must not be created"
        }),
    )
    .await;
    let (update_status, _) = api_json_payload_request(
        test_router.clone(),
        &operator,
        Method::PATCH,
        item_uri.clone(),
        json!({"name": "must not update"}),
    )
    .await;
    let (delete_status, _) =
        api_json_request(test_router, &operator, Method::DELETE, item_uri).await;

    assert_eq!(list_status, StatusCode::FORBIDDEN);
    assert_eq!(create_status, StatusCode::FORBIDDEN);
    assert_eq!(update_status, StatusCode::FORBIDDEN);
    assert_eq!(delete_status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn cron_create_rejects_conversation_without_agent_membership() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let operator = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-cron-membership".into(),
            principal_type: PrincipalType::Human,
            name: "Operator".into(),
            avatar_url: None,
        })
        .unwrap();
    let agent = choruz_domain::Principal {
        id: "cron-membership-agent".into(),
        workspace_id: operator.workspace_id.clone(),
        principal_type: PrincipalType::Agent,
        name: "Cron Agent".into(),
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
    app.inject_principal(agent.clone());
    let conversation = app
        .create_group(CreateGroupRequest {
            actor_id: operator.id.clone(),
            name: "Human-only conversation".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![],
            workspace_id: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &operator).await;
    seed_principal_to_db(&database.database_url, &agent).await;
    seed_conversation_to_db(&database.database_url, &conversation).await;

    let test_router = router_with_db(app, &database.database_url);
    let (status, _) = api_json_payload_request(
        test_router,
        &operator,
        Method::POST,
        format!("/v1/agents/{}/cron", agent.id),
        json!({
            "name": "invalid target",
            "schedule_type": "every",
            "schedule_value": "60",
            "message": "must not be created",
            "conversation_id": conversation.id
        }),
    )
    .await;

    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn generic_principal_creation_route_is_not_registered() {
    let database = TestDatabase::create().await;
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
    seed_principal_to_db(&database.database_url, &operator).await;
    seed_principal_to_db(&database.database_url, &human).await;
    let router = router_with_db(app.clone(), &database.database_url);

    let unauthorized = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/principals")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_vec(&CreatePrincipalRequest {
                        workspace_id: operator.workspace_id.clone(),
                        principal_type: PrincipalType::Human,
                        name: "Unauthorized".into(),
                        avatar_url: None,
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::NOT_FOUND);

    let human_create = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/principals")
                .header("authorization", format!("Bearer {}", session_token(&human)))
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_vec(&CreatePrincipalRequest {
                        workspace_id: operator.workspace_id.clone(),
                        principal_type: PrincipalType::Human,
                        name: "Forbidden".into(),
                        avatar_url: None,
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(human_create.status(), StatusCode::NOT_FOUND);

    let forbidden_workspace = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/principals")
                .header(
                    "authorization",
                    format!("Bearer {}", session_token(&operator)),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_vec(&CreatePrincipalRequest {
                        workspace_id: "ws-other".into(),
                        principal_type: PrincipalType::Human,
                        name: "Cross Workspace".into(),
                        avatar_url: None,
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(forbidden_workspace.status(), StatusCode::NOT_FOUND);

    let spoofed = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/principals")
                .header(
                    "authorization",
                    format!("Bearer {}", session_token(&operator)),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "workspace_id": operator.workspace_id.clone(),
                        "principal_type": "human",
                        "name": "Spoofed Visibility",
                        "avatar_url": null,
                        "channel_visibility": "internal"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(spoofed.status(), StatusCode::NOT_FOUND);

    let created = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/principals")
                .header(
                    "authorization",
                    format!("Bearer {}", session_token(&operator)),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_vec(&CreatePrincipalRequest {
                        workspace_id: operator.workspace_id.clone(),
                        principal_type: PrincipalType::Human,
                        name: "Created".into(),
                        avatar_url: None,
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn create_agent_allows_humans_to_set_internal_channel_visibility() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let auth = LocalAuthConfig::from_env();
    let operator = auth.ensure_operator_sync(&app).unwrap();
    let human = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: operator.workspace_id.clone(),
            principal_type: PrincipalType::Human,
            name: "Agent Creator Impostor".into(),
            avatar_url: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &operator).await;
    seed_principal_to_db(&database.database_url, &human).await;
    let router = router_with_db(app, &database.database_url);

    let human_created = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/agents")
                .header("authorization", format!("Bearer {}", session_token(&human)))
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_vec(&CreateAgentRequest {
                        actor_id: human.id.clone(),
                        name: "Human Internal Delegate".into(),
                        scopes: vec!["messages:read".into()],
                        workspace_id: None,
                        channel_visibility: Some(choruz_domain::ChannelVisibility::Internal),
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(human_created.status(), StatusCode::CREATED);

    let spoofed_actor = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/agents")
                .header("authorization", format!("Bearer {}", session_token(&human)))
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_vec(&CreateAgentRequest {
                        actor_id: operator.id.clone(),
                        name: "Spoofed Internal Delegate".into(),
                        scopes: vec!["messages:read".into()],
                        workspace_id: None,
                        channel_visibility: Some(choruz_domain::ChannelVisibility::Internal),
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        matches!(
            spoofed_actor.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ),
        "token principal must not be able to create agents by spoofing actor_id"
    );

    let created = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/agents")
                .header(
                    "authorization",
                    format!("Bearer {}", session_token(&operator)),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_vec(&CreateAgentRequest {
                        actor_id: operator.id.clone(),
                        name: "Internal Delegate".into(),
                        scopes: vec!["messages:read".into()],
                        workspace_id: None,
                        channel_visibility: Some(choruz_domain::ChannelVisibility::Internal),
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let body = to_bytes(created.into_body(), usize::MAX).await.unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let agent_id = payload["principal"]["id"].as_str().unwrap();
    assert_eq!(
        payload["principal"]["channel_visibility"].as_str(),
        Some("internal")
    );

    let (db, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let row = db
        .query_one(
            "SELECT channel_visibility FROM principal WHERE id = $1",
            &[&agent_id],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>("channel_visibility"), "internal");
}

#[tokio::test]
async fn batch_disable_soft_deletes_agents_so_names_can_be_reused() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let auth = LocalAuthConfig::from_env();
    let operator = auth.ensure_operator_sync(&app).unwrap();
    seed_principal_to_db(&database.database_url, &operator).await;
    let router = router_with_db(app.clone(), &database.database_url);
    let token = session_token(&operator);

    let create_body = CreateAgentRequest {
        actor_id: operator.id.clone(),
        name: "project-operator".into(),
        scopes: vec!["messages:read".into()],
        workspace_id: None,
        channel_visibility: None,
    };
    let created = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/agents")
                .header("authorization", format!("Bearer {token}"))
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&create_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let created_body = to_bytes(created.into_body(), usize::MAX).await.unwrap();
    let created_payload: Value = serde_json::from_slice(&created_body).unwrap();
    let agent_id = created_payload["principal"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let conversation_id = "batch-disable-binding-conversation";
    let (mut db_client, db_connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect binding prerequisites");
    tokio::spawn(async move {
        let _ = db_connection.await;
    });
    db_client
        .execute(
            "INSERT INTO conversation
             (id, workspace_id, type, creator_id, created_at, updated_at)
             VALUES ($1, $2, 'direct', $3, NOW(), NOW())",
            &[&conversation_id, &operator.workspace_id, &operator.id],
        )
        .await
        .expect("seed binding conversation");
    let runtime = RuntimeStore::new(database.database_url.clone());
    let batch_binding_id = choruz_common::new_id();
    let batch_create_tx = db_client.transaction().await.expect("begin binding create");
    batch_create_tx
        .query_one(
            "SELECT id FROM principal WHERE id = $1 FOR SHARE",
            &[&agent_id],
        )
        .await
        .expect("lock Agent for binding creation");
    batch_create_tx
        .execute(
            "INSERT INTO agent_runtime_bindings
             (id, conversation_id, agent_principal_id, driver_type, workspace_path)
             VALUES ($1, $2, $3, 'opencode_terminal', '/tmp/batch-disable-binding')",
            &[&batch_binding_id, &conversation_id, &agent_id],
        )
        .await
        .expect("insert binding while Agent is share-locked");

    let batch_router = router.clone();
    let batch_token = token.clone();
    let batch_actor_id = operator.id.clone();
    let batch_agent_id = agent_id.clone();
    let mut batch_disable = tokio::spawn(async move {
        batch_router
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/agents/batch-disable")
                    .header("authorization", format!("Bearer {batch_token}"))
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({
                            "actor_id": batch_actor_id,
                            "agent_ids": [batch_agent_id],
                            "conversation_ids": [],
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap()
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut batch_disable)
            .await
            .is_err(),
        "batch disable must wait for an in-flight binding creation lock"
    );
    batch_create_tx
        .commit()
        .await
        .expect("commit binding creation");
    let disabled = batch_disable.await.expect("join batch disable");
    assert_eq!(disabled.status(), StatusCode::OK);
    assert_eq!(
        runtime.get_binding(&batch_binding_id).await.unwrap().state,
        BindingState::Disabled,
        "batch disable must catch the binding committed ahead of it"
    );
    assert!(
        runtime
            .list_active_bindings()
            .await
            .unwrap()
            .iter()
            .all(|active| active.agent_principal_id != agent_id)
    );
    assert!(
        runtime
            .create_binding(CreateBindingInput {
                conversation_id: conversation_id.into(),
                agent_principal_id: agent_id.clone(),
                driver_type: DriverType::OpenCodeTerminal,
                workspace_path: "/tmp/rejected-batch-binding".into(),
                git_worktree_path: None,
                config_json: json!({}),
                audit_actor: None,
            })
            .await
            .is_err(),
        "binding creation must reject a soft-deleted Agent"
    );

    let recreated = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/agents")
                .header("authorization", format!("Bearer {token}"))
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&create_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(recreated.status(), StatusCode::CREATED);
    let recreated_body = to_bytes(recreated.into_body(), usize::MAX).await.unwrap();
    let recreated_payload: Value = serde_json::from_slice(&recreated_body).unwrap();
    let recreated_agent_id = recreated_payload["principal"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let single_conversation_id = "single-disable-binding-conversation";
    db_client
        .execute(
            "INSERT INTO conversation
             (id, workspace_id, type, creator_id, created_at, updated_at)
             VALUES ($1, $2, 'direct', $3, NOW(), NOW())",
            &[
                &single_conversation_id,
                &operator.workspace_id,
                &operator.id,
            ],
        )
        .await
        .expect("seed single-disable conversation");
    let single_binding_id = choruz_common::new_id();
    let single_create_tx = db_client.transaction().await.expect("begin binding create");
    single_create_tx
        .query_one(
            "SELECT id FROM principal WHERE id = $1 FOR SHARE",
            &[&recreated_agent_id],
        )
        .await
        .expect("lock Agent for binding creation");
    single_create_tx
        .execute(
            "INSERT INTO agent_runtime_bindings
             (id, conversation_id, agent_principal_id, driver_type, workspace_path)
             VALUES ($1, $2, $3, 'claude_print', '/tmp/single-disable-binding')",
            &[
                &single_binding_id,
                &single_conversation_id,
                &recreated_agent_id,
            ],
        )
        .await
        .expect("insert single binding while Agent is share-locked");
    let single_actor_id = operator.id.clone();
    let single_agent_id = recreated_agent_id.clone();
    let mut single_disable = tokio::spawn(async move {
        router
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(format!(
                        "/v1/principals/{single_agent_id}/disable?actor_id={single_actor_id}"
                    ))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut single_disable)
            .await
            .is_err(),
        "single disable must wait for an in-flight binding creation lock"
    );
    single_create_tx
        .commit()
        .await
        .expect("commit single binding creation");
    let single_disabled = single_disable.await.expect("join single disable");
    assert_eq!(single_disabled.status(), StatusCode::OK);
    assert_eq!(
        runtime.get_binding(&single_binding_id).await.unwrap().state,
        BindingState::Disabled
    );
    assert!(
        runtime
            .create_binding(CreateBindingInput {
                conversation_id: single_conversation_id.into(),
                agent_principal_id: recreated_agent_id,
                driver_type: DriverType::ClaudePrint,
                workspace_path: "/tmp/rejected-single-binding".into(),
                git_worktree_path: None,
                config_json: json!({}),
                audit_actor: None,
            })
            .await
            .is_err(),
        "binding creation must reject a disabled Agent"
    );
}

#[tokio::test]
async fn human_role_migration_preserves_one_user_and_backfills_membership() {
    let database = TestDatabase::create_without_migrations().await;
    database
        .apply_migrations_through("V019__choruz_database_cutover.sql")
        .await;
    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect for human-role migration setup");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .batch_execute(
            "INSERT INTO principal
               (id, workspace_id, type, name, disabled, channel_visibility, created_at, updated_at)
             VALUES
               ('legacy-human', 'ws-human', 'human', 'Signed Up User', FALSE, 'visible', NOW() - INTERVAL '1 day', NOW()),
               ('legacy-admin', 'ws-local', 'admin', 'Bootstrap Admin', FALSE, 'visible', NOW(), NOW());
             INSERT INTO company
               (id, name, slug, owner_id, agents_active, created_at, updated_at)
             VALUES ('legacy-company', 'Legacy', 'legacy', 'legacy-admin', TRUE, NOW(), NOW());
             INSERT INTO company_member (company_id, principal_id, role, joined_at)
             VALUES ('legacy-company', 'legacy-admin', 'owner', NOW());
             INSERT INTO conversation
               (id, workspace_id, type, name, creator_id, created_at, updated_at)
             VALUES ('legacy-conversation', 'legacy-company', 'group', 'Legacy Group', 'legacy-admin', NOW(), NOW());
             INSERT INTO conversation_member (conv_id, principal_id, role, joined_at)
             VALUES ('legacy-conversation', 'legacy-admin', 'owner', NOW());",
        )
        .await
        .expect("seed legacy human/admin state");

    database
        .apply_migration("V021__collapse_human_roles.sql")
        .await;

    let active_humans: i64 = client
        .query_one(
            "SELECT count(*) FROM principal
             WHERE type = 'human' AND disabled = FALSE AND deleted_at IS NULL",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(active_humans, 1);
    let company_owner: String = client
        .query_one(
            "SELECT owner_id FROM company WHERE id = 'legacy-company'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(company_owner, "legacy-human");
    let human_membership: i64 = client
        .query_one(
            "SELECT count(*) FROM conversation_member
             WHERE conv_id = 'legacy-conversation'
               AND principal_id = 'legacy-human'
               AND removed_at IS NULL",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(human_membership, 1);
    let role_columns: i64 = client
        .query_one(
            "SELECT count(*) FROM information_schema.columns
             WHERE table_schema = 'public'
               AND table_name IN ('company_member', 'conversation_member')
               AND column_name = 'role'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(role_columns, 0);
}

#[tokio::test]
async fn agent_privacy_surfaces_are_scoped_to_authorized_workspace_context() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let runtime = RuntimeStore::new(database.database_url.clone());
    let router = runtime_router_with_db(app.clone(), runtime.clone(), &database.database_url);

    let alice = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "team007-alice-home".into(),
            principal_type: PrincipalType::Human,
            name: "Team007 Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    let bob = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "team007-bob-home".into(),
            principal_type: PrincipalType::Human,
            name: "Team007 Bob".into(),
            avatar_url: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &alice).await;
    seed_principal_to_db(&database.database_url, &bob).await;

    let (alice_company_status, alice_company) = api_json_payload_request(
        router.clone(),
        &alice,
        Method::POST,
        "/v1/companies".into(),
        json!({
            "actor_id": alice.id.clone(),
            "name": "Team007 Alice Company",
            "description": null,
        }),
    )
    .await;
    assert_eq!(alice_company_status, StatusCode::CREATED);
    let alice_company_id = alice_company["id"].as_str().unwrap().to_string();

    let (bob_company_status, bob_company) = api_json_payload_request(
        router.clone(),
        &bob,
        Method::POST,
        "/v1/companies".into(),
        json!({
            "actor_id": bob.id.clone(),
            "name": "Team007 Bob Company",
            "description": null,
        }),
    )
    .await;
    assert_eq!(bob_company_status, StatusCode::CREATED);
    let bob_company_id = bob_company["id"].as_str().unwrap().to_string();

    let agent_a = choruz_domain::Principal {
        id: "team007-agent-a".into(),
        workspace_id: alice_company_id.clone(),
        principal_type: PrincipalType::Agent,
        name: "Team007 Agent A".into(),
        avatar_url: None,
        scopes: vec![
            "messages:read".into(),
            "messages:write".into(),
            "events:read".into(),
        ],
        secret_hash: None,
        disabled: false,
        deleted_at: None,
        channel_visibility: choruz_domain::ChannelVisibility::Visible,
        created_at: choruz_common::now(),
        updated_at: choruz_common::now(),
        user_id: None,
    };
    let agent_b = choruz_domain::Principal {
        id: "team007-agent-b".into(),
        workspace_id: bob_company_id.clone(),
        principal_type: PrincipalType::Agent,
        name: "Team007 Agent B".into(),
        avatar_url: None,
        scopes: vec![
            "messages:read".into(),
            "messages:write".into(),
            "events:read".into(),
        ],
        secret_hash: None,
        disabled: false,
        deleted_at: None,
        channel_visibility: choruz_domain::ChannelVisibility::Visible,
        created_at: choruz_common::now(),
        updated_at: choruz_common::now(),
        user_id: None,
    };
    seed_principal_to_db(&database.database_url, &agent_a).await;
    seed_principal_to_db(&database.database_url, &agent_b).await;

    let (alice_group_status, alice_group) = api_json_payload_request(
        router.clone(),
        &alice,
        Method::POST,
        "/v1/groups".into(),
        json!({
            "actor_id": alice.id.clone(),
            "name": "Team007 Alice Group",
            "description": null,
            "avatar_url": null,
            "member_ids": [agent_a.id.clone()],
            "workspace_id": alice_company_id,
        }),
    )
    .await;
    assert_eq!(alice_group_status, StatusCode::CREATED);
    let alice_group_id = alice_group["id"].as_str().unwrap().to_string();

    let (bob_group_status, bob_group) = api_json_payload_request(
        router.clone(),
        &bob,
        Method::POST,
        "/v1/groups".into(),
        json!({
            "actor_id": bob.id.clone(),
            "name": "Team007 Bob Group",
            "description": null,
            "avatar_url": null,
            "member_ids": [agent_b.id.clone()],
            "workspace_id": bob_company_id,
        }),
    )
    .await;
    assert_eq!(bob_group_status, StatusCode::CREATED);
    let bob_group_id = bob_group["id"].as_str().unwrap().to_string();

    assert_eq!(
        api_send_text_message(
            router.clone(),
            &alice,
            &alice_group_id,
            "team007-alice-message",
            "team007 alice authorized context",
        )
        .await,
        StatusCode::CREATED
    );
    assert_eq!(
        api_send_text_message(
            router.clone(),
            &bob,
            &bob_group_id,
            "team007-bob-message",
            "team007 bob private context",
        )
        .await,
        StatusCode::CREATED
    );

    let bob_attachment_upload = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/attachments")
                .header("authorization", format!("Bearer {}", session_token(&bob)))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&UploadAttachmentRequest {
                        actor_id: bob.id.clone(),
                        filename: "team007-bob.txt".into(),
                        content_type: "text/plain".into(),
                        data_base64: "dGVhbTAwNyBib2IgYXR0YWNobWVudCBieXRlcw==".into(),
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bob_attachment_upload.status(), StatusCode::CREATED);
    let bob_attachment_body = to_bytes(bob_attachment_upload.into_body(), usize::MAX)
        .await
        .unwrap();
    let bob_attachment: AttachmentRecord = serde_json::from_slice(&bob_attachment_body).unwrap();

    let bob_attachment_message = router
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
                        conversation_id: bob_group_id.clone(),
                        idempotency_key: "team007-bob-attachment-message".into(),
                        content: "Attachment: team007-bob.txt".into(),
                        content_type: "attachment".into(),
                        metadata: json!({
                            "attachment_id": bob_attachment.id.clone(),
                            "filename": bob_attachment.filename.clone(),
                            "mime_type": bob_attachment.content_type.clone(),
                            "size_bytes": bob_attachment.size_bytes,
                            "download_path": bob_attachment.download_path.clone(),
                        }),
                        trace_id: None,
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bob_attachment_message.status(), StatusCode::CREATED);

    let bob_binding = runtime
        .create_binding(CreateBindingInput {
            conversation_id: bob_group_id.clone(),
            agent_principal_id: agent_b.id.clone(),
            driver_type: DriverType::CodexExec,
            workspace_path: "/private/team007-bob-workspace-path".into(),
            git_worktree_path: Some("/private/team007-bob-worktree-path".into()),
            config_json: json!({}),
            audit_actor: None,
        })
        .await
        .unwrap();

    let (agent_a_conversations_status, agent_a_conversations) =
        api_list_conversations(router.clone(), &agent_a).await;
    assert_eq!(agent_a_conversations_status, StatusCode::OK);
    assert!(
        agent_a_conversations
            .as_array()
            .unwrap()
            .iter()
            .any(|conversation| conversation["id"] == alice_group_id)
    );
    assert!(
        agent_a_conversations
            .as_array()
            .unwrap()
            .iter()
            .all(|conversation| conversation["id"] != bob_group_id)
    );

    let (agent_a_reads_bob_status, _) =
        api_list_messages(router.clone(), &agent_a, &bob_group_id).await;
    assert_eq!(agent_a_reads_bob_status, StatusCode::FORBIDDEN);

    let (agent_a_search_bob_status, agent_a_search_bob) =
        api_search_messages(router.clone(), &agent_a, "team007%20bob%20private", None).await;
    assert_eq!(agent_a_search_bob_status, StatusCode::OK);
    assert!(agent_a_search_bob.as_array().unwrap().is_empty());

    let agent_a_downloads_bob_attachment = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/attachments/{}?actor_id={}",
                    bob_attachment.id, agent_a.id
                ))
                .header(
                    "authorization",
                    format!("Bearer {}", session_token(&agent_a)),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        agent_a_downloads_bob_attachment.status(),
        StatusCode::FORBIDDEN
    );

    let agent_a_lists_runtime = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/runtime/bindings")
                .header(
                    "authorization",
                    format!("Bearer {}", session_token(&agent_a)),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(agent_a_lists_runtime.status(), StatusCode::FORBIDDEN);

    let agent_a_reads_bob_binding = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/runtime/bindings/{}", bob_binding.id))
                .header(
                    "authorization",
                    format!("Bearer {}", session_token(&agent_a)),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(agent_a_reads_bob_binding.status(), StatusCode::FORBIDDEN);

    let (agent_b_messages_status, agent_b_messages) =
        api_list_messages(router.clone(), &agent_b, &bob_group_id).await;
    assert_eq!(agent_b_messages_status, StatusCode::OK);
    assert!(
        agent_b_messages
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["content"] == "team007 bob private context")
    );

    let agent_b_downloads_bob_attachment = router
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/attachments/{}?actor_id={}",
                    bob_attachment.id, agent_b.id
                ))
                .header(
                    "authorization",
                    format!("Bearer {}", session_token(&agent_b)),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(agent_b_downloads_bob_attachment.status(), StatusCode::OK);
}
