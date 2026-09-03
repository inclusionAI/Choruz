use super::*;

#[tokio::test]
async fn thread_reply_canonicalizes_to_root() {
    // Reply-to-a-threaded-reply must re-point reply_event_id at the
    // thread ROOT (flat threads, Slack semantics) so reads stay
    // single-level. The user-supplied metadata.reply_to_id is preserved
    // as sent; only the column is canonicalized.
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let router = router_with_db(app.clone(), &database.database_url);

    let alice = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-threads-canon".into(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    let bob = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-threads-canon".into(),
            principal_type: PrincipalType::Human,
            name: "Bob".into(),
            avatar_url: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &alice).await;
    seed_principal_to_db(&database.database_url, &bob).await;

    let (group_status, group) = api_json_payload_request(
        router.clone(),
        &alice,
        Method::POST,
        "/v1/groups".into(),
        json!({
            "actor_id": alice.id.clone(),
            "name": "threads-canon-group",
            "description": null,
            "avatar_url": null,
            "member_ids": [bob.id.clone()],
        }),
    )
    .await;
    assert_eq!(group_status, StatusCode::CREATED);
    let conv_id = group["id"].as_str().unwrap();

    // Root message.
    let (root_status, root) = api_send_with_metadata(
        router.clone(),
        &alice,
        conv_id,
        "root message",
        "t-root",
        json!({}),
    )
    .await;
    assert_eq!(root_status, StatusCode::CREATED);
    let root_id = root["id"].as_str().unwrap();

    // First threaded reply → targets the root directly.
    let (r1_status, r1) = api_send_with_metadata(
        router.clone(),
        &bob,
        conv_id,
        "first reply",
        "t-r1",
        json!({"reply_to_id": root_id, "thread": true}),
    )
    .await;
    assert_eq!(r1_status, StatusCode::CREATED, "r1 body: {r1}");
    let r1_id = r1["id"].as_str().unwrap();
    assert_eq!(
        db_reply_event_id(&database.database_url, r1_id)
            .await
            .as_deref(),
        Some(root_id),
        "first reply points at root"
    );

    // Second threaded reply TARGETS THE FIRST REPLY → must canonicalize
    // to the root.
    let (r2_status, r2) = api_send_with_metadata(
        router.clone(),
        &alice,
        conv_id,
        "reply to the reply",
        "t-r2",
        json!({"reply_to_id": r1_id, "thread": true}),
    )
    .await;
    assert_eq!(r2_status, StatusCode::CREATED, "r2 body: {r2}");
    let r2_id = r2["id"].as_str().unwrap();
    assert_eq!(
        db_reply_event_id(&database.database_url, r2_id)
            .await
            .as_deref(),
        Some(root_id),
        "reply-to-reply canonicalizes to the thread root"
    );
    // User-supplied metadata preserved as sent (UI may show which message
    // was actually replied to).
    assert_eq!(r2["metadata"]["reply_to_id"], r1_id);

    // Threading onto a LEGACY QUOTE-REPLY (reply_event_id set, no thread
    // flag): the quote-reply itself acts as the new thread's root — it
    // must NOT be chased up to ITS quote target. This pins the
    // acts-as-root branch of canonicalize_thread_root, which is also the
    // branch a strict-vs-permissive thread-flag divergence would
    // mis-route through.
    let (q_status, q) = api_send_with_metadata(
        router.clone(),
        &bob,
        conv_id,
        "legacy quote of root",
        "t-q1",
        json!({"reply_to_id": root_id}),
    )
    .await;
    assert_eq!(q_status, StatusCode::CREATED);
    let q_id = q["id"].as_str().unwrap();
    let (tq_status, tq) = api_send_with_metadata(
        router.clone(),
        &alice,
        conv_id,
        "thread under the quote-reply",
        "t-q2",
        json!({"reply_to_id": q_id, "thread": true}),
    )
    .await;
    assert_eq!(tq_status, StatusCode::CREATED, "tq body: {tq}");
    let tq_id = tq["id"].as_str().unwrap();
    assert_eq!(
        db_reply_event_id(&database.database_url, tq_id)
            .await
            .as_deref(),
        Some(q_id),
        "threading onto a legacy quote-reply roots on the quote-reply itself, not its quote target"
    );
}

