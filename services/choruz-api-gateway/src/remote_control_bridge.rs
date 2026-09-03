//! The host end of Remote Control. One bridge task per principal with a paired
//! browser keeps a connection to the Cloud Gateway: it advertises the transport
//! room to paired devices, greets them, and hands every `http.*` / `stream.*`
//! frame to `remote_control_executor`, so a paired browser runs the normal
//! dashboard against this host. Everything that crosses the Cloud Gateway is an
//! `e2e` envelope encrypted with the device session key.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::{SinkExt, StreamExt};
use rand::RngCore;
use serde_json::{Value, json};
use tokio::{sync::mpsc, task::JoinHandle};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::{
    ApiState,
    handlers_remote_control::{BridgeMaterial, load_bridge_material},
    remote_control_executor::{ExecutorTargets, RelayExecutor, TokenIssuer},
};

const TRANSPORT_LIVENESS_TIMEOUT: Duration = Duration::from_secs(75);
const EXECUTOR_OUTBOUND_CAPACITY: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameAction {
    None,
    DeviceHello,
}

#[derive(Clone)]
pub(crate) struct RemoteControlBridgeHub {
    refresh_tx: mpsc::UnboundedSender<String>,
}

impl RemoteControlBridgeHub {
    pub(crate) fn new() -> (Self, mpsc::UnboundedReceiver<String>) {
        let (refresh_tx, refresh_rx) = mpsc::unbounded_channel();
        (Self { refresh_tx }, refresh_rx)
    }

    pub(crate) fn refresh(&self, principal_id: &str) {
        let _ = self.refresh_tx.send(principal_id.to_owned());
    }
}

pub(crate) fn spawn(state: ApiState, mut refresh_rx: mpsc::UnboundedReceiver<String>) {
    tokio::spawn(async move {
        let mut tasks: HashMap<String, JoinHandle<()>> = HashMap::new();
        let mut attempt = 0;
        loop {
            match active_principals(&state).await {
                Ok(principals) => {
                    for principal_id in principals {
                        restart_bridge(&state, &mut tasks, principal_id);
                    }
                    break;
                }
                Err(error) => {
                    tracing::warn!(%error, "remote-control bridge startup scan failed");
                    tokio::time::sleep(reconnect_delay(attempt)).await;
                    attempt = attempt.saturating_add(1);
                }
            }
        }
        while let Some(principal_id) = refresh_rx.recv().await {
            restart_bridge(&state, &mut tasks, principal_id);
        }
        for (_, task) in tasks {
            task.abort();
        }
    });
}

fn restart_bridge(
    state: &ApiState,
    tasks: &mut HashMap<String, JoinHandle<()>>,
    principal_id: String,
) {
    if let Some(previous) = tasks.remove(&principal_id) {
        previous.abort();
    }
    let bridge_state = state.clone();
    let task_principal = principal_id.clone();
    tasks.insert(
        principal_id,
        tokio::spawn(async move { maintain_bridge(bridge_state, task_principal).await }),
    );
}

async fn active_principals(state: &ApiState) -> Result<Vec<String>, String> {
    let client = state
        .event_store
        .connect()
        .await
        .map_err(|error| error.to_string())?;
    client
        .query(
            "SELECT DISTINCT principal_id FROM remote_control_device
             WHERE revoked_at IS NULL",
            &[],
        )
        .await
        .map(|rows| rows.into_iter().map(|row| row.get(0)).collect())
        .map_err(|error| error.to_string())
}

async fn maintain_bridge(state: ApiState, principal_id: String) {
    let mut attempt = 0u32;
    loop {
        let material = match load_bridge_material(&state, &principal_id).await {
            Ok(Some(material)) => material,
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(%principal_id, error = %error.0, "remote-control bridge configuration failed");
                tokio::time::sleep(reconnect_delay(attempt)).await;
                attempt = attempt.saturating_add(1);
                continue;
            }
        };
        match serve_bridge_session(&state, &principal_id, &material).await {
            Ok(()) => attempt = 0,
            Err(error) => {
                tracing::warn!(%principal_id, %error, "remote-control bridge disconnected");
                attempt = attempt.saturating_add(1);
            }
        }
        tokio::time::sleep(reconnect_delay(attempt)).await;
    }
}

fn reconnect_delay(attempt: u32) -> Duration {
    Duration::from_secs(2u64.saturating_pow(attempt.min(5)).min(30))
}

fn socket_url(base: &str, ticket: &str) -> Result<String, String> {
    let mut url = reqwest::Url::parse(base).map_err(|error| error.to_string())?;
    url.set_scheme(if url.scheme() == "https" { "wss" } else { "ws" })
        .map_err(|_| "invalid gateway scheme".to_owned())?;
    url.set_path("/connect");
    url.set_query(None);
    url.query_pairs_mut().append_pair("ticket", ticket);
    Ok(url.to_string())
}

