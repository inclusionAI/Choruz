use choruz_application::{ChatApp, DbService};
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::Sha256;
use std::collections::HashSet;

// ── Webhook flush ─────────────────────────────────────────────────────

#[derive(Debug, Serialize, Default)]
pub(crate) struct WebhookFlushResponse {
    pub(crate) attempted: usize,
    pub(crate) delivered: usize,
}

pub(crate) async fn flush_webhooks(app: &ChatApp) -> WebhookFlushResponse {
    flush_inner(app, None).await
}

/// Preferred entrypoint: also scans the DB-backed `event_webhook` +
/// `outbox_event` tables. Needed because newer code paths (DbService
/// send_message, webhook_agent install, etc.) write events to
/// Postgres without mirroring them into the in-memory `ChatApp` state.
/// Without this both routes would be fired, but DB-only events would
/// sit un-delivered forever.
pub(crate) async fn flush_webhooks_all(app: &ChatApp, db: &DbService) -> WebhookFlushResponse {
    flush_inner(app, Some(db)).await
}

async fn flush_inner(app: &ChatApp, db: Option<&DbService>) -> WebhookFlushResponse {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("failed to build HTTP client");

    // Merge in-memory + DB-backed pending deliveries; dedup by
    // (principal_id, delivery_seq) because the two views can overlap
    // during migration.
    let mut deliveries = app.collect_pending_webhook_deliveries();
    if let Some(db) = db {
        if let Ok(db_deliveries) = db.collect_pending_webhook_deliveries().await {
            let seen: std::collections::HashSet<(String, u64)> = deliveries
                .iter()
                .map(|d| (d.principal_id.clone(), d.event.delivery_seq))
                .collect();
            for d in db_deliveries {
                if !seen.contains(&(d.principal_id.clone(), d.event.delivery_seq)) {
                    deliveries.push(d);
                }
            }
        }
    }

    deliveries.sort_by(|left, right| {
        left.principal_id
            .cmp(&right.principal_id)
            .then(left.event.delivery_seq.cmp(&right.event.delivery_seq))
    });

    let mut attempted = 0;
    let mut delivered = 0;
    let mut blocked_principals = HashSet::new();

    for delivery in &deliveries {
        // A webhook cursor represents a contiguous delivered prefix. Once an
        // event fails, defer later events for that principal instead of
        // advancing the cursor past the gap. Other principals remain isolated
        // and continue flushing normally.
        if blocked_principals.contains(&delivery.principal_id) {
            continue;
        }

        // Serialize the body once so the signature and the POST use the
        // exact same bytes (serde_json may re-order keys otherwise, which
        // would break HMAC verification on the receiver).
        let body = match serde_json::to_vec(&delivery.event) {
            Ok(b) => b,
            Err(error) => {
                tracing::warn!(
                    principal_id = %delivery.principal_id,
                    url = %delivery.url,
                    error = %error,
                    "webhook payload serialization failed"
                );
                blocked_principals.insert(delivery.principal_id.clone());
                continue;
            }
        };
        let timestamp = chrono::Utc::now().timestamp().to_string();
        let signature = match sign_webhook(&delivery.secret, &timestamp, &body) {
            Ok(signature) => signature,
            Err(error) => {
                tracing::error!(
                    principal_id = %delivery.principal_id,
                    url = %delivery.url,
                    %error,
                    "webhook delivery refused because its secret is invalid"
                );
                blocked_principals.insert(delivery.principal_id.clone());
                continue;
            }
        };

        attempted += 1;
        match client
            .post(&delivery.url)
            .header("content-type", "application/json")
            .header("x-choruz-event-id", &delivery.event.event_id)
            .header("x-choruz-timestamp", &timestamp)
            .header("x-choruz-signature", &signature)
            .body(body)
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                // Mark delivered in both in-memory and DB if applicable.
                // Either succeeding is enough for `delivered` accounting —
                // the other will no-op if its cursor is already advanced.
                let mem_ok = app
                    .mark_webhook_delivered(&delivery.principal_id, delivery.event.delivery_seq)
                    .is_ok();
                let db_ok = if let Some(db) = db {
                    db.mark_webhook_delivered(&delivery.principal_id, delivery.event.delivery_seq)
                        .await
                        .is_ok()
                } else {
                    false
                };
                if mem_ok || db_ok {
                    delivered += 1;
                }
            }
            Ok(response) => {
                tracing::warn!(
                    principal_id = %delivery.principal_id,
                    url = %delivery.url,
                    status = %response.status(),
                    "webhook delivery failed"
                );
                blocked_principals.insert(delivery.principal_id.clone());
            }
            Err(error) => {
                tracing::warn!(
                    principal_id = %delivery.principal_id,
                    url = %delivery.url,
                    error = %error,
                    "webhook delivery errored"
                );
                blocked_principals.insert(delivery.principal_id.clone());
            }
        }
    }

    WebhookFlushResponse {
        attempted,
        delivered,
    }
}