#[tokio::test]
async fn thread_reply_counter_semantics() {
    // Non-broadcast threaded replies must NOT bump
    // conversation.total_msg_count; broadcast replies and normal
    // messages must. (RFC counter table.)
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let router = router_with_db(app.clone(), &database.database_url);

    let alice = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-threads-counter".into(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &alice).await;

    let (_, group) = api_json_payload_request(
        router.clone(),
        &alice,
        Method::POST,
        "/v1/groups".into(),
        json!({
            "actor_id": alice.id.clone(),
            "name": "threads-counter-group",
            "description": null,
            "avatar_url": null,
            "member_ids": [],
        }),
    )
    .await;
    let conv_id = group["id"].as_str().unwrap();

    // Root: bumps 0 → 1.
    let (_, root) =
        api_send_with_metadata(router.clone(), &alice, conv_id, "root", "c-root", json!({})).await;
    let root_id = root["id"].as_str().unwrap();
    assert_eq!(db_total_msg_count(&database.database_url, conv_id).await, 1);

    // Non-broadcast threaded reply: count stays 1.
    let (nb_status, _) = api_send_with_metadata(
        router.clone(),
        &alice,
        conv_id,
        "quiet thread reply",
        "c-r1",
        json!({"reply_to_id": root_id, "thread": true}),
    )
    .await;
    assert_eq!(nb_status, StatusCode::CREATED);
    assert_eq!(
        db_total_msg_count(&database.database_url, conv_id).await,
        1,
        "non-broadcast threaded reply must not bump conversation unread"
    );

    // Broadcast threaded reply: bumps 1 → 2.
    let (b_status, _) = api_send_with_metadata(
        router.clone(),
        &alice,
        conv_id,
        "loud thread reply",
        "c-r2",
        json!({"reply_to_id": root_id, "thread": true, "broadcast": true}),
    )
    .await;
    assert_eq!(b_status, StatusCode::CREATED);
    assert_eq!(
        db_total_msg_count(&database.database_url, conv_id).await,
        2,
        "broadcast threaded reply counts like a normal message"
    );

    // Legacy quote-reply (no thread flag): bumps 2 → 3 (unchanged behavior).
    let (q_status, _) = api_send_with_metadata(
        router.clone(),
        &alice,
        conv_id,
        "legacy quote reply",
        "c-r3",
        json!({"reply_to_id": root_id}),
    )
    .await;
    assert_eq!(q_status, StatusCode::CREATED);
    assert_eq!(
        db_total_msg_count(&database.database_url, conv_id).await,
        3,
        "legacy quote-replies keep today's counter behavior"
    );
}

