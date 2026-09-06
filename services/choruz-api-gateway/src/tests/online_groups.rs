use super::*;
use choruz_application::db_service::{OnlineGroupLink, OnlineIdentity};

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
    let conversation = db
        .create_group(choruz_application::CreateGroupRequest {
            actor_id: owner.id.clone(),
            name: "Shared group".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![],
            workspace_id: None,
        })
        .await
        .unwrap();
    let channel = choruz_common::new_id();
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
    db.save_online_batch(&guest, &joined.id, &history)
        .await
        .unwrap();
    assert_eq!(
        db.online_messages(&guest, &joined.id, 0).await.unwrap()["messages"]
            .as_array()
            .unwrap()
            .len(),
        1
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
    assert_eq!(syncs[0].1["after"], 7);
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