/// Produce a `sha256=<hex>` signature of the timestamp and raw body using
/// `secret` as the HMAC key. Binding the timestamp prevents a captured body
/// from being replayed with a fresh timestamp.
///
fn sign_webhook(secret: &str, timestamp: &str, body: &[u8]) -> Result<String, &'static str> {
    if secret.is_empty() {
        return Err("webhook secret must not be empty");
    }
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes())
        .expect("HMAC can accept any key length");
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(body);
    let digest = mac.finalize().into_bytes();
    Ok(format!("sha256={}", hex::encode(digest)))
}

#[cfg(test)]
mod tests {
    use super::{flush_webhooks, sign_webhook};
    use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
    use choruz_application::{
        ChatApp, CreateAgentRequest, CreateDirectConversationRequest, CreatePrincipalRequest,
        SetEventWebhookRequest,
    };
    use choruz_common::now;
    use choruz_domain::{Message, PrincipalType};
    use serde_json::{Value, json};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use tokio::{net::TcpListener, sync::Mutex};

    #[derive(Clone)]
    struct OrderedReceiverState {
        fail_first_sequence: Arc<AtomicBool>,
        first_sequence: u64,
        attempts: Arc<Mutex<Vec<u64>>>,
        healthy_attempts: Arc<AtomicUsize>,
    }

    async fn healthy_receiver(State(state): State<OrderedReceiverState>) -> StatusCode {
        state.healthy_attempts.fetch_add(1, Ordering::SeqCst);
        StatusCode::OK
    }

    async fn ordered_receiver(
        State(state): State<OrderedReceiverState>,
        Json(payload): Json<Value>,
    ) -> StatusCode {
        let sequence = payload["delivery_seq"]
            .as_u64()
            .expect("webhook payload should include delivery_seq");
        state.attempts.lock().await.push(sequence);
        if sequence == state.first_sequence && state.fail_first_sequence.load(Ordering::SeqCst) {
            StatusCode::INTERNAL_SERVER_ERROR
        } else {
            StatusCode::OK
        }
    }

    fn message(id: &str, conversation_id: &str, sender_id: &str, sequence: u64) -> Message {
        Message {
            id: id.into(),
            workspace_id: "ws-test".into(),
            conversation_id: conversation_id.into(),
            sender_id: sender_id.into(),
            content: id.into(),
            content_type: "text".into(),
            metadata: json!({}),
            edited_at: None,
            edited_by: None,
            server_seq: sequence,
            idempotency_key: format!("key-{id}"),
            created_at: now(),
        }
    }

    #[test]
    fn sign_webhook_produces_sha256_prefix() {
        let sig = sign_webhook("topsecret", "123", b"{\"hello\":\"world\"}").unwrap();
        assert!(sig.starts_with("sha256="));
        // hex-encoded sha256 is 64 chars
        assert_eq!(sig.len(), "sha256=".len() + 64);
    }

    #[test]
    fn empty_secret_is_rejected() {
        assert_eq!(
            sign_webhook("", "123", b"anything"),
            Err("webhook secret must not be empty")
        );
    }