#[tokio::test]
async fn thread_reply_rejects_bad_targets() {
    // (a) thread=true without reply_to_id → 400.
    // (b) thread target in a different conversation → uniform 404
    //     (indistinguishable from missing — no existence oracle).
    // (c) thread target that does not exist → 404.
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let router = router_with_db(app.clone(), &database.database_url);

    let alice = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-threads-reject".into(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &alice).await;

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
        mk_group("threads-reject-a"),
    )
    .await;
    let (_, group_b) = api_json_payload_request(
        router.clone(),
        &alice,
        Method::POST,
        "/v1/groups".into(),
        mk_group("threads-reject-b"),
    )
    .await;
    let conv_a = group_a["id"].as_str().unwrap();
    let conv_b = group_b["id"].as_str().unwrap();

    // Seed a message in conversation A.
    let (_, a_msg) = api_send_with_metadata(
        router.clone(),
        &alice,
        conv_a,
        "message in A",
        "rej-a",
        json!({}),
    )
    .await;
    let a_msg_id = a_msg["id"].as_str().unwrap();

    // (a) thread without target.
    let (no_target_status, _) = api_send_with_metadata(
        router.clone(),
        &alice,
        conv_a,
        "thread no target",
        "rej-1",
        json!({"thread": true}),
    )
    .await;
    assert_eq!(
        no_target_status,
        StatusCode::BAD_REQUEST,
        "thread=true without reply_to_id must 400"
    );

    // (b) cross-conversation target → uniform 404, indistinguishable
    // from a nonexistent id, so the lookup can't be used as a cross-
    // tenant message-existence oracle.
    let (cross_status, _) = api_send_with_metadata(
        router.clone(),
        &alice,
        conv_b,
        "cross-conv thread",
        "rej-2",
        json!({"reply_to_id": a_msg_id, "thread": true}),
    )
    .await;
    assert_eq!(
        cross_status,
        StatusCode::NOT_FOUND,
        "thread target in another conversation must be uniform 404 (no existence oracle)"
    );

    // (c) nonexistent target.
    let (missing_status, _) = api_send_with_metadata(
        router.clone(),
        &alice,
        conv_a,
        "ghost thread",
        "rej-3",
        json!({"reply_to_id": "does-not-exist", "thread": true}),
    )
    .await;
    assert_eq!(
        missing_status,
        StatusCode::NOT_FOUND,
        "nonexistent thread target must 404"
    );

    // (d) target in a conversation the SENDER cannot access at all (case
    // (b)'s sender was a member of both sides). Bob's private group's
    // message must be exactly as invisible as a nonexistent id.
    let bob = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-threads-reject".into(),
            principal_type: PrincipalType::Human,
            name: "Bob".into(),
            avatar_url: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &bob).await;
    let (_, group_c) = api_json_payload_request(
        router.clone(),
        &bob,
        Method::POST,
        "/v1/groups".into(),
        json!({
            "actor_id": bob.id.clone(),
            "name": "threads-reject-c-private",
            "description": null,
            "avatar_url": null,
            "member_ids": [],
        }),
    )
    .await;
    let conv_c = group_c["id"].as_str().unwrap();
    let (_, c_msg) = api_send_with_metadata(
        router.clone(),
        &bob,
        conv_c,
        "private message in C",
        "rej-c",
        json!({}),
    )
    .await;
    let c_msg_id = c_msg["id"].as_str().unwrap();
    let (inaccessible_status, _) = api_send_with_metadata(
        router.clone(),
        &alice,
        conv_a,
        "thread at inaccessible target",
        "rej-4",
        json!({"reply_to_id": c_msg_id, "thread": true}),
    )
    .await;
    assert_eq!(
        inaccessible_status,
        StatusCode::NOT_FOUND,
        "thread target in a conversation the sender can't access must be uniform 404 (no oracle)"
    );
}

