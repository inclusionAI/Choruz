use super::*;
use futures_util::FutureExt;
use tokio::{net::TcpListener, time::timeout};
use tokio_tungstenite::{WebSocketStream, accept_async};

type Peer = WebSocketStream<tokio::net::TcpStream>;

async fn next_text(peer: &mut Peer) -> Value {
    loop {
        match timeout(Duration::from_secs(3), peer.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
        {
            Message::Text(text) => return serde_json::from_str(&text).unwrap(),
            Message::Ping(bytes) => peer.send(Message::Pong(bytes)).await.unwrap(),
            other => panic!("unexpected frame: {other:?}"),
        }
    }
}

async fn advance_heartbeat(seconds: u64) {
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(seconds)).await;
    tokio::time::resume();
}

async fn answer_probe(peer: &mut Peer) {
    let probe = next_text(peer).await;
    assert_eq!(probe["type"], "gateway.ping");
    peer.send(Message::Text(
        json!({"type": "gateway.pong", "nonce": probe["nonce"]})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
}

// An encrypted rejected request is an acknowledged read barrier on transport:
// the production executor replies without contacting an external HTTP service.
async fn transport_barrier(peer: &mut Peer, key: &str) {
    let frame = encrypt_envelope(
        key,
        &json!({"kind": "http.request", "payload": {
            "request_id": "barrier", "method": "GET", "path": "https://forbidden.example/"
        }}),
    )
    .unwrap();
    peer.send(Message::Text(frame.to_string().into()))
        .await
        .unwrap();
    let response = decrypt_envelope(key, &next_text(peer).await).unwrap();
    assert_eq!(response["kind"], "http.response");
    assert_eq!(response["payload"]["request_id"], "barrier");
    for _ in 0..response["payload"]["body_chunks"].as_u64().unwrap_or(0) {
        let body = decrypt_envelope(key, &next_text(peer).await).unwrap();
        assert_eq!(body["kind"], "http.body");
        assert_eq!(body["payload"]["request_id"], "barrier");
    }
}

async fn rendezvous_barrier(peer: &mut Peer) {
    peer.send(Message::Text(
        json!({"type": "gateway.peer_joined", "role": "device", "device_id": "owned-device"})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let offer = next_text(peer).await;
    assert_eq!(offer["kind"], "session.offer");
    assert_eq!(offer["target_device_id"], "owned-device");
}

async fn exercise_sockets(silent: Option<&str>) {
    let database = crate::tests::TestDatabase::create().await;
    let store = choruz_store::EventStore::new(&database.database_url);
    let db = choruz_application::DbService::new(store.clone());
    let principal = db
        .create_human_user("bridge-owner", "test-password-123")
        .await
        .unwrap();
    let app = choruz_application::ChatApp::new();
    let attachments = tempfile::tempdir().unwrap();
    let state = ApiState {
        experience_worker: None,
        app: app.clone(),
        db: db.clone(),
        runtime: choruz_agent_runtime::RuntimeStore::new(&database.database_url),
        session: choruz_session::PgSessionStore::new(&database.database_url),
        attachments: crate::attachments::AttachmentStore::new(attachments.path(), store.clone()),
        auth: crate::LocalAuthConfig::new(
            "test-session-secret".into(),
            "test-password".into(),
            "test-workspace".into(),
            "test-user".into(),
            1,
        ),
        sync_wakeups: crate::sync_wakeup::SyncWakeupHub::spawn(database.database_url.clone()),
        remote_control_bridges: RemoteControlBridgeHub::new().0,
        local_host: crate::host_runtime::LocalHost::new(),
        host_links: crate::host_link::HostLinkHub::default(),
        online: crate::online_groups::OnlineHub::spawn(app, db),
        event_store: store,
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let material = BridgeMaterial {
        gateway_url: format!("http://{}", listener.local_addr().unwrap()),
        session_key: URL_SAFE_NO_PAD.encode([7u8; 32]),
        host_session_ticket: "test-control".into(),
        host_transport_ticket: "test-transport".into(),
        revoked_device_ids: vec![],
        transport_session_id: "owned-session".into(),
        device_transport_tickets: HashMap::from([("owned-device".into(), "test-device".into())]),
    };
    let bridge_state = state.clone();
    let bridge_material = material.clone();
    let mut task = tokio::spawn(async move {
        serve_bridge_session(&bridge_state, &principal.id, &bridge_material).await
    });
    let result = std::panic::AssertUnwindSafe(async {
        let mut transport = accept_async(listener.accept().await.unwrap().0)
            .await
            .unwrap();
        let mut control = accept_async(listener.accept().await.unwrap().0)
            .await
            .unwrap();
        assert_eq!(
            next_text(&mut control).await["type"],
            "gateway.sync_revocations"
        );
        assert_eq!(next_text(&mut control).await["kind"], "session.offer");
        rendezvous_barrier(&mut control).await;
        transport_barrier(&mut transport, &material.session_key).await;
        for _ in 0..2 {
            advance_heartbeat(25).await;
            if silent == Some("transport") {
                assert_eq!(next_text(&mut transport).await["type"], "gateway.ping");
            } else {
                answer_probe(&mut transport).await;
                transport_barrier(&mut transport, &material.session_key).await;
            }
            if silent == Some("rendezvous") {
                assert_eq!(next_text(&mut control).await["type"], "gateway.ping");
            } else {
                answer_probe(&mut control).await;
                rendezvous_barrier(&mut control).await;
            }
        }
        advance_heartbeat(50).await;
        if let Some(socket) = silent {
            let outcome = timeout(Duration::from_secs(3), &mut task)
                .await
                .expect("silent socket must end session")
                .unwrap();
            assert_eq!(
                outcome.unwrap_err(),
                format!("{socket} peer stopped responding")
            );
        } else {
            answer_probe(&mut transport).await;
            transport_barrier(&mut transport, &material.session_key).await;
            answer_probe(&mut control).await;
            rendezvous_barrier(&mut control).await;
            control.close(None).await.unwrap();
            assert_eq!(
                timeout(Duration::from_secs(3), &mut task)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap_err(),
                "rendezvous closed"
            );
        }
    })
    .catch_unwind()
    .await;
    if !task.is_finished() {
        task.abort();
        let _ = task.await;
    }
    drop(state);
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[tokio::test]
async fn silent_control_expires_while_transport_remains_responsive() {
    exercise_sockets(Some("rendezvous")).await;
}

#[tokio::test]
async fn silent_transport_expires_while_control_remains_responsive() {
    exercise_sockets(Some("transport")).await;
}

#[tokio::test]
async fn responsive_sockets_keep_offering_and_executing_until_control_closes() {
    exercise_sockets(None).await;
}
