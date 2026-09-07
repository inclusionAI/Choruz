use super::*;
use choruz_application::db_service::{OnlineGroupLink, OnlineIdentity};

#[tokio::test]
async fn online_guest_agent_crosses_two_stores_without_granting_runtime_authority() {
    guest_agent_roundtrip(false).await;
}

#[tokio::test]
async fn online_company_agent_keeps_its_workspace_through_the_roundtrip() {
    guest_agent_roundtrip(true).await;
}

async fn guest_agent_roundtrip(company_agent: bool) {
    let a = TestDatabase::create().await;
    let b = TestDatabase::create().await;
    let host_db =
        choruz_application::DbService::new(choruz_store::EventStore::new(&a.database_url));
    let guest_db =
        choruz_application::DbService::new(choruz_store::EventStore::new(&b.database_url));
    let owner = host_db
        .create_human_user("host", "test-password")
        .await
        .unwrap();
    let guest = guest_db
        .create_human_user("guest", "test-password")
        .await
        .unwrap();
    for (db, actor) in [(&host_db, &owner), (&guest_db, &guest)] {
        db.connect_online_identity(
            actor,
            &OnlineIdentity {
                account_id: actor.id.clone(),
                device_id: choruz_common::new_id(),
                service_url: "https://online.test".into(),
                session_token: "private-session".into(),
                display_name: actor.name.clone(),
            },
        )
        .await
        .unwrap();
    }
    let group = host_db
        .create_group(CreateGroupRequest {
            actor_id: owner.id.clone(),
            name: "Shared group".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![],
            workspace_id: None,
        })
        .await
        .unwrap();
    let host = OnlineGroupLink {
        id: choruz_common::new_id(),
        channel_id: choruz_common::new_id(),
        principal_id: owner.id.clone(),
        workspace_id: owner.workspace_id.clone(),
        account_id: owner.id.clone(),
        peer_account_id: Some(guest.id.clone()),
        role: "host".into(),
        conversation_id: group.id.clone(),
        peer_principal_id: Some(choruz_common::new_id()),
        encryption_key: "private-key".into(),
        name: "Shared group".into(),
        status: "pending".into(),
        last_seq: 0,
    };
    let joined = OnlineGroupLink {
        id: choruz_common::new_id(),
        principal_id: guest.id.clone(),
        workspace_id: guest.workspace_id.clone(),
        account_id: guest.id.clone(),
        peer_account_id: Some(owner.id.clone()),
        role: "guest".into(),
        peer_principal_id: None,
        ..host.clone()
    };
    host_db.save_online_group(&owner, &host).await.unwrap();
    guest_db.save_online_group(&guest, &joined).await.unwrap();
    host_db
        .activate_online_guest(&owner, &host.id, "Guest")
        .await
        .unwrap();
    guest_db
        .save_online_batch(
            &guest,
            &joined.id,
            &json!({"kind":"welcome","name":"Shared group"}),
        )
        .await
        .unwrap();
    let app = choruz_application::ChatApp::new();
    let router = router_with_db(app.clone(), &b.database_url);
    let workspace = if company_agent {
        guest_db
            .create_company(CreateCompanyRequest {
                actor_id: guest.id.clone(),
                name: "Guest company".into(),
                slug: None,
                description: None,
                folder_path: None,
            })
            .await
            .unwrap()
            .id
    } else {
        guest.workspace_id.clone()
    };
    let (status, created) = api_json_payload_request(
        router.clone(),
        &guest,
        Method::POST,
        "/v1/agents".into(),
        json!({"actor_id":guest.id,"name":"B assistant","scopes":[],"workspace_id":workspace}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let agent_id = created["principal"]["id"].as_str().unwrap().to_owned();
    let agent = guest_db.get_principal(&agent_id).await.unwrap();
    let direct = guest_db
        .create_direct_conversation(CreateDirectConversationRequest {
            actor_id: guest.id.clone(),
            peer_principal_id: agent_id.clone(),
            workspace_id: Some(workspace.clone()),
        })
        .await
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    RuntimeStore::new(&b.database_url)
        .create_binding(CreateBindingInput {
            conversation_id: direct.id,
            agent_principal_id: agent_id.clone(),
            driver_type: DriverType::ClaudeTerminal,
            workspace_path: root.path().to_str().unwrap().into(),
            git_worktree_path: None,
            config_json: json!({"harness_account_name":"B account"}),
            audit_actor: None,
        })
        .await
        .unwrap();
    let endpoint = format!("/v1/online/groups/{}/agents", joined.id);
    let (status, _) = api_json_payload_request(
        router.clone(),
        &agent,
        Method::POST,
        endpoint.clone(),
        json!({"agent_id":agent_id}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let outsider = guest_db
        .create_human_user("unrelated-user", "test-password")
        .await
        .unwrap();
    let (status, outside) = api_json_payload_request(
        router.clone(),
        &outsider,
        Method::POST,
        "/v1/agents".into(),
        json!({"actor_id":outsider.id,"name":"Outside Agent","scopes":[]}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let outside_id = outside["principal"]["id"].as_str().unwrap();
    let outside_direct = guest_db
        .create_direct_conversation(CreateDirectConversationRequest {
            actor_id: outsider.id.clone(),
            peer_principal_id: outside_id.into(),
            workspace_id: None,
        })
        .await
        .unwrap();
    RuntimeStore::new(&b.database_url)
        .create_binding(CreateBindingInput {
            conversation_id: outside_direct.id,
            agent_principal_id: outside_id.into(),
            driver_type: DriverType::ClaudeTerminal,
            workspace_path: root.path().to_str().unwrap().into(),
            git_worktree_path: None,
            config_json: json!({}),
            audit_actor: None,
        })
        .await
        .unwrap();
    assert!(
        !guest_db
            .online_agents(&guest, &joined.id)
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == outside_id)
    );
    let (status, _) = api_json_payload_request(
        router.clone(),
        &guest,
        Method::POST,
        endpoint.clone(),
        json!({"agent_id":outside_id}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = api_json_payload_request(
        router.clone(),
        &guest,
        Method::POST,
        endpoint.clone(),
        json!({"agent_id":owner.id}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = api_json_payload_request(
        router.clone(),
        &guest,
        Method::POST,
        endpoint.clone(),
        json!({"agent_id":agent_id}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(
        guest_db.online_agents(&guest, &joined.id).await.unwrap()[0]["status"],
        "pending"
    );
    let (delivery, registration) = guest_db
        .online_outgoing(&guest, &joined.id)
        .await
        .unwrap()
        .into_iter()
        .find(|(_, b)| b["kind"] == "agent_join")
        .unwrap();
    let current = host_db.online_group(&owner, &host.id).await.unwrap();
    // Replace only mailbox delivery here; exercise the real protocol receiver and two independent stores.
    crate::online_groups::process(&app, &host_db, &owner, &current, &delivery, &registration)
        .await
        .unwrap();
    let (ack_id, ack) = host_db
        .online_outgoing(&owner, &host.id)
        .await
        .unwrap()
        .into_iter()
        .find(|(_, b)| b["kind"] == "agent_joined")
        .unwrap();
    assert_eq!(ack_id.len(), 36, "Gateway shipment IDs must be UUIDs");
    assert!(ack_id.bytes().all(|c| c.is_ascii_hexdigit() || c == b'-'));
    guest_db
        .confirm_online_agent(&guest, &joined.id, &ack)
        .await
        .unwrap();
    assert_eq!(
        guest_db.online_agents(&guest, &joined.id).await.unwrap()[0]["status"],
        "active"
    );
    let (proxy, _) = host_db
        .online_agent_sender(&owner, &host.id, &agent_id)
        .await
        .unwrap();
    assert_ne!(proxy, agent_id);
    assert!(
        host_db
            .get_principal(&proxy)
            .await
            .unwrap()
            .secret_hash
            .is_none()
    );
    assert!(host_db.get_principal(&agent_id).await.is_err());
    assert!(
        RuntimeStore::new(&a.database_url)
            .list_bindings()
            .await
            .unwrap()
            .is_empty()
    );
    let visible = guest_db.list_conversations(&guest.id).await.unwrap();
    assert!(visible.iter().all(|c| !c.id.starts_with("online-group:")));
    let companion = if company_agent {
        // A second projection in the same link proves cursor progress does not consume another workspace's input.
        let (status, created) = api_json_payload_request(
            router.clone(),
            &guest,
            Method::POST,
            "/v1/agents".into(),
            json!({"actor_id":guest.id,"name":"Z personal assistant","scopes":[]}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let id = created["principal"]["id"].as_str().unwrap().to_owned();
        let direct = guest_db
            .create_direct_conversation(CreateDirectConversationRequest {
                actor_id: guest.id.clone(),
                peer_principal_id: id.clone(),
                workspace_id: None,
            })
            .await
            .unwrap();
        RuntimeStore::new(&b.database_url)
            .create_binding(CreateBindingInput {
                conversation_id: direct.id,
                agent_principal_id: id.clone(),
                driver_type: DriverType::ClaudeTerminal,
                workspace_path: root.path().to_str().unwrap().into(),
                git_worktree_path: None,
                config_json: json!({}),
                audit_actor: None,
            })
            .await
            .unwrap();
        guest_db
            .add_online_agent(&guest, &joined.id, &id)
            .await
            .unwrap();
        let (delivery, registration) = guest_db
            .online_outgoing(&guest, &joined.id)
            .await
            .unwrap()
            .into_iter()
            .find(|(_, v)| v["kind"] == "agent_join" && v["agent"]["id"] == id)
            .unwrap();
        crate::online_groups::process(&app, &host_db, &owner, &current, &delivery, &registration)
            .await
            .unwrap();
        let (_, ack) = host_db
            .online_outgoing(&owner, &host.id)
            .await
            .unwrap()
            .into_iter()
            .find(|(_, v)| v["kind"] == "agent_joined" && v["agent_id"] == id)
            .unwrap();
        guest_db
            .confirm_online_agent(&guest, &joined.id, &ack)
            .await
            .unwrap();
        Some(id)
    } else {
        None
    };
    for (id, _) in host_db.online_outgoing(&owner, &host.id).await.unwrap() {
        host_db.online_accepted(&owner, &id).await.unwrap();
    }
    let message = crate::handlers_messages::publish_message(
        &app,
        &host_db,
        SendMessageRequest {
            actor_id: owner.id.clone(),
            conversation_id: group.id.clone(),
            idempotency_key: choruz_common::new_id(),
            content: "@B assistant Please reply".into(),
            content_type: "text".into(),
            metadata: json!({}),
            trace_id: None,
        },
    )
    .await
    .unwrap();
    host_db
        .queue_online_history(&owner, &host.id)
        .await
        .unwrap();
    let (_, batch) = host_db
        .online_outgoing(&owner, &host.id)
        .await
        .unwrap()
        .into_iter()
        .find(|(_, b)| b["kind"] == "batch")
        .unwrap();
    guest_db
        .save_online_batch(&guest, &joined.id, &batch)
        .await
        .unwrap();
    let input = guest_db
        .next_online_agent_input(&guest, &joined.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(input.content, message.content);
    let mirror_id = input.conversation_id.clone();
    assert_eq!(
        guest_db
            .get_conversation(&mirror_id)
            .await
            .unwrap()
            .workspace_id,
        workspace
    );
    assert_eq!(
        guest_db
            .get_principal(&input.actor_id)
            .await
            .unwrap()
            .workspace_id,
        workspace
    );
    let seq = input.metadata["online_seq"].as_i64().unwrap();
    let delivered = crate::handlers_messages::publish_message(&app, &guest_db, input.clone())
        .await
        .unwrap();
    let retry = crate::handlers_messages::publish_message(&app, &guest_db, input)
        .await
        .unwrap();
    assert_eq!(delivered.id, retry.id);
    guest_db
        .finish_online_agent_input(&guest, &joined.id, &mirror_id, seq)
        .await
        .unwrap();
    let companion_mirror = if companion.is_some() {
        let other = guest_db
            .next_online_agent_input(&guest, &joined.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(other.content, message.content);
        assert_ne!(other.conversation_id, mirror_id);
        assert_eq!(
            guest_db
                .get_conversation(&other.conversation_id)
                .await
                .unwrap()
                .workspace_id,
            guest.workspace_id
        );
        let result = crate::handlers_messages::publish_message(&app, &guest_db, other)
            .await
            .unwrap();
        guest_db
            .finish_online_agent_input(&guest, &joined.id, &result.conversation_id, seq)
            .await
            .unwrap();
        Some(result.conversation_id)
    } else {
        None
    };
    assert!(
        guest_db
            .next_online_agent_input(&guest, &joined.id)
            .await
            .unwrap()
            .is_none()
    );
    // The runtime result boundary is deterministic here; the live two-device smoke owns real CLI execution.
    guest_db
        .send_message(SendMessageRequest {
            actor_id: agent_id.clone(),
            conversation_id: mirror_id.clone(),
            idempotency_key: choruz_common::new_id(),
            content: "B response".into(),
            content_type: "text".into(),
            metadata: json!({}),
            trace_id: None,
        })
        .await
        .unwrap();
    guest_db
        .queue_online_agent_replies(&guest, &joined.id)
        .await
        .unwrap();
    guest_db
        .queue_online_agent_replies(&guest, &joined.id)
        .await
        .unwrap();
    let replies: Vec<_> = guest_db
        .online_outgoing(&guest, &joined.id)
        .await
        .unwrap()
        .into_iter()
        .filter(|(_, b)| b["kind"] == "agent_say")
        .collect();
    assert_eq!(replies.len(), 1);
    if let (Some(companion), Some(conversation_id)) = (companion, companion_mirror) {
        guest_db
            .send_message(SendMessageRequest {
                actor_id: companion.clone(),
                conversation_id,
                idempotency_key: choruz_common::new_id(),
                content: "Personal workspace reply".into(),
                content_type: "text".into(),
                metadata: json!({}),
                trace_id: None,
            })
            .await
            .unwrap();
        guest_db
            .queue_online_agent_replies(&guest, &joined.id)
            .await
            .unwrap();
        assert!(
            guest_db
                .online_outgoing(&guest, &joined.id)
                .await
                .unwrap()
                .iter()
                .any(|(_, v)| v["kind"] == "agent_say" && v["agent_id"] == companion)
        );
    }
    assert_eq!(replies[0].0.len(), 36, "Gateway shipment IDs must be UUIDs");
    assert!(
        replies[0]
            .0
            .bytes()
            .all(|c| c.is_ascii_hexdigit() || c == b'-')
    );
    let (id, body) = &replies[0];
    crate::online_groups::process(&app, &host_db, &owner, &current, id, body)
        .await
        .unwrap();
    crate::online_groups::process(&app, &host_db, &owner, &current, id, body)
        .await
        .unwrap();
    let history = host_db.list_messages(&group.id, None, None).await.unwrap();
    let replies: Vec<_> = history
        .iter()
        .filter(|m| m.content == "B response")
        .collect();
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].sender_id, proxy);
    assert_eq!(
        replies[0].metadata["online_author_context"]["owner_name"],
        "guest"
    );
    assert_eq!(
        replies[0].metadata["online_author_context"]["account_name"],
        "B account"
    );
    assert_eq!(replies[0].metadata["runtime_host_name"], "Guest's device");
    host_db
        .queue_online_history(&owner, &host.id)
        .await
        .unwrap();
    for (_, body) in host_db.online_outgoing(&owner, &host.id).await.unwrap() {
        if body["kind"] == "batch" {
            guest_db
                .save_online_batch(&guest, &joined.id, &body)
                .await
                .unwrap();
        }
    }
    assert!(
        guest_db
            .next_online_agent_input(&guest, &joined.id)
            .await
            .unwrap()
            .is_none(),
        "Canonical echo of our Agent must not dispatch another local turn in any workspace"
    );
    let before = guest_db.online_outgoing(&guest, &joined.id).await.unwrap();
    guest_db
        .queue_online_agent_replies(&guest, &joined.id)
        .await
        .unwrap();
    assert_eq!(
        guest_db.online_outgoing(&guest, &joined.id).await.unwrap(),
        before
    );
    let forged = json!({"kind":"agent_say","agent_id":owner.id,"content":"forged"});
    assert!(
        crate::online_groups::process(
            &app,
            &host_db,
            &owner,
            &current,
            &choruz_common::new_id(),
            &forged
        )
        .await
        .is_err()
    );
    let (status, _) = api_json_request(
        router.clone(),
        &guest,
        Method::DELETE,
        format!("{endpoint}/{agent_id}"),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (late_status, _) = api_json_payload_request(
        router,
        &agent,
        Method::POST,
        "/v1/messages".into(),
        json!({"actor_id":agent_id,"conversation_id":mirror_id,"content":"Late reply after removal","content_type":"text","metadata":{},"idempotency_key":choruz_common::new_id()}),
    ).await;
    assert_eq!(
        late_status,
        StatusCode::FORBIDDEN,
        "Removed Agent must not auto-rejoin through message publication"
    );
    host_db
        .accept_online_agent_leave(&owner, &host.id, &agent_id, 2)
        .await
        .unwrap();
    assert!(
        host_db
            .online_agent_sender(&owner, &host.id, &agent_id)
            .await
            .is_err()
    );
    assert!(
        !host_db
            .get_conversation(&group.id)
            .await
            .unwrap()
            .members
            .contains_key(&proxy)
    );
    assert_eq!(
        guest_db.online_agents(&guest, &joined.id).await.unwrap()[0]["status"],
        "removed"
    );
    // Reordered delivery must not resurrect a removed registration on either device.
    host_db
        .accept_online_agent(&owner, &host.id, &delivery, &registration)
        .await
        .unwrap();
    guest_db
        .confirm_online_agent(&guest, &joined.id, &ack)
        .await
        .unwrap();
    assert!(
        host_db
            .online_agent_sender(&owner, &host.id, &agent_id)
            .await
            .is_err()
    );
    assert_eq!(
        guest_db.online_agents(&guest, &joined.id).await.unwrap()[0]["status"],
        "removed"
    );
    guest_db
        .add_online_agent(&guest, &joined.id, &agent_id)
        .await
        .unwrap();
    guest_db
        .confirm_online_agent(&guest, &joined.id, &ack)
        .await
        .unwrap();
    assert_eq!(
        guest_db.online_agents(&guest, &joined.id).await.unwrap()[0]["status"],
        "pending"
    );
    host_db.revoke_online_group(&owner, &host.id).await.unwrap();
    assert!(
        host_db
            .accept_online_agent(&owner, &host.id, &delivery, &registration)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn online_group_store_preserves_delivery_identity_and_revokes_only_the_invited_group() {
    let database = TestDatabase::create().await;
    let store = choruz_store::EventStore::new(&database.database_url);
    let db = choruz_application::DbService::new(store.clone());
    let owner = db
        .create_human_user("online-owner", "owner-password")
        .await
        .unwrap();
    let guest = db
        .create_human_user("online-guest", "guest-password")
        .await
        .unwrap();
    for actor in [&owner, &guest] {
        db.connect_online_identity(
            actor,
            &OnlineIdentity {
                account_id: actor.id.clone(),
                device_id: choruz_common::new_id(),
                service_url: "https://online.test".into(),
                session_token: "internal-test-session".into(),
                display_name: actor.name.clone(),
            },
        )
        .await
        .unwrap();
    }
    let agent = db
        .create_agent(choruz_application::CreateAgentRequest {
            actor_id: owner.id.clone(),
            name: "Shared assistant".into(),
            scopes: vec!["messages:write".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .await
        .unwrap()
        .principal;
    let conversation = db
        .create_group(choruz_application::CreateGroupRequest {
            actor_id: owner.id.clone(),
            name: "Shared group".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![agent.id.clone()],
            workspace_id: None,
        })
        .await
        .unwrap();
    let channel = choruz_common::new_id();
    let profile = tempfile::tempdir().unwrap();
    choruz_agent_runtime::RuntimeStore::new(&database.database_url).create_binding(choruz_agent_runtime::CreateBindingInput {
        conversation_id: conversation.id.clone(), agent_principal_id: agent.id.clone(), driver_type: choruz_agent_runtime::DriverType::ClaudeTerminal,
        workspace_path: profile.path().to_str().unwrap().into(), git_worktree_path: None,
        config_json: json!({"harness_account_name":"Research account", "private_token":"must-not-export", "harness_account_home":"/private/profile"}), audit_actor: None,
    }).await.unwrap();
    let host = OnlineGroupLink {
        id: choruz_common::new_id(),
        channel_id: channel.clone(),
        principal_id: owner.id.clone(),
        workspace_id: owner.workspace_id.clone(),
        account_id: owner.id.clone(),
        peer_account_id: Some(guest.id.clone()),
        role: "host".into(),
        conversation_id: conversation.id.clone(),
        peer_principal_id: Some(choruz_common::new_id()),
        encryption_key: "private-key".into(),
        name: "Shared group".into(),
        status: "pending".into(),
        last_seq: 0,
    };
    db.save_online_group(&owner, &host).await.unwrap();
    assert!(db.online_group(&guest, &host.id).await.is_err());
    assert!(!host.summary().to_string().contains("private-key"));
    let joined = OnlineGroupLink {
        id: choruz_common::new_id(),
        channel_id: channel,
        principal_id: guest.id.clone(),
        workspace_id: guest.workspace_id.clone(),
        account_id: guest.id.clone(),
        peer_account_id: Some(owner.id.clone()),
        role: "guest".into(),
        peer_principal_id: None,
        ..host.clone()
    };
    db.save_online_group(&guest, &joined).await.unwrap();
    assert!(
        db.queue_online_text(&guest, &joined.id, &choruz_common::new_id(), "Too early")
            .await
            .is_err()
    );
    db.activate_online_guest(&owner, &host.id, &owner.name)
        .await
        .unwrap();
    let another = OnlineGroupLink {
        id: choruz_common::new_id(),
        channel_id: choruz_common::new_id(),
        peer_account_id: Some(choruz_common::new_id()),
        peer_principal_id: Some(choruz_common::new_id()),
        ..host.clone()
    };
    db.save_online_group(&owner, &another).await.unwrap();
    db.activate_online_guest(&owner, &another.id, &owner.name)
        .await
        .unwrap();
    assert_eq!(
        db.find_human_by_username(&owner.name)
            .await
            .unwrap()
            .unwrap()
            .id,
        owner.id
    );
    let peer = db
        .get_principal(host.peer_principal_id.as_ref().unwrap())
        .await
        .unwrap();
    assert!(peer.secret_hash.is_none());
    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .unwrap();
    let connection_task = tokio::spawn(connection);
    let credential_change = client
        .execute(
            "UPDATE principal SET secret_hash='forbidden-test-value' WHERE id=$1",
            &[&peer.id],
        )
        .await;
    assert_eq!(
        credential_change
            .unwrap_err()
            .as_db_error()
            .unwrap()
            .constraint(),
        Some("online_guest_has_no_login")
    );
    drop(client);
    connection_task.await.unwrap().unwrap();
    let conv = db.get_conversation(&conversation.id).await.unwrap();
    assert!(conv.members.contains_key(&peer.id));
    db.save_online_batch(
        &guest,
        &joined.id,
        &json!({"kind":"welcome","name":"Shared group"}),
    )
    .await
    .unwrap();
    let history = json!({"kind":"batch","messages":[{"id":choruz_common::new_id(),"seq":7,"sender_id":owner.id,"sender_name":"Owner","agent":false,"own":false,"content":"Retained history","created_at":"2026-09-01T00:00:00Z"}]});
    db.save_online_batch(&guest, &joined.id, &history)
        .await
        .unwrap();
    let summaries = db.online_group_summaries(&guest).await.unwrap();
    assert_eq!(summaries[0]["message_count"], 1);
    assert_eq!(summaries[0]["latest_seq"], 7);
    assert!(!json!(summaries).to_string().contains("private-key"));
    let mut context_history = history.clone();
    context_history["messages"][0]["id"] = json!(choruz_common::new_id());
    context_history["messages"][0]["seq"] = json!(8);
    context_history["messages"][0]["author_context"] = json!({"owner_name":"Owner","device_name":"Build server","account_name":"Work account","harness":"claude_terminal"});
    db.save_online_batch(&guest, &joined.id, &context_history)
        .await
        .unwrap();
    let received = db.online_messages(&guest, &joined.id, 7).await.unwrap();
    assert_eq!(
        received["messages"][0]["author_context"],
        context_history["messages"][0]["author_context"]
    );
    let mut invalid_context = context_history.clone();
    invalid_context["messages"][0]["author_context"]["token"] = json!("must-never-share");
    assert!(
        db.save_online_batch(&guest, &joined.id, &invalid_context)
            .await
            .is_err()
    );
    for (id, _) in db.online_outgoing(&owner, &host.id).await.unwrap() {
        db.online_accepted(&owner, &id).await.unwrap();
    }
    db.send_message(choruz_application::SendMessageRequest {
        actor_id: agent.id.clone(),
        conversation_id: conversation.id.clone(),
        idempotency_key: choruz_common::new_id(),
        content: "Shared assistant reply".into(),
        content_type: "text".into(),
        metadata: json!({"private_metadata":"must-not-export"}),
        trace_id: None,
    })
    .await
    .unwrap();
    db.queue_online_history(&owner, &host.id).await.unwrap();
    let shipments = db.online_outgoing(&owner, &host.id).await.unwrap();
    let projected = &shipments
        .iter()
        .find(|(_, body)| body["kind"] == "batch")
        .unwrap()
        .1["messages"][0];
    assert_eq!(
        projected["author_context"]["account_name"],
        "Research account"
    );
    assert_eq!(projected["author_context"]["harness"], "claude_terminal");
    assert_eq!(
        projected["author_context"]["device_name"],
        "Group owner's device"
    );
    assert_eq!(projected["author_context"]["owner_name"], owner.name);
    assert!(!projected.to_string().contains("must-not-export"));
    assert!(!projected.to_string().contains("/private/profile"));
    invalid_context["messages"][0]["author_context"] = json!({"device_name":"x".repeat(201)});
    assert!(
        db.save_online_batch(&guest, &joined.id, &invalid_context)
            .await
            .is_err()
    );
    db.save_online_batch(&guest, &joined.id, &history)
        .await
        .unwrap();
    assert_eq!(
        db.online_messages(&guest, &joined.id, 0).await.unwrap()["messages"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    db.queue_online_resync(&guest, &joined.id).await.unwrap();
    db.queue_online_resync(&guest, &joined.id).await.unwrap();
    let syncs: Vec<_> = db
        .online_outgoing(&guest, &joined.id)
        .await
        .unwrap()
        .into_iter()
        .filter(|(_, body)| body["kind"] == "sync")
        .collect();
    assert_eq!(syncs.len(), 1);
    assert_eq!(syncs[0].1["after"], 8);
    let id = choruz_common::new_id();
    let body = json!({"kind":"say","content":"Durable text"});
    db.receive_online(&owner, &host.id, &id, &body)
        .await
        .unwrap();
    db.receive_online(&owner, &host.id, &id, &body)
        .await
        .unwrap();
    assert!(
        db.receive_online(
            &owner,
            &host.id,
            &id,
            &json!({"kind":"say","content":"Changed text"})
        )
        .await
        .is_err()
    );
    let restarted = choruz_application::DbService::new(store);
    assert_eq!(
        restarted.online_incoming(&owner, &host.id).await.unwrap(),
        vec![(id.clone(), body)]
    );
    db.online_processed(&owner, &id).await.unwrap();
    assert!(
        restarted
            .online_incoming(&owner, &host.id)
            .await
            .unwrap()
            .is_empty()
    );
    let send_id = choruz_common::new_id();
    let (first, retry) = tokio::join!(
        db.queue_online_text(&guest, &joined.id, &send_id, "A queued message"),
        db.queue_online_text(&guest, &joined.id, &send_id, "A queued message"),
    );
    first.unwrap();
    retry.unwrap();
    assert!(
        db.queue_online_text(&guest, &joined.id, &send_id, "Changed message")
            .await
            .is_err()
    );
    assert_eq!(
        db.online_outgoing(&guest, &joined.id)
            .await
            .unwrap()
            .iter()
            .filter(|(id, _)| id == &send_id)
            .count(),
        1
    );
    db.online_accepted(&owner, &send_id).await.unwrap();
    assert!(
        db.online_outgoing(&guest, &joined.id)
            .await
            .unwrap()
            .iter()
            .any(|(id, _)| id == &send_id)
    );
    db.online_accepted(&guest, &send_id).await.unwrap();
    assert!(
        !db.online_outgoing(&guest, &joined.id)
            .await
            .unwrap()
            .iter()
            .any(|(id, _)| id == &send_id)
    );
    db.revoke_online_group(&owner, &host.id).await.unwrap();
    assert!(
        db.get_conversation(&conversation.id)
            .await
            .unwrap()
            .members
            .contains_key(another.peer_principal_id.as_ref().unwrap())
    );
    assert!(
        !db.get_conversation(&conversation.id)
            .await
            .unwrap()
            .members
            .contains_key(&peer.id)
    );
    assert!(
        db.receive_online(
            &owner,
            &host.id,
            &choruz_common::new_id(),
            &json!({"kind":"say","content":"Denied"})
        )
        .await
        .is_err()
    );
    assert!(
        db.activate_online_guest(&owner, &host.id, "Rejoin denied")
            .await
            .is_err()
    );
    db.revoke_online_group(&guest, &joined.id).await.unwrap();
    assert!(
        db.save_online_batch(&guest, &joined.id, &history)
            .await
            .is_err()
    );
    assert!(
        db.queue_online_text(&guest, &joined.id, &choruz_common::new_id(), "Denied")
            .await
            .is_err()
    );
}
