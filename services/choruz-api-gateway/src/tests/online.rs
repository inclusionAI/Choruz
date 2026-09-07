use super::*;
use choruz_application::db_service::OnlineIdentity;

#[tokio::test]
async fn online_verification_preserves_throttling_and_rechecks_recovery_and_revocation() {
    let database = TestDatabase::create().await;
    let db =
        choruz_application::DbService::new(choruz_store::EventStore::new(&database.database_url));
    let human = db
        .create_human_user("online-throttling", "test-password")
        .await
        .unwrap();
    let upstream_status = Arc::new(AtomicUsize::new(429));
    let status = upstream_status.clone();
    let upstream = Router::new().route(
        "/v1/online/auth/get-session",
        get(move || {
            let status = status.clone();
            async move {
                (
                    StatusCode::from_u16(status.load(Ordering::SeqCst) as u16).unwrap(),
                    [("x-retry-after", "7")],
                    AxumJson(json!({"user":{"id":"cloud-account"}})),
                )
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        axum::serve(listener, upstream)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    db.connect_online_identity(
        &human,
        &OnlineIdentity {
            account_id: "cloud-account".into(),
            device_id: "cloud-device".into(),
            service_url: format!("http://{address}"),
            session_token: "fixture-session".into(),
            display_name: "Online".into(),
        },
    )
    .await
    .unwrap();
    let router = router_with_db(choruz_application::ChatApp::new(), &database.database_url);
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/online/session")
                .header("authorization", format!("Bearer {}", session_token(&human)))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers()["retry-after"], "7");
    assert!(db.online_identity(&human).await.unwrap().is_some());
    // Local history remains available; it does not grant cloud transport access.
    let (status, _) = api_json_request(
        router.clone(),
        &human,
        Method::GET,
        "/v1/online/groups".into(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    upstream_status.store(200, Ordering::SeqCst);
    let (status, body) = api_json_request(
        router.clone(),
        &human,
        Method::GET,
        "/v1/online/session".into(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["state"], "signed_in");
    upstream_status.store(401, Ordering::SeqCst);
    let (status, body) =
        api_json_request(router, &human, Method::GET, "/v1/online/session".into()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["state"], "reauth_required");
    let _ = stop.send(());
    server.await.unwrap();
}

#[tokio::test]
async fn online_routes_reject_agent_credentials_before_contacting_account_service() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let human = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "online-routes".into(),
            principal_type: PrincipalType::Human,
            name: "owner".into(),
            avatar_url: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &human).await;
    let agent = app
        .create_agent(CreateAgentRequest {
            actor_id: human.id.clone(),
            name: "agent".into(),
            scopes: vec![],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    seed_principal_to_db(&database.database_url, &agent).await;
    let router = router_with_db(app, &database.database_url);
    for path in ["sign-in", "sign-up"] {
        let (status, _) = api_json_payload_request(
            router.clone(),
            &agent,
            Method::POST,
            format!("/v1/online/{path}"),
            json!({"email":"agent@example.test","password":"test-password","name":"Agent"}),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    for method in [Method::GET, Method::DELETE] {
        let (status, _) =
            api_json_request(router.clone(), &agent, method, "/v1/online/session".into()).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    let (status, value) =
        api_json_request(router, &human, Method::GET, "/v1/online/session".into()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value, json!({"state":"signed_out"}));
}

#[tokio::test]
async fn online_identity_is_actor_scoped_and_never_replaced_by_a_concurrent_login() {
    let database = TestDatabase::create().await;
    let db =
        choruz_application::DbService::new(choruz_store::EventStore::new(&database.database_url));
    let a = db
        .create_human_user("online-a", "test-password-a")
        .await
        .unwrap();
    let b = db
        .create_human_user("online-b", "test-password-b")
        .await
        .unwrap();
    let identity = OnlineIdentity {
        account_id: "cloud-account".into(),
        device_id: "cloud-device".into(),
        service_url: "https://online.test".into(),
        session_token: "never-log-this-session".into(),
        display_name: "Online A".into(),
    };
    db.connect_online_identity(&a, &identity).await.unwrap();
    assert!(db.online_identity(&b).await.unwrap().is_none());
    assert_eq!(
        db.online_identity(&a).await.unwrap().unwrap().account_id,
        identity.account_id
    );
    assert!(matches!(
        db.connect_online_identity(&a, &identity).await,
        Err(AppError::Conflict(_))
    ));
    db.disconnect_online_identity(&b, &identity.device_id)
        .await
        .unwrap();
    assert!(db.online_identity(&a).await.unwrap().is_some());
    db.disconnect_online_identity(&a, "outdated-device")
        .await
        .unwrap();
    assert!(db.online_identity(&a).await.unwrap().is_some());
    db.disconnect_online_identity(&a, &identity.device_id)
        .await
        .unwrap();
    assert!(db.online_identity(&a).await.unwrap().is_none());
    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let rows = client.query("SELECT action,metadata::text FROM audit_log WHERE actor_id=$1 AND action LIKE 'online.%' ORDER BY created_at", &[&a.id]).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get::<_, String>(0), "online.signed_in");
    assert_eq!(rows[1].get::<_, String>(0), "online.signed_out");
    for row in rows {
        assert!(!row.get::<_, String>(1).contains(&identity.session_token));
    }
}