// ─────────────────────────────────────────────────────────────────────
// Message threads — Phase 2 read path (timeline view, thread detail,
// thread receipts + unread)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn timeline_view_filters_quiet_replies_and_rolls_up() {
    // ?view=timeline must (a) hide quiet threaded replies, (b) keep
    // broadcast replies and legacy quote-replies inline, (c) attach a
    // thread_summaries rollup for roots with threaded replies, and
    // (d) leave the default (no view param) flat array unchanged.
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let router = router_with_db(app.clone(), &database.database_url);

    let alice = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-threads-timeline".into(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    let bob = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-threads-timeline".into(),
            principal_type: PrincipalType::Human,
            name: "Bob".into(),
            avatar_url: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &alice).await;
    seed_principal_to_db(&database.database_url, &bob).await;

    let (_, group) = api_json_payload_request(
        router.clone(),
        &alice,
        Method::POST,
        "/v1/groups".into(),
        json!({
            "actor_id": alice.id.clone(),
            "name": "threads-timeline-group",
            "description": null,
            "avatar_url": null,
            "member_ids": [bob.id.clone()],
        }),
    )
    .await;
    let conv_id = group["id"].as_str().unwrap();

    // root + quiet reply + broadcast reply + legacy quote-reply.
    let (_, root) = api_send_with_metadata(
        router.clone(),
        &alice,
        conv_id,
        "root",
        "tl-root",
        json!({}),
    )
    .await;
    let root_id = root["id"].as_str().unwrap();
    let (_, _quiet) = api_send_with_metadata(
        router.clone(),
        &bob,
        conv_id,
        "quiet reply",
        "tl-q",
        json!({"reply_to_id": root_id, "thread": true}),
    )
    .await;
    let (_, broadcast) = api_send_with_metadata(
        router.clone(),
        &bob,
        conv_id,
        "broadcast reply",
        "tl-b",
        json!({"reply_to_id": root_id, "thread": true, "broadcast": true}),
    )
    .await;
    let broadcast_id = broadcast["id"].as_str().unwrap();
    let (_, quote) = api_send_with_metadata(
        router.clone(),
        &alice,
        conv_id,
        "legacy quote",
        "tl-lq",
        json!({"reply_to_id": root_id}),
    )
    .await;
    let quote_id = quote["id"].as_str().unwrap();

    // Timeline view: root + broadcast + quote visible; quiet hidden.
    let (tl_status, tl) = api_json_request(
        router.clone(),
        &alice,
        Method::GET,
        format!(
            "/v1/conversations/{conv_id}/messages?principal_id={}&view=timeline",
            alice.id
        ),
    )
    .await;
    assert_eq!(tl_status, StatusCode::OK, "timeline body: {tl}");
    let tl_ids: Vec<&str> = tl["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert!(tl_ids.contains(&root_id), "root visible");
    assert!(tl_ids.contains(&broadcast_id), "broadcast reply visible");
    assert!(
        tl_ids.contains(&quote_id),
        "legacy quote-reply stays inline"
    );
    assert_eq!(
        tl_ids.len(),
        3,
        "quiet threaded reply hidden from timeline; got {tl_ids:?}"
    );

    // Rollup present for the root: 2 threaded replies (quiet + broadcast).
    let summaries = tl["thread_summaries"].as_array().unwrap();
    let root_summary = summaries
        .iter()
        .find(|s| s["root_event_id"] == root_id)
        .expect("rollup for root");
    assert_eq!(root_summary["reply_count"], 2);
    assert!(
        root_summary["last_reply_at"].is_string(),
        "rollup carries last_reply_at: {root_summary}"
    );
    let sample: Vec<&str> = root_summary["participant_sample"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    // Both replies were sent by bob — the sample holds distinct sender ids
    // (NO ordering contract; see ThreadSummary doc).
    assert_eq!(sample, vec![bob.id.as_str()], "sample: {sample:?}");

    // Default view unchanged: flat array with all 4 messages.
    let (flat_status, flat) = api_list_messages(router.clone(), &alice, conv_id).await;
    assert_eq!(flat_status, StatusCode::OK);
    assert_eq!(
        flat.as_array().unwrap().len(),
        4,
        "default view keeps today's flat shape including quiet replies"
    );
}

#[tokio::test]
async fn thread_detail_and_receipts() {
    // GET threads/{root} returns root + replies in order; POST view
    // upserts the receipt and clears thread_unread_count in /v1/unreads;
    // non-members are rejected; missing roots 404.
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let router = router_with_db(app.clone(), &database.database_url);

    let alice = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-threads-detail".into(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    let bob = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-threads-detail".into(),
            principal_type: PrincipalType::Human,
            name: "Bob".into(),
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
    seed_principal_to_db(&database.database_url, &bob).await;
    seed_principal_to_db(&database.database_url, &outsider).await;

    let (_, group) = api_json_payload_request(
        router.clone(),
        &alice,
        Method::POST,
        "/v1/groups".into(),
        json!({
            "actor_id": alice.id.clone(),
            "name": "threads-detail-group",
            "description": null,
            "avatar_url": null,
            "member_ids": [bob.id.clone()],
        }),
    )
    .await;
    let conv_id = group["id"].as_str().unwrap();

    let (_, root) = api_send_with_metadata(
        router.clone(),
        &alice,
        conv_id,
        "root",
        "td-root",
        json!({}),
    )
    .await;
    let root_id = root["id"].as_str().unwrap();
    for i in 0..3 {
        let (st, _) = api_send_with_metadata(
            router.clone(),
            &alice,
            conv_id,
            &format!("reply {i}"),
            &format!("td-r{i}"),
            json!({"reply_to_id": root_id, "thread": true}),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED);
    }

    // Thread detail: root + 3 replies in seq order.
    let (detail_status, detail) = api_json_request(
        router.clone(),
        &bob,
        Method::GET,
        format!("/v1/conversations/{conv_id}/threads/{root_id}"),
    )
    .await;
    assert_eq!(detail_status, StatusCode::OK, "detail: {detail}");
    assert_eq!(detail["root"]["id"], root_id);
    let replies = detail["replies"].as_array().unwrap();
    assert_eq!(replies.len(), 3);
    let seqs: Vec<i64> = replies
        .iter()
        .map(|r| r["server_seq"].as_i64().unwrap())
        .collect();
    let mut sorted = seqs.clone();
    sorted.sort();
    assert_eq!(seqs, sorted, "replies in ascending seq order");

    // Bob has 1 unread thread (never viewed).
    let (unreads_status, unreads) =
        api_json_request(router.clone(), &bob, Method::GET, "/v1/unreads".into()).await;
    assert_eq!(unreads_status, StatusCode::OK);
    let conv_unread = unreads
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["conversation_id"] == conv_id)
        .expect("conversation in unreads");
    assert_eq!(
        conv_unread["thread_unread_count"], 1,
        "one unread thread before viewing: {conv_unread}"
    );

    // Bob views the thread → unread clears.
    let (view_status, _) = api_json_payload_request(
        router.clone(),
        &bob,
        Method::POST,
        format!("/v1/conversations/{conv_id}/threads/{root_id}/view"),
        json!({}),
    )
    .await;
    assert_eq!(view_status, StatusCode::NO_CONTENT);
    let (_, unreads_after) =
        api_json_request(router.clone(), &bob, Method::GET, "/v1/unreads".into()).await;
    let conv_after = unreads_after
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["conversation_id"] == conv_id)
        .expect("conversation still listed");
    assert_eq!(
        conv_after["thread_unread_count"], 0,
        "thread unread cleared after view: {conv_after}"
    );

    // A new reply re-lights the unread.
    let (st, relight_reply) = api_send_with_metadata(
        router.clone(),
        &alice,
        conv_id,
        "another reply",
        "td-r9",
        json!({"reply_to_id": root_id, "thread": true}),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let relight_reply_id = relight_reply["id"].as_str().unwrap();
    let (_, unreads_relit) =
        api_json_request(router.clone(), &bob, Method::GET, "/v1/unreads".into()).await;
    let conv_relit = unreads_relit
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["conversation_id"] == conv_id)
        .unwrap();
    assert_eq!(conv_relit["thread_unread_count"], 1, "new reply re-lights");

    // Canonicalize-on-view (Gate-4 blind-1 regression): viewing via a
    // REPLY's id (deep-link case, mirroring the GET endpoint) must key
    // the receipt on the canonical ROOT — otherwise the receipt is a
    // dead row and the unread dot never clears.
    let (view_by_reply_status, _) = api_json_payload_request(
        router.clone(),
        &bob,
        Method::POST,
        format!("/v1/conversations/{conv_id}/threads/{relight_reply_id}/view"),
        json!({}),
    )
    .await;
    assert_eq!(view_by_reply_status, StatusCode::NO_CONTENT);
    let (_, unreads_canon) =
        api_json_request(router.clone(), &bob, Method::GET, "/v1/unreads".into()).await;
    let conv_canon = unreads_canon
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["conversation_id"] == conv_id)
        .unwrap();
    assert_eq!(
        conv_canon["thread_unread_count"], 0,
        "view by reply id must clear the thread unread via the canonical root: {conv_canon}"
    );

    // Outsider (workspace member but NOT conversation member) is rejected.
    let (outsider_status, _) = api_json_request(
        router.clone(),
        &outsider,
        Method::GET,
        format!("/v1/conversations/{conv_id}/threads/{root_id}"),
    )
    .await;
    assert_eq!(
        outsider_status,
        StatusCode::FORBIDDEN,
        "thread visibility equals conversation visibility"
    );

    // Missing root → 404.
    let (missing_status, _) = api_json_request(
        router.clone(),
        &bob,
        Method::GET,
        format!("/v1/conversations/{conv_id}/threads/no-such-root"),
    )
    .await;
    assert_eq!(missing_status, StatusCode::NOT_FOUND);

    // Mirror both negative paths on the receipt-WRITE endpoint too, so the
    // auth/error contract can't drift between the two thread surfaces.
    let (outsider_view_status, _) = api_json_payload_request(
        router.clone(),
        &outsider,
        Method::POST,
        format!("/v1/conversations/{conv_id}/threads/{root_id}/view"),
        json!({}),
    )
    .await;
    assert_eq!(
        outsider_view_status,
        StatusCode::FORBIDDEN,
        "view endpoint must enforce the same membership gate as the GET"
    );
    let (missing_view_status, _) = api_json_payload_request(
        router.clone(),
        &bob,
        Method::POST,
        format!("/v1/conversations/{conv_id}/threads/no-such-root/view"),
        json!({}),
    )
    .await;
    assert_eq!(
        missing_view_status,
        StatusCode::NOT_FOUND,
        "view endpoint must 404 unknown roots like the GET"
    );
}