async fn serve_bridge_session(
    state: &ApiState,
    principal_id: &str,
    material: &BridgeMaterial,
) -> Result<(), String> {
    let transport_url = socket_url(&material.gateway_url, &material.host_transport_ticket)?;
    let (transport, _) = connect_async(&transport_url)
        .await
        .map_err(|error| format!("connect transport: {error}"))?;
    let rendezvous_url = socket_url(&material.gateway_url, &material.host_session_ticket)?;
    let (rendezvous, _) = connect_async(&rendezvous_url)
        .await
        .map_err(|error| format!("connect rendezvous: {error}"))?;
    let (mut transport_tx, mut transport_rx) = transport.split();
    let (mut rendezvous_tx, mut rendezvous_rx) = rendezvous.split();

    rendezvous_tx
        .send(Message::Text(
            json!({
                "type": "gateway.sync_revocations",
                "device_ids": material.revoked_device_ids,
            })
            .to_string()
            .into(),
        ))
        .await
        .map_err(|error| error.to_string())?;
    offer_transport(&mut rendezvous_tx, material, None).await?;

    let principal = state
        .db
        .get_principal(principal_id)
        .await
        .map_err(|error| error.to_string())?;
    let issue_token: TokenIssuer = {
        let auth = state.auth.clone();
        Arc::new(move || {
            auth.issue_user_session_token(&principal)
                .map_err(|error| error.to_string())
        })
    };
    let (executor_tx, mut executor_rx) = mpsc::channel(EXECUTOR_OUTBOUND_CAPACITY);
    let mut executor = RelayExecutor::new(
        ExecutorTargets {
            api_url: internal_api_url(),
            web_url: internal_web_url(),
        },
        issue_token,
        executor_tx,
    )?;

    let mut heartbeat = tokio::time::interval(Duration::from_secs(25));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    heartbeat.tick().await;
    let mut last_transport_frame = Instant::now();

    loop {
        tokio::select! {
            frame = transport_rx.next() => {
                let Some(frame) = frame else { return Err("transport closed".into()) };
                last_transport_frame = Instant::now();
                match frame.map_err(|error| error.to_string())? {
                    Message::Text(text) => {
                        match handle_remote_frame(state, principal_id, material, &mut executor, &text).await {
                            Ok(FrameAction::DeviceHello) => executor.reset(),
                            Ok(FrameAction::None) => {}
                            Err(error) => {
                                tracing::warn!(%principal_id, %error, "ignored invalid remote-control frame");
                            }
                        }
                    }
                    Message::Ping(bytes) => transport_tx.send(Message::Pong(bytes)).await.map_err(|error| error.to_string())?,
                    Message::Close(_) => return Err("transport closed".into()),
                    _ => {}
                }
            }
            envelope = executor_rx.recv() => {
                let Some(envelope) = envelope else { return Err("relay executor closed".into()) };
                send_encrypted(&mut transport_tx, &material.session_key, &envelope).await?;
            }
            frame = rendezvous_rx.next() => {
                let Some(frame) = frame else { return Err("rendezvous closed".into()) };
                if let Message::Text(text) = frame.map_err(|error| error.to_string())? {
                    let control: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                    if control.get("type").and_then(Value::as_str) == Some("gateway.peer_joined")
                        && control.get("role").and_then(Value::as_str) == Some("device")
                    {
                        offer_transport(
                            &mut rendezvous_tx,
                            material,
                            control.get("device_id").and_then(Value::as_str),
                        ).await?;
                    }
                }
            }
            _ = heartbeat.tick() => {
                if last_transport_frame.elapsed() > TRANSPORT_LIVENESS_TIMEOUT {
                    return Err("transport peer stopped responding".into());
                }
                transport_tx.send(Message::Ping(Vec::new().into())).await.map_err(|error| error.to_string())?;
            }
        }
    }
}

