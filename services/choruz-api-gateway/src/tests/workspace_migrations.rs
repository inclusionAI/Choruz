use super::*;

#[tokio::test]
async fn workspace_and_webhook_mutations_require_a_durable_commit() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let owner = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "commit-source".into(),
            principal_type: PrincipalType::Human,
            name: "Owner".into(),
            avatar_url: None,
        })
        .unwrap();
    let agent = app
        .create_agent(CreateAgentRequest {
            actor_id: owner.id.clone(),
            name: "Bot".into(),
            scopes: vec!["events:read".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let conversation = app
        .create_direct_conversation(CreateDirectConversationRequest {
            actor_id: owner.id.clone(),
            peer_principal_id: agent.id.clone(),
            workspace_id: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &owner).await;
    seed_principal_to_db(&database.database_url, &agent).await;
    seed_conversation_to_db(&database.database_url, &conversation).await;
    let db =
        choruz_application::DbService::new(choruz_store::EventStore::new(&database.database_url));
    let original = db
        .send_message(SendMessageRequest {
            actor_id: owner.id.clone(),
            conversation_id: conversation.id.clone(),
            idempotency_key: "migration-history".into(),
            content: "retained history".into(),
            content_type: "text".into(),
            metadata: json!({}),
            trace_id: None,
        })
        .await
        .unwrap();
    let target = db
        .create_company(CreateCompanyRequest {
            actor_id: owner.id.clone(),
            name: "Destination".into(),
            slug: Some("destination".into()),
            description: None,
            folder_path: None,
        })
        .await
        .unwrap();
    app.inject_company(
        target.clone(),
        db.list_company_members(&target.id).await.unwrap(),
    );
    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .unwrap();
    let connection = tokio::spawn(connection);
    // A real database failure after the mutation has begun must roll back the
    // row, audit and any process-local projection, not return a success response.
    client.batch_execute("CREATE FUNCTION reject_commit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'owned persistence rejection'; END $$;
        CREATE TRIGGER reject_audit BEFORE INSERT ON audit_log FOR EACH ROW EXECUTE FUNCTION reject_commit();
        CREATE TRIGGER reject_webhook BEFORE INSERT OR UPDATE ON event_webhook FOR EACH ROW EXECUTE FUNCTION reject_commit();").await.unwrap();
    let router = router_with_db(app.clone(), &database.database_url);
    for (path, method, body) in [
        (
            format!("/v1/principals/{}/workspace", agent.id),
            Method::PATCH,
            json!({"actor_id":owner.id,"workspace_id":target.id}),
        ),
        (
            format!("/v1/conversations/{}/workspace", conversation.id),
            Method::PATCH,
            json!({"actor_id":owner.id,"workspace_id":target.id}),
        ),
        (
            format!("/v1/principals/{}/event-webhook", agent.id),
            Method::POST,
            json!({"actor_id":owner.id,"url":"http://127.0.0.1:1/hook","event_types":["message.created"],"secret":"owned-secret"}),
        ),
    ] {
        let (status, _) =
            api_json_payload_request(router.clone(), &owner, method, path, body).await;
        assert_eq!(
            status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "a failed commit must not look successful"
        );
        assert_eq!(
            db.get_principal(&agent.id).await.unwrap().workspace_id,
            "commit-source"
        );
        assert_eq!(
            db.get_conversation(&conversation.id)
                .await
                .unwrap()
                .workspace_id,
            "commit-source"
        );
        assert_eq!(
            app.get_principal(&agent.id).unwrap().workspace_id,
            "commit-source"
        );
        assert_eq!(
            app.get_conversation(&conversation.id).unwrap().workspace_id,
            "commit-source"
        );
        assert_eq!(
            client
                .query_one("SELECT COUNT(*) FROM event_webhook", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            0
        );
    }
    client
        .batch_execute(
            "DROP TRIGGER reject_audit ON audit_log; DROP TRIGGER reject_webhook ON event_webhook;",
        )
        .await
        .unwrap();
    // Fresh process state proves these commands read durable identities rather
    // than depending on a successful mutation of the old in-memory shell.
    let restarted = router_with_db(choruz_application::ChatApp::new(), &database.database_url);
    for path in [
        format!("/v1/conversations/{}/workspace", conversation.id),
        format!("/v1/principals/{}/workspace", agent.id),
    ] {
        let (status, body) = api_json_payload_request(
            restarted.clone(),
            &owner,
            Method::PATCH,
            path,
            json!({"actor_id":owner.id,"workspace_id":target.id}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["workspace_id"], target.id);
    }
    assert_eq!(
        db.get_principal(&agent.id).await.unwrap().workspace_id,
        target.id
    );
    assert_eq!(
        db.get_conversation(&conversation.id)
            .await
            .unwrap()
            .workspace_id,
        target.id
    );
    let (status, body) = api_json_payload_request(restarted.clone(),&owner,Method::POST,
        format!("/v1/principals/{}/event-webhook",agent.id),
        json!({"actor_id":owner.id,"url":"http://127.0.0.1:1/hook","event_types":["message.created"],"secret":"owned-secret"})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        client
            .query_one(
                "SELECT webhook_secret FROM event_webhook WHERE principal_id=$1",
                &[&agent.id]
            )
            .await
            .unwrap()
            .get::<_, String>(0),
        "owned-secret"
    );
    let audits = db.list_audit_logs(&owner.workspace_id).await.unwrap();
    assert_eq!(
        audits
            .iter()
            .filter(|a| a.action.ends_with("workspace_migrated"))
            .count(),
        2
    );
    assert_eq!(
        audits
            .iter()
            .filter(|a| a.action == "principal.webhook_configured")
            .count(),
        1
    );
    assert_eq!(
        client
            .query_one(
                "SELECT content FROM conversation_events WHERE event_id=$1 AND conversation_id=$2",
                &[&original.id, &conversation.id]
            )
            .await
            .unwrap()
            .get::<_, String>(0),
        "retained history"
    );
    let outsider = db
        .ensure_local_operator("outside-workspace", "Outsider")
        .await
        .unwrap();
    for path in [
        format!("/v1/conversations/{}/workspace", conversation.id),
        format!("/v1/principals/{}/workspace", agent.id),
    ] {
        let (status, _) = api_json_payload_request(
            restarted.clone(),
            &outsider,
            Method::PATCH,
            path,
            json!({"actor_id":outsider.id,"workspace_id":outsider.workspace_id}),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "migration must check access to the source as well as the destination"
        );
    }
    assert_eq!(
        db.get_conversation(&conversation.id)
            .await
            .unwrap()
            .workspace_id,
        target.id
    );
    assert_eq!(
        db.get_principal(&agent.id).await.unwrap().workspace_id,
        target.id
    );
    drop(restarted);
    drop(router);
    drop(db);
    drop(client);
    connection.await.unwrap().unwrap();
}