#[tokio::test]
async fn quiet_thread_reply_preserves_sender_unread_state() {
    // Pins the bumps_conversation_unread gate on mark_conversation_viewed
    // (Gate-1 surface-2): a quiet (non-broadcast) threaded reply must NOT
    // clear the sender's PRE-EXISTING main-timeline unread. Reverting the
    // gate to the old unconditional mark_conversation_viewed makes this
    // test fail.
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let router = router_with_db(app.clone(), &database.database_url);

    let alice = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-threads-quiet-unread".into(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    let bob = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-threads-quiet-unread".into(),
            principal_type: PrincipalType::Human,
            name: "Bob".into(),
            avatar_url: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &alice).await;
    seed_principal_to_db(&database.database_url, &bob).await;

    let (_, group) = api_json_payload_request(
        router.clone(),
        &alice,
        Method::POST,
        "/v1/groups".into(),
        json!({
            "actor_id": alice.id.clone(),
            "name": "threads-quiet-unread-group",
            "description": null,
            "avatar_url": null,
            "member_ids": [bob.id.clone()],
        }),
    )
    .await;
    let conv_id = group["id"].as_str().unwrap();

    // Bob posts the root (also gives Alice her pre-existing unread).
    let (_, root) = api_send_with_metadata(
        router.clone(),
        &bob,
        conv_id,
        "root from bob",
        "qu-root",
        json!({}),
    )
    .await;
    let root_id = root["id"].as_str().unwrap();

    async fn unread_for(
        router: Router,
        principal: &choruz_domain::Principal,
        conv_id: &str,
    ) -> i64 {
        let (st, body) =
            api_json_request(router, principal, Method::GET, "/v1/unreads".into()).await;
        assert_eq!(st, StatusCode::OK);
        body.as_array()
            .unwrap()
            .iter()
            .find(|u| u["conversation_id"] == conv_id)
            .map(|u| u["unread_count"].as_i64().unwrap())
            .unwrap_or(0)
    }
    let before = unread_for(router.clone(), &alice, conv_id).await;
    assert_eq!(before, 1, "alice has 1 unread from bob's root");

    // Alice sends a QUIET threaded reply. Her pre-existing unread must
    // survive (the gate skips mark_conversation_viewed).
    let (st, _) = api_send_with_metadata(
        router.clone(),
        &alice,
        conv_id,
        "quiet reply from alice",
        "qu-r1",
        json!({"reply_to_id": root_id, "thread": true}),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let after_quiet = unread_for(router.clone(), &alice, conv_id).await;
    assert_eq!(
        after_quiet, 1,
        "quiet thread reply must NOT clear the sender's pre-existing unread"
    );

    // A NORMAL message from Alice still auto-marks her read (old path
    // intact): her unread drops to 0.
    let (st2, _) = api_send_with_metadata(
        router.clone(),
        &alice,
        conv_id,
        "normal message",
        "qu-n1",
        json!({}),
    )
    .await;
    assert_eq!(st2, StatusCode::CREATED);
    let after_normal = unread_for(router.clone(), &alice, conv_id).await;
    assert_eq!(
        after_normal, 0,
        "normal sends keep auto-marking the sender as read"
    );
}