async fn offer_transport<S>(
    sink: &mut S,
    material: &BridgeMaterial,
    target_device_id: Option<&str>,
) -> Result<(), String>
where
    S: futures_util::Sink<Message> + Unpin,
    S::Error: std::fmt::Display,
{
    let tickets: Vec<_> = match target_device_id {
        Some(device_id) => material
            .device_transport_tickets
            .get(device_id)
            .map(|ticket| vec![(device_id, ticket)])
            .unwrap_or_default(),
        None => material
            .device_transport_tickets
            .iter()
            .map(|(device_id, ticket)| (device_id.as_str(), ticket))
            .collect(),
    };
    for (device_id, ticket) in tickets {
        sink.send(Message::Text(
            json!({
                "kind": "session.offer",
                "target_device_id": device_id,
                "payload": {
                    "session_id": material.transport_session_id,
                    "gateway_url": material.gateway_url,
                    "gateway_ticket": ticket,
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

async fn handle_remote_frame(
    state: &ApiState,
    principal_id: &str,
    material: &BridgeMaterial,
    executor: &mut RelayExecutor,
    text: &str,
) -> Result<FrameAction, String> {
    let outer: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    if outer.get("kind").and_then(Value::as_str) != Some("e2e") {
        return Ok(FrameAction::None);
    }
    let envelope = decrypt_envelope(&material.session_key, &outer)?;
    let kind = envelope
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let payload = envelope.get("payload").and_then(Value::as_object);
    match kind {
        "device.hello" => {
            let Some(device_id) = payload
                .and_then(|value| value.get("device_id"))
                .and_then(Value::as_str)
            else {
                return Ok(FrameAction::None);
            };
            let client = state
                .event_store
                .connect()
                .await
                .map_err(|error| error.to_string())?;
            let updated = client
                .execute(
                    "UPDATE remote_control_device SET last_seen_at = NOW()
                     WHERE id = $1 AND principal_id = $2 AND revoked_at IS NULL",
                    &[&device_id, &principal_id],
                )
                .await
                .map_err(|error| error.to_string())?;
            if updated == 0 {
                return Ok(FrameAction::None);
            }
            Ok(FrameAction::DeviceHello)
        }
        kind if kind.starts_with("http.") || kind.starts_with("stream.") => {
            let Some(payload) = payload else {
                return Ok(FrameAction::None);
            };
            executor.handle(kind, payload).await?;
            Ok(FrameAction::None)
        }
        _ => Ok(FrameAction::None),
    }
}

fn internal_api_url() -> String {
    std::env::var("CHORUZ_INTERNAL_API_URL")
        .unwrap_or_else(|_| {
            format!(
                "http://127.0.0.1:{}",
                std::env::var("CHORUZ_API_PORT").unwrap_or_else(|_| "3000".into())
            )
        })
        .trim_end_matches('/')
        .to_owned()
}

fn internal_web_url() -> String {
    std::env::var("CHORUZ_INTERNAL_WEB_URL")
        .unwrap_or_else(|_| {
            format!(
                "http://127.0.0.1:{}",
                std::env::var("CHORUZ_WEB_PORT").unwrap_or_else(|_| "3100".into())
            )
        })
        .trim_end_matches('/')
        .to_owned()
}

async fn send_encrypted<S>(sink: &mut S, session_key: &str, value: &Value) -> Result<(), String>
where
    S: futures_util::Sink<Message> + Unpin,
    S::Error: std::fmt::Display,
{
    let frame = encrypt_envelope(session_key, value)?;
    sink.send(Message::Text(frame.to_string().into()))
        .await
        .map_err(|error| error.to_string())
}

fn encrypt_envelope(session_key: &str, value: &Value) -> Result<Value, String> {
    let key = URL_SAFE_NO_PAD
        .decode(session_key)
        .map_err(|error| error.to_string())?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|error| error.to_string())?;
    let mut iv = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut iv);
    let plaintext = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&iv), plaintext.as_ref())
        .map_err(|error| error.to_string())?;
    Ok(json!({
        "kind": "e2e",
        "iv": URL_SAFE_NO_PAD.encode(iv),
        "ciphertext": URL_SAFE_NO_PAD.encode(ciphertext),
    }))
}

fn decrypt_envelope(session_key: &str, outer: &Value) -> Result<Value, String> {
    let key = URL_SAFE_NO_PAD
        .decode(session_key)
        .map_err(|error| error.to_string())?;
    let iv = URL_SAFE_NO_PAD
        .decode(
            outer
                .get("iv")
                .and_then(Value::as_str)
                .ok_or("missing iv")?,
        )
        .map_err(|error| error.to_string())?;
    let ciphertext = URL_SAFE_NO_PAD
        .decode(
            outer
                .get("ciphertext")
                .and_then(Value::as_str)
                .ok_or("missing ciphertext")?,
        )
        .map_err(|error| error.to_string())?;
    if iv.len() != 12 {
        return Err("invalid iv length".into());
    }
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|error| error.to_string())?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&iv), ciphertext.as_ref())
        .map_err(|error| error.to_string())?;
    serde_json::from_slice(&plaintext).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{decrypt_envelope, encrypt_envelope};
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use serde_json::json;

    fn session_key() -> String {
        URL_SAFE_NO_PAD.encode([7u8; 32])
    }

    #[test]
    fn encryption_round_trips_with_browser_wire_shape() {
        let key = session_key();
        let envelope = json!({"kind": "http.request", "payload": {"request_id": "r1", "method": "GET", "path": "/api/v1/bootstrap"}});
        let frame = encrypt_envelope(&key, &envelope).expect("encrypt");
        assert_eq!(frame["kind"], "e2e");
        assert!(frame["iv"].is_string());
        assert!(frame["ciphertext"].is_string());
        assert_eq!(decrypt_envelope(&key, &frame).expect("decrypt"), envelope);
    }

    #[test]
    fn malformed_encryption_iv_is_rejected_without_panicking() {
        let key = session_key();
        let frame = json!({"kind": "e2e", "iv": URL_SAFE_NO_PAD.encode([1u8; 4]), "ciphertext": URL_SAFE_NO_PAD.encode([2u8; 20])});
        assert_eq!(
            decrypt_envelope(&key, &frame).unwrap_err(),
            "invalid iv length"
        );
        let frame = json!({"kind": "e2e", "iv": "*not base64*", "ciphertext": "AA"});
        assert!(decrypt_envelope(&key, &frame).is_err());
    }
}