    #[test]
    fn deterministic_for_same_input() {
        let a = sign_webhook("k", "123", b"{\"a\":1}").unwrap();
        let b = sign_webhook("k", "123", b"{\"a\":1}").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn signature_binds_timestamp() {
        assert_ne!(
            sign_webhook("k", "123", b"{\"a\":1}").unwrap(),
            sign_webhook("k", "124", b"{\"a\":1}").unwrap(),
        );
    }

    #[tokio::test]
    async fn failed_event_blocks_its_principal_without_stalling_other_webhooks() {
        let app = ChatApp::new();
        let human = app
            .create_principal(CreatePrincipalRequest {
                workspace_id: "ws-test".into(),
                principal_type: PrincipalType::Human,
                name: "Operator".into(),
                avatar_url: None,
            })
            .unwrap();
        let agent = app
            .create_agent(CreateAgentRequest {
                actor_id: human.id.clone(),
                name: "Webhook agent".into(),
                scopes: vec!["events:read".into()],
                workspace_id: None,
                channel_visibility: None,
            })
            .unwrap();
        let conversation = app
            .create_direct_conversation(CreateDirectConversationRequest {
                actor_id: human.id.clone(),
                peer_principal_id: agent.principal.id.clone(),
                workspace_id: None,
            })
            .unwrap();
        let healthy_agent = app
            .create_agent(CreateAgentRequest {
                actor_id: human.id.clone(),
                name: "Healthy webhook agent".into(),
                scopes: vec!["events:read".into()],
                workspace_id: None,
                channel_visibility: None,
            })
            .unwrap();
        let healthy_conversation = app
            .create_direct_conversation(CreateDirectConversationRequest {
                actor_id: human.id.clone(),
                peer_principal_id: healthy_agent.principal.id.clone(),
                workspace_id: None,
            })
            .unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        app.set_event_webhook(
            &agent.principal.id,
            SetEventWebhookRequest {
                actor_id: human.id.clone(),
                url: format!("http://{address}/blocked"),
                event_types: vec!["message.created".into()],
                secret: Some("test-secret".into()),
            },
        )
        .unwrap();
        app.set_event_webhook(
            &healthy_agent.principal.id,
            SetEventWebhookRequest {
                actor_id: human.id.clone(),
                url: format!("http://{address}/healthy"),
                event_types: vec!["message.created".into()],
                secret: Some("test-secret".into()),
            },
        )
        .unwrap();
        app.inject_message_with_event(message("first", &conversation.id, &human.id, 1));
        app.inject_message_with_event(message("second", &conversation.id, &human.id, 2));
        app.inject_message_with_event(message("healthy", &healthy_conversation.id, &human.id, 1));

        let pending = app.collect_pending_webhook_deliveries();
        assert_eq!(pending.len(), 3);
        let primary_pending: Vec<_> = pending
            .iter()
            .filter(|delivery| delivery.principal_id == agent.principal.id)
            .collect();
        let first_sequence = primary_pending[0].event.delivery_seq;
        let second_sequence = primary_pending[1].event.delivery_seq;
        let receiver_state = OrderedReceiverState {
            fail_first_sequence: Arc::new(AtomicBool::new(true)),
            first_sequence,
            attempts: Arc::new(Mutex::new(Vec::new())),
            healthy_attempts: Arc::new(AtomicUsize::new(0)),
        };
        let server_state = receiver_state.clone();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/blocked", post(ordered_receiver))
                    .route("/healthy", post(healthy_receiver))
                    .with_state(server_state),
            )
            .await
            .unwrap();
        });

        let failed = flush_webhooks(&app).await;
        assert_eq!(failed.attempted, 2);
        assert_eq!(failed.delivered, 1);
        assert_eq!(*receiver_state.attempts.lock().await, vec![first_sequence]);
        assert_eq!(receiver_state.healthy_attempts.load(Ordering::SeqCst), 1);
        assert_eq!(app.collect_pending_webhook_deliveries().len(), 2);

        receiver_state
            .fail_first_sequence
            .store(false, Ordering::SeqCst);
        let recovered = flush_webhooks(&app).await;
        assert_eq!(recovered.attempted, 2);
        assert_eq!(recovered.delivered, 2);
        assert_eq!(
            *receiver_state.attempts.lock().await,
            vec![first_sequence, first_sequence, second_sequence]
        );
        assert!(app.collect_pending_webhook_deliveries().is_empty());

        server.abort();
    }
}