#[tokio::test]
async fn delete_conversation_cascades_thread_receipts() {
    // Pins the V018 FK ordering integration (Gate-3 surface-4): hard
    // conversation deletion runs while thread_read_receipt rows still
    // exist, so both ON DELETE CASCADE paths (conversation_id and
    // thread_root_id) fire mid-delete. The delete must succeed — no FK
    // violation — and take the receipts with it.
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let router = router_with_db(app.clone(), &database.database_url);

    let alice = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-threads-delete".into(),
            principal_type: PrincipalType::Human,
            name: "Alice".into(),
            avatar_url: None,
        })
        .unwrap();
    seed_principal_to_db(&database.database_url, &alice).await;

    let (_, group) = api_json_payload_request(
        router.clone(),
        &alice,
        Method::POST,
        "/v1/groups".into(),
        json!({
            "actor_id": alice.id.clone(),
            "name": "threads-delete-group",
            "description": null,
            "avatar_url": null,
            "member_ids": [],
        }),
    )
    .await;
    let conv_id = group["id"].as_str().unwrap();

    let (_, root) = api_send_with_metadata(
        router.clone(),
        &alice,
        conv_id,
        "root",
        "del-root",
        json!({}),
    )
    .await;
    let root_id = root["id"].as_str().unwrap();
    let (st, _) = api_send_with_metadata(
        router.clone(),
        &alice,
        conv_id,
        "reply",
        "del-r1",
        json!({"reply_to_id": root_id, "thread": true}),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    // Create a receipt row.
    let (view_st, _) = api_json_payload_request(
        router.clone(),
        &alice,
        Method::POST,
        format!("/v1/conversations/{conv_id}/threads/{root_id}/view"),
        json!({}),
    )
    .await;
    assert_eq!(view_st, StatusCode::NO_CONTENT);

    let receipt_count = |db_url: &str, conv: &str| {
        let db_url = db_url.to_string();
        let conv = conv.to_string();
        async move {
            let (client, connection) = tokio_postgres::connect(&db_url, NoTls)
                .await
                .expect("connect");
            tokio::spawn(async move {
                let _ = connection.await;
            });
            client
                .query_one(
                    "SELECT COUNT(*)::BIGINT FROM thread_read_receipt WHERE conversation_id = $1",
                    &[&conv],
                )
                .await
                .expect("count")
                .get::<_, i64>(0)
        }
    };
    assert_eq!(receipt_count(&database.database_url, conv_id).await, 1);

    // V017 product metadata uses NO ACTION foreign keys, so the HTTP batch
    // deletion path must explicitly remove/detach it before the conversation.
    let (metadata_client, metadata_connection) =
        tokio_postgres::connect(&database.database_url, NoTls)
            .await
            .expect("connect for group metadata seed");
    tokio::spawn(async move {
        let _ = metadata_connection.await;
    });
    let company_id = "company-threads-delete";
    let job_id = "job-threads-delete";
    metadata_client
        .execute(
            "INSERT INTO company (id, name, slug, owner_id)
             VALUES ($1, $1, $1, $2)",
            &[&company_id, &alice.id],
        )
        .await
        .expect("seed group metadata company");
    metadata_client
        .execute(
            "INSERT INTO group_provisioning_job
               (id, company_id, requested_by, group_template_id,
                group_template_version, status, idempotency_key, created_group_id)
             VALUES ($1, $2, $3, 'curated-team', '1', 'completed', $1, $4)",
            &[&job_id, &company_id, &alice.id, &conv_id],
        )
        .await
        .expect("seed provisioning job");
    metadata_client
        .execute(
            "INSERT INTO group_template_instance
               (group_conversation_id, group_template_id, group_template_version,
                mission, kickoff_text, originating_job_id)
             VALUES ($1, 'curated-team', '1', 'mission', 'kickoff', $2)",
            &[&conv_id, &job_id],
        )
        .await
        .expect("seed group template instance");
    metadata_client
        .execute(
            "INSERT INTO group_template_role_assignment
               (id, group_conversation_id, slot_id, action,
                role_template_id, role_template_version, originating_job_id)
             VALUES ('assignment-threads-delete', $1, 'optional-role', 'skipped',
                     'reviewer', '1', $2)",
            &[&conv_id, &job_id],
        )
        .await
        .expect("seed group role assignment");

    // The human member hard-deletes the conversation via the only HTTP deletion
    // surface (`POST /v1/agents/batch-disable`). It explicitly clears the
    // V017 NO ACTION metadata, then removes events and the conversation;
    // both receipt cascade paths must remain FK-safe.
    let (del_st, del_body) = api_json_payload_request(
        router.clone(),
        &alice,
        Method::POST,
        "/v1/agents/batch-disable".into(),
        json!({
            "actor_id": alice.id.clone(),
            "agent_ids": [],
            "conversation_ids": [conv_id],
        }),
    )
    .await;
    assert_eq!(
        del_st,
        StatusCode::OK,
        "conversation delete must succeed with receipts present: {del_body}"
    );
    assert_eq!(del_body["conversations_deleted"], 1);
    assert_eq!(del_body["conversations_failed"], 0);
    assert_eq!(
        receipt_count(&database.database_url, conv_id).await,
        0,
        "receipts cascade away with the conversation"
    );
    let metadata_state = metadata_client
        .query_one(
            "SELECT
               (SELECT created_group_id IS NULL FROM group_provisioning_job WHERE id = $1),
               NOT EXISTS(SELECT 1 FROM group_template_instance WHERE group_conversation_id = $2),
               NOT EXISTS(SELECT 1 FROM group_template_role_assignment WHERE group_conversation_id = $2)",
            &[&job_id, &conv_id],
        )
        .await
        .expect("verify group metadata cleanup");
    assert!(metadata_state.get::<_, bool>(0));
    assert!(metadata_state.get::<_, bool>(1));
    assert!(metadata_state.get::<_, bool>(2));

    // Drive the OTHER order too — db_service::delete_conversation removes
    // conversation_events BEFORE the conversation row, so the
    // thread_root_id cascade must clear receipts on its own. Reproduced
    // with raw SQL (the events-first order has no HTTP surface).
    let (_, group2) = api_json_payload_request(
        router.clone(),
        &alice,
        Method::POST,
        "/v1/groups".into(),
        json!({
            "actor_id": alice.id.clone(),
            "name": "threads-delete-group-2",
            "description": null,
            "avatar_url": null,
            "member_ids": [],
        }),
    )
    .await;
    let conv2_id = group2["id"].as_str().unwrap();
    let (_, root2) = api_send_with_metadata(
        router.clone(),
        &alice,
        conv2_id,
        "root2",
        "del2-root",
        json!({}),
    )
    .await;
    let root2_id = root2["id"].as_str().unwrap();
    let (st2, _) = api_send_with_metadata(
        router.clone(),
        &alice,
        conv2_id,
        "reply2",
        "del2-r1",
        json!({"reply_to_id": root2_id, "thread": true}),
    )
    .await;
    assert_eq!(st2, StatusCode::CREATED);
    let (view2_st, _) = api_json_payload_request(
        router.clone(),
        &alice,
        Method::POST,
        format!("/v1/conversations/{conv2_id}/threads/{root2_id}/view"),
        json!({}),
    )
    .await;
    assert_eq!(view2_st, StatusCode::NO_CONTENT);
    assert_eq!(receipt_count(&database.database_url, conv2_id).await, 1);

    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "DELETE FROM conversation_events WHERE conversation_id = $1",
            &[&conv2_id],
        )
        .await
        .expect("events-first delete must not hit an FK violation");
    assert_eq!(
        receipt_count(&database.database_url, conv2_id).await,
        0,
        "thread_root_id cascade clears receipts when events go first"
    );
    client
        .execute("DELETE FROM conversation WHERE id = $1", &[&conv2_id])
        .await
        .expect("conversation delete after events");
}
