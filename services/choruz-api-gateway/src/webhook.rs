use choruz_application::DbService;
use choruz_common::AppError;
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

/// Deliver canonical database events in order. A failed delivery stops only its
/// recipient; a storage error fails the flush without advancing another cursor.
pub(crate) async fn flush_webhooks(db: &DbService) -> Result<WebhookFlushResponse, AppError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("failed to build HTTP client");

    let mut deliveries = db.collect_pending_webhook_deliveries().await?;

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
                db.mark_webhook_delivered(&delivery.principal_id, delivery.event.delivery_seq)
                    .await?;
                delivered += 1;
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

    Ok(WebhookFlushResponse {
        attempted,
        delivered,
    })
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
        CreateAgentRequest, CreateDirectConversationRequest, DbService, SendMessageRequest,
        SetEventWebhookRequest,
    };
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

    fn message(id: &str, conversation_id: &str, sender_id: &str) -> SendMessageRequest {
        SendMessageRequest {
            conversation_id: conversation_id.into(),
            actor_id: sender_id.into(),
            content: id.into(),
            content_type: "text".into(),
            metadata: json!({}),
            idempotency_key: format!("key-{id}"),
            trace_id: None,
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
        let database = crate::tests::TestDatabase::create().await;
        let db = DbService::new(choruz_store::EventStore::new(&database.database_url));
        let human = db
            .ensure_local_operator("ws-test", "Operator")
            .await
            .unwrap();
        let agent = db
            .create_agent(CreateAgentRequest {
                actor_id: human.id.clone(),
                name: "Webhook agent".into(),
                scopes: vec!["events:read".into()],
                workspace_id: None,
                channel_visibility: None,
            })
            .await
            .unwrap();
        let conversation = db
            .create_direct_conversation(CreateDirectConversationRequest {
                actor_id: human.id.clone(),
                peer_principal_id: agent.principal.id.clone(),
                workspace_id: None,
            })
            .await
            .unwrap();
        let healthy_agent = db
            .create_agent(CreateAgentRequest {
                actor_id: human.id.clone(),
                name: "Healthy webhook agent".into(),
                scopes: vec!["events:read".into()],
                workspace_id: None,
                channel_visibility: None,
            })
            .await
            .unwrap();
        let healthy_conversation = db
            .create_direct_conversation(CreateDirectConversationRequest {
                actor_id: human.id.clone(),
                peer_principal_id: healthy_agent.principal.id.clone(),
                workspace_id: None,
            })
            .await
            .unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        db.set_event_webhook(
            &agent.principal.id,
            SetEventWebhookRequest {
                actor_id: human.id.clone(),
                url: format!("http://{address}/blocked"),
                event_types: vec!["message.created".into()],
                secret: Some("test-secret".into()),
            },
        )
        .await
        .unwrap();
        db.set_event_webhook(
            &healthy_agent.principal.id,
            SetEventWebhookRequest {
                actor_id: human.id.clone(),
                url: format!("http://{address}/healthy"),
                event_types: vec!["message.created".into()],
                secret: Some("test-secret".into()),
            },
        )
        .await
        .unwrap();
        db.send_message(message("first", &conversation.id, &human.id))
            .await
            .unwrap();
        db.send_message(message("second", &conversation.id, &human.id))
            .await
            .unwrap();
        db.send_message(message("healthy", &healthy_conversation.id, &human.id))
            .await
            .unwrap();

        let pending = db.collect_pending_webhook_deliveries().await.unwrap();
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

        let failed = flush_webhooks(&db).await.unwrap();
        assert_eq!(failed.attempted, 2);
        assert_eq!(failed.delivered, 1);
        assert_eq!(*receiver_state.attempts.lock().await, vec![first_sequence]);
        assert_eq!(receiver_state.healthy_attempts.load(Ordering::SeqCst), 1);
        assert_eq!(
            db.collect_pending_webhook_deliveries().await.unwrap().len(),
            2
        );
        db.ack_events(&agent.principal.id, second_sequence)
            .await
            .unwrap();
        assert_eq!(
            db.collect_pending_webhook_deliveries().await.unwrap().len(),
            2,
            "polling acknowledgements must not discard the webhook's failed prefix"
        );

        receiver_state
            .fail_first_sequence
            .store(false, Ordering::SeqCst);
        let recovered = flush_webhooks(&db).await.unwrap();
        assert_eq!(recovered.attempted, 2);
        assert_eq!(recovered.delivered, 2);
        assert_eq!(
            *receiver_state.attempts.lock().await,
            vec![first_sequence, first_sequence, second_sequence]
        );
        assert!(
            db.collect_pending_webhook_deliveries()
                .await
                .unwrap()
                .is_empty()
        );

        server.abort();
        let _ = server.await;
    }
}
