//! Client side of Choruz Remote Control's encrypted transport.
//!
//! Browsers use the TypeScript implementation under `apps/web/lib/remote`.
//! This crate implements the same wire contract for a headless runtime host,
//! so `choruz-connector` can reach a controller that is not publicly exposed:
//! `http.*` frames carry one request and its response, `stream.*` frames
//! carry a WebSocket the controller opens on the device's behalf.

pub mod online;

use std::{collections::HashMap, time::Duration};

use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::{SinkExt, StreamExt};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use p256::{
    EncodedPoint, PublicKey, SecretKey, ecdh::diffie_hellman, elliptic_curve::sec1::ToEncodedPoint,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::{connect_async, tungstenite::Message};

const CHUNK_BYTES: usize = 384 * 1024;
const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(30);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(35);

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RemoteCredentials {
    pub device_id: String,
    pub gateway_url: String,
    pub gateway_ticket: String,
    pub session_key: String,
}

#[derive(Debug)]
pub struct RelayResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Clone)]
pub struct RelayHttpClient {
    commands: mpsc::Sender<Command>,
}

enum Command {
    Request(HttpCommand),
    OpenStream {
        path: String,
        reply: oneshot::Sender<Result<RelayStream, String>>,
    },
}

struct HttpCommand {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    reply: oneshot::Sender<Result<RelayResponse, String>>,
}

/// One WebSocket message carried over a relay stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamMessage {
    Text(String),
    Binary(Vec<u8>),
}

/// A WebSocket the controller opened for this device. Dropping `outbound`
/// closes the stream; `inbound` ends when the controller closes it or the
/// relay session drops.
pub struct RelayStream {
    pub outbound: mpsc::Sender<StreamMessage>,
    pub inbound: mpsc::Receiver<StreamMessage>,
}

/// A stream the controller has confirmed, from the run loop's side.
struct OpenStreamState {
    to_app: mpsc::Sender<StreamMessage>,
    partial: Vec<u8>,
    partial_binary: bool,
}

/// A stream the device asked for and the controller has not confirmed yet.
struct PendingStream {
    reply: oneshot::Sender<Result<RelayStream, String>>,
    to_app: mpsc::Sender<StreamMessage>,
    handoff: RelayStream,
}

#[derive(Default)]
struct Streams {
    open: HashMap<String, OpenStreamState>,
    pending: HashMap<String, PendingStream>,
}

const STREAM_CAPACITY: usize = 256;

struct Pending {
    reply: oneshot::Sender<Result<RelayResponse, String>>,
    status: Option<u16>,
    headers: Vec<(String, String)>,
    chunks: Vec<Option<Vec<u8>>>,
    received: usize,
}

impl RelayHttpClient {
    pub fn start(credentials: RemoteCredentials) -> Self {
        let (commands, receiver) = mpsc::channel(64);
        tokio::spawn(run(credentials, receiver));
        Self { commands }
    }

    pub async fn request(
        &self,
        method: impl Into<String>,
        path: impl Into<String>,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> Result<RelayResponse, String> {
        if body.len() > MAX_BODY_BYTES {
            return Err("relay request body is too large".into());
        }
        let (reply, response) = oneshot::channel();
        self.commands
            .send(Command::Request(HttpCommand {
                method: method.into(),
                path: path.into(),
                headers,
                body,
                reply,
            }))
            .await
            .map_err(|_| "relay client stopped".to_owned())?;
        tokio::time::timeout(REQUEST_TIMEOUT, response)
            .await
            .map_err(|_| "the controller did not respond in time".to_owned())?
            .map_err(|_| "relay client stopped".to_owned())?
    }

    /// Open a WebSocket at `path` on the controller (a gateway socket path
    /// such as `/v1/ws/...`) through the relay.
    pub async fn open_stream(&self, path: impl Into<String>) -> Result<RelayStream, String> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(Command::OpenStream {
                path: path.into(),
                reply,
            })
            .await
            .map_err(|_| "relay client stopped".to_owned())?;
        tokio::time::timeout(REQUEST_TIMEOUT, response)
            .await
            .map_err(|_| "the controller did not open the stream in time".to_owned())?
            .map_err(|_| "relay client stopped".to_owned())?
    }
}

pub async fn pair(
    gateway_url: &str,
    credential: &str,
    device_name: &str,
) -> Result<RemoteCredentials, String> {
    install_tls_provider()?;
    let (pairing_id, credential_secret) = parse_credential(credential)?;
    let secret = SecretKey::random(&mut rand::rngs::OsRng);
    let device_public_key = public_jwk(secret.public_key());
    let device_nonce = random_base64(16);
    let device_commitment = pairing_commitment(&device_public_key, &device_nonce);
    let mut socket = tokio::time::timeout(
        CONNECTION_TIMEOUT,
        connect_async(socket_url(
            gateway_url,
            &[("pairing_id", pairing_id), ("role", "pair_client")],
        )?),
    )
    .await
    .map_err(|_| "connect to Cloud Gateway timed out".to_owned())?
    .map_err(|error| format!("connect to Cloud Gateway: {error}"))?
    .0;
    socket
        .send(Message::Text(
            json!({
                "kind": "pair.commit",
                "device_name": if device_name.trim().is_empty() { "Choruz runtime host" } else { device_name.trim() },
                "device_commitment": device_commitment,
            })
            .to_string()
            .into(),
        ))
        .await
        .map_err(|error| format!("send pairing commitment: {error}"))?;

    let mut host_commitment = None;
    let mut derived = None;
    let mut host_public_key_value = None;
    while let Some(frame) = socket.next().await {
        let frame = frame.map_err(|error| format!("read Cloud Gateway: {error}"))?;
        let Message::Text(text) = frame else { continue };
        let value: Value = serde_json::from_str(&text)
            .map_err(|error| format!("decode pairing frame: {error}"))?;
        match value["kind"].as_str() {
            Some("pair.commit") => {
                let commitment = value["host_commitment"]
                    .as_str()
                    .ok_or("host pairing commitment is missing")?;
                if host_commitment.replace(commitment.to_owned()).is_some() {
                    return Err("host sent more than one pairing commitment".into());
                }
                socket
                    .send(Message::Text(
                        json!({
                            "kind": "pair.reveal",
                            "device_public_key": device_public_key,
                            "device_nonce": device_nonce,
                        })
                        .to_string()
                        .into(),
                    ))
                    .await
                    .map_err(|error| format!("reveal pairing key: {error}"))?;
            }
            Some("pair.reveal") => {
                let host_public_key = value["host_public_key"]
                    .as_str()
                    .ok_or("host public key is missing")?;
                let host_nonce = value["host_nonce"]
                    .as_str()
                    .ok_or("host pairing nonce is missing")?;
                let host_proof = value["host_proof"]
                    .as_str()
                    .ok_or("host pairing proof is missing")?;
                if host_commitment.as_deref()
                    != Some(pairing_commitment(host_public_key, host_nonce).as_str())
                {
                    return Err("host pairing commitment did not match".into());
                }
                let pairing_key = pairing_secret(&secret, host_public_key, credential_secret)?;
                if pairing_proof(&pairing_key, "host", host_public_key, &device_public_key)?
                    != host_proof
                {
                    return Err("host did not prove possession of the pairing credential".into());
                }
                let proof =
                    pairing_proof(&pairing_key, "device", host_public_key, &device_public_key)?;
                socket
                    .send(Message::Text(
                        json!({"kind": "pair.proof", "device_proof": proof})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .map_err(|error| format!("send pairing proof: {error}"))?;
                derived = Some(pairing_key);
                host_public_key_value = Some(host_public_key.to_owned());
            }
            Some("pair.complete") => {
                if host_public_key_value.is_none() {
                    return Err("host completed pairing before proving its identity".into());
                }
                let key = derived
                    .as_deref()
                    .ok_or("pairing key was not established")?;
                let complete = decrypt(
                    key,
                    value["iv"].as_str().ok_or("pairing completion has no iv")?,
                    value["ciphertext"]
                        .as_str()
                        .ok_or("pairing completion has no ciphertext")?,
                )?;
                return serde_json::from_value(complete)
                    .map_err(|error| format!("decode pairing credentials: {error}"));
            }
            _ => {}
        }
    }
    Err("Cloud Gateway closed before pairing completed".into())
}

async fn run(credentials: RemoteCredentials, mut commands: mpsc::Receiver<Command>) {
    let mut backoff = Duration::from_secs(1);
    loop {
        match connect_transport(&credentials).await {
            Ok((mut rendezvous, mut transport)) => {
                backoff = Duration::from_secs(1);
                let mut pending = HashMap::<String, Pending>::new();
                let mut streams = Streams::default();
                // Every open stream's outbound messages arrive here tagged
                // with the stream id; `None` means the device dropped its end.
                let (stream_out_tx, mut stream_out_rx) =
                    mpsc::channel::<(String, Option<StreamMessage>)>(STREAM_CAPACITY);
                loop {
                    tokio::select! {
                        command = commands.recv() => {
                            let Some(command) = command else { return };
                            match command {
                                Command::Request(command) => {
                                    if let Err((error, reply)) = send_request(&credentials, &mut transport, command, &mut pending).await {
                                        let _ = reply.send(Err(error));
                                    }
                                }
                                Command::OpenStream { path, reply } => {
                                    if let Err((error, reply)) = open_stream(&credentials, &mut transport, path, reply, &mut streams, stream_out_tx.clone()).await {
                                        let _ = reply.send(Err(error));
                                    }
                                }
                            }
                        }
                        outgoing = stream_out_rx.recv() => {
                            let Some((stream_id, message)) = outgoing else { continue };
                            if let Err(error) = send_stream_message(&credentials, &mut transport, &stream_id, message, &mut streams).await {
                                fail_pending(&mut pending, &error);
                                break;
                            }
                        }
                        frame = transport.next() => {
                            match frame {
                                Some(Ok(Message::Text(text))) => {
                                    if let Err(error) = receive_frame(&credentials, &text, &mut pending, &mut streams).await {
                                        fail_pending(&mut pending, &error);
                                        break;
                                    }
                                }
                                Some(Ok(Message::Ping(bytes))) => {
                                    if transport.send(Message::Pong(bytes)).await.is_err() { break; }
                                }
                                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => {
                                    fail_pending(&mut pending, "the controller connection closed");
                                    break;
                                }
                                _ => {}
                            }
                        }
                        frame = rendezvous.next() => {
                            match frame {
                                Some(Ok(Message::Ping(bytes))) => {
                                    if rendezvous.send(Message::Pong(bytes)).await.is_err() { break; }
                                }
                                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => {
                                    fail_pending(&mut pending, "the controller rendezvous closed");
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            Err(error) => {
                while let Ok(command) = commands.try_recv() {
                    match command {
                        Command::Request(command) => {
                            let _ = command.reply.send(Err(error.clone()));
                        }
                        Command::OpenStream { reply, .. } => {
                            let _ = reply.send(Err(error.clone()));
                        }
                    }
                }
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(5));
    }
}

async fn connect_transport(credentials: &RemoteCredentials) -> Result<(Socket, Socket), String> {
    install_tls_provider()?;
    let (mut rendezvous, _) = tokio::time::timeout(
        CONNECTION_TIMEOUT,
        connect_async(socket_url(
            &credentials.gateway_url,
            &[("ticket", credentials.gateway_ticket.as_str())],
        )?),
    )
    .await
    .map_err(|_| "connect to controller rendezvous timed out".to_owned())?
    .map_err(|error| format!("connect to controller rendezvous: {error}"))?;
    let offer = tokio::time::timeout(CONNECTION_TIMEOUT, async {
        while let Some(frame) = rendezvous.next().await {
            let Message::Text(text) = frame.map_err(|error| error.to_string())? else {
                continue;
            };
            let value: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
            if value["kind"].as_str() == Some("session.offer")
                && value["target_device_id"].as_str() == Some(&credentials.device_id)
            {
                return Ok::<Value, String>(value["payload"].clone());
            }
        }
        Err("controller rendezvous closed before a transport offer".into())
    })
    .await
    .map_err(|_| "controller did not offer a transport in time".to_owned())??;
    let gateway_url = offer["gateway_url"]
        .as_str()
        .ok_or("transport offer has no gateway URL")?;
    let ticket = offer["gateway_ticket"]
        .as_str()
        .ok_or("transport offer has no gateway ticket")?;
    let (mut transport, _) = connect_async(socket_url(gateway_url, &[("ticket", ticket)])?)
        .await
        .map_err(|error| format!("connect controller transport: {error}"))?;
    let hello = encrypt(
        &credentials.session_key,
        &json!({"kind": "device.hello", "payload": {"device_id": credentials.device_id}}),
    )?;
    transport
        .send(Message::Text(hello.to_string().into()))
        .await
        .map_err(|error| format!("greet controller: {error}"))?;
    Ok((rendezvous, transport))
}

async fn open_stream(
    credentials: &RemoteCredentials,
    transport: &mut Socket,
    path: String,
    reply: oneshot::Sender<Result<RelayStream, String>>,
    streams: &mut Streams,
    stream_out: mpsc::Sender<(String, Option<StreamMessage>)>,
) -> Result<(), (String, oneshot::Sender<Result<RelayStream, String>>)> {
    let stream_id = uuid::Uuid::now_v7().to_string();
    let frame = json!({
        "kind": "stream.open",
        "payload": { "stream_id": stream_id, "path": path },
    });
    if let Err(error) = send_encrypted(transport, &credentials.session_key, &frame).await {
        return Err((error, reply));
    }
    let (to_app, inbound) = mpsc::channel(STREAM_CAPACITY);
    let (outbound, mut from_app) = mpsc::channel::<StreamMessage>(STREAM_CAPACITY);
    // The device writes into `outbound`; this task tags each message with
    // the stream id for the run loop and reports the device dropping its end.
    let forward_id = stream_id.clone();
    tokio::spawn(async move {
        while let Some(message) = from_app.recv().await {
            if stream_out
                .send((forward_id.clone(), Some(message)))
                .await
                .is_err()
            {
                return;
            }
        }
        let _ = stream_out.send((forward_id, None)).await;
    });
    streams.pending.insert(
        stream_id,
        PendingStream {
            reply,
            to_app,
            handoff: RelayStream { outbound, inbound },
        },
    );
    Ok(())
}

async fn send_stream_message(
    credentials: &RemoteCredentials,
    transport: &mut Socket,
    stream_id: &str,
    message: Option<StreamMessage>,
    streams: &mut Streams,
) -> Result<(), String> {
    if !streams.open.contains_key(stream_id) {
        return Ok(());
    }
    let Some(message) = message else {
        streams.open.remove(stream_id);
        let frame = json!({
            "kind": "stream.close",
            "payload": { "stream_id": stream_id, "code": 1000, "reason": "" },
        });
        return send_encrypted(transport, &credentials.session_key, &frame).await;
    };
    let (encoding, bytes) = match &message {
        StreamMessage::Text(text) => ("text", text.as_bytes()),
        StreamMessage::Binary(bytes) => ("binary", bytes.as_slice()),
    };
    let chunks: Vec<&[u8]> = if bytes.is_empty() {
        vec![&[][..]]
    } else {
        bytes.chunks(CHUNK_BYTES).collect()
    };
    let count = chunks.len();
    for (index, chunk) in chunks.into_iter().enumerate() {
        let frame = json!({
            "kind": "stream.data",
            "payload": {
                "stream_id": stream_id,
                "encoding": encoding,
                "data": URL_SAFE_NO_PAD.encode(chunk),
                "last": index + 1 == count,
            },
        });
        send_encrypted(transport, &credentials.session_key, &frame).await?;
    }
    Ok(())
}

async fn send_request(
    credentials: &RemoteCredentials,
    transport: &mut Socket,
    command: HttpCommand,
    pending: &mut HashMap<String, Pending>,
) -> Result<(), (String, oneshot::Sender<Result<RelayResponse, String>>)> {
    let request_id = uuid::Uuid::now_v7().to_string();
    let headers = command
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), Value::String(value.clone())))
        .collect::<Map<String, Value>>();
    let chunks = command.body.chunks(CHUNK_BYTES).collect::<Vec<_>>();
    let frame = json!({
        "kind": "http.request",
        "payload": {
            "request_id": request_id,
            "method": command.method,
            "path": command.path,
            "headers": headers,
            "body_chunks": chunks.len(),
        }
    });
    if let Err(error) = send_encrypted(transport, &credentials.session_key, &frame).await {
        return Err((error, command.reply));
    }
    for (index, chunk) in chunks.into_iter().enumerate() {
        let frame = json!({
            "kind": "http.body",
            "payload": {
                "request_id": request_id,
                "index": index,
                "data": URL_SAFE_NO_PAD.encode(chunk),
            }
        });
        if let Err(error) = send_encrypted(transport, &credentials.session_key, &frame).await {
            return Err((error, command.reply));
        }
    }
    pending.insert(
        request_id,
        Pending {
            reply: command.reply,
            status: None,
            headers: Vec::new(),
            chunks: Vec::new(),
            received: 0,
        },
    );
    Ok(())
}

async fn receive_frame(
    credentials: &RemoteCredentials,
    raw: &str,
    pending: &mut HashMap<String, Pending>,
    streams: &mut Streams,
) -> Result<(), String> {
    let outer: Value = serde_json::from_str(raw).map_err(|error| error.to_string())?;
    if outer["kind"].as_str() != Some("e2e") {
        return Ok(());
    }
    let envelope = decrypt(
        &credentials.session_key,
        outer["iv"].as_str().ok_or("relay frame has no iv")?,
        outer["ciphertext"]
            .as_str()
            .ok_or("relay frame has no ciphertext")?,
    )?;
    let kind = envelope["kind"].as_str().unwrap_or_default().to_owned();
    if kind.starts_with("stream.") {
        return receive_stream_frame(&kind, &envelope, streams).await;
    }
    receive_response(&envelope, pending)
}

async fn receive_stream_frame(
    kind: &str,
    envelope: &Value,
    streams: &mut Streams,
) -> Result<(), String> {
    let payload = envelope["payload"]
        .as_object()
        .ok_or("relay stream frame has no payload")?;
    let stream_id = payload["stream_id"]
        .as_str()
        .ok_or("relay stream frame has no stream id")?;
    match kind {
        "stream.opened" => {
            let Some(pending) = streams.pending.remove(stream_id) else {
                return Ok(());
            };
            streams.open.insert(
                stream_id.to_owned(),
                OpenStreamState {
                    to_app: pending.to_app,
                    partial: Vec::new(),
                    partial_binary: false,
                },
            );
            let _ = pending.reply.send(Ok(pending.handoff));
        }
        "stream.data" => {
            let Some(state) = streams.open.get_mut(stream_id) else {
                return Ok(());
            };
            let data = URL_SAFE_NO_PAD
                .decode(payload["data"].as_str().unwrap_or_default())
                .map_err(|error| error.to_string())?;
            state.partial_binary = payload["encoding"].as_str() == Some("binary");
            state.partial.extend_from_slice(&data);
            if state.partial.len() > MAX_BODY_BYTES {
                return Err("relay stream message is too large".into());
            }
            if payload["last"].as_bool().unwrap_or(true) {
                let bytes = std::mem::take(&mut state.partial);
                let message = if state.partial_binary {
                    StreamMessage::Binary(bytes)
                } else {
                    StreamMessage::Text(
                        String::from_utf8(bytes)
                            .map_err(|_| "relay stream text is not UTF-8".to_owned())?,
                    )
                };
                if state.to_app.send(message).await.is_err() {
                    streams.open.remove(stream_id);
                }
            }
        }
        "stream.close" => {
            if let Some(pending) = streams.pending.remove(stream_id) {
                let reason = payload["reason"].as_str().unwrap_or("stream refused");
                let _ = pending
                    .reply
                    .send(Err(format!("the controller refused the stream: {reason}")));
            }
            streams.open.remove(stream_id);
        }
        _ => {}
    }
    Ok(())
}

fn receive_response(
    envelope: &Value,
    pending: &mut HashMap<String, Pending>,
) -> Result<(), String> {
    let payload = envelope["payload"]
        .as_object()
        .ok_or("relay response has no payload")?;
    let request_id = payload["request_id"]
        .as_str()
        .ok_or("relay response has no request id")?;
    match envelope["kind"].as_str() {
        Some("http.response") => {
            if let Some(error) = payload.get("error").and_then(Value::as_str) {
                if let Some(request) = pending.remove(request_id) {
                    let _ = request.reply.send(Err(error.to_owned()));
                }
                return Ok(());
            }
            let status = payload["status"]
                .as_u64()
                .and_then(|value| u16::try_from(value).ok())
                .ok_or("relay response has an invalid status")?;
            let count = payload["body_chunks"]
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .filter(|value| value.saturating_mul(CHUNK_BYTES) <= MAX_BODY_BYTES)
                .ok_or("relay response has an invalid body chunk count")?;
            let request = pending
                .get_mut(request_id)
                .ok_or("relay response has an unknown request id")?;
            request.status = Some(status);
            request.headers = payload
                .get("headers")
                .and_then(Value::as_object)
                .map(|headers| {
                    headers
                        .iter()
                        .filter_map(|(name, value)| {
                            value.as_str().map(|value| (name.clone(), value.to_owned()))
                        })
                        .collect()
                })
                .unwrap_or_default();
            request.chunks = vec![None; count];
            if count == 0 {
                complete(request_id, pending);
            }
        }
        Some("http.body") => {
            let index = payload["index"]
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or("relay body has an invalid index")?;
            let data = URL_SAFE_NO_PAD
                .decode(payload["data"].as_str().ok_or("relay body has no data")?)
                .map_err(|error| error.to_string())?;
            let request = pending
                .get_mut(request_id)
                .ok_or("relay body has an unknown request id")?;
            let slot = request
                .chunks
                .get_mut(index)
                .ok_or("relay body index is out of range")?;
            if slot.is_none() {
                request.received += 1;
            }
            *slot = Some(data);
            if request.received == request.chunks.len() {
                complete(request_id, pending);
            }
        }
        _ => {}
    }
    Ok(())
}

fn complete(request_id: &str, pending: &mut HashMap<String, Pending>) {
    let Some(request) = pending.remove(request_id) else {
        return;
    };
    let body = request.chunks.into_iter().flatten().flatten().collect();
    let _ = request.reply.send(Ok(RelayResponse {
        status: request.status.unwrap_or(500),
        headers: request.headers,
        body,
    }));
}

fn fail_pending(pending: &mut HashMap<String, Pending>, error: &str) {
    for (_, request) in pending.drain() {
        let _ = request.reply.send(Err(error.to_owned()));
    }
}

async fn send_encrypted(
    socket: &mut Socket,
    session_key: &str,
    value: &Value,
) -> Result<(), String> {
    socket
        .send(Message::Text(
            encrypt(session_key, value)?.to_string().into(),
        ))
        .await
        .map_err(|error| error.to_string())
}

fn install_tls_provider() -> Result<(), String> {
    if rustls::crypto::CryptoProvider::get_default().is_some()
        || rustls::crypto::ring::default_provider()
            .install_default()
            .is_ok()
        || rustls::crypto::CryptoProvider::get_default().is_some()
    {
        Ok(())
    } else {
        Err("initialize TLS cryptography provider".into())
    }
}

fn socket_url(base: &str, parameters: &[(&str, &str)]) -> Result<String, String> {
    let mut url = reqwest::Url::parse(base).map_err(|error| error.to_string())?;
    url.set_scheme(if url.scheme() == "https" { "wss" } else { "ws" })
        .map_err(|_| "invalid gateway scheme".to_owned())?;
    url.set_path("/connect");
    url.set_query(None);
    url.query_pairs_mut()
        .extend_pairs(parameters.iter().copied());
    Ok(url.to_string())
}

fn parse_credential(credential: &str) -> Result<(&str, &str), String> {
    let mut parts = credential.trim().split('.');
    let (Some("v1"), Some(id), Some(secret), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err("pairing credential is malformed".into());
    };
    let valid = |value: &str| {
        value.len() == 22
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    };
    if !valid(id) || !valid(secret) {
        return Err("pairing credential is malformed".into());
    }
    Ok((id, secret))
}

fn random_base64(bytes: usize) -> String {
    let mut output = vec![0; bytes];
    rand::rngs::OsRng.fill_bytes(&mut output);
    URL_SAFE_NO_PAD.encode(output)
}

fn public_jwk(public_key: PublicKey) -> String {
    let point = public_key.to_encoded_point(false);
    json!({
        "kty": "EC",
        "crv": "P-256",
        "x": URL_SAFE_NO_PAD.encode(point.x().expect("uncompressed x")),
        "y": URL_SAFE_NO_PAD.encode(point.y().expect("uncompressed y")),
        "ext": true,
        "key_ops": [],
    })
    .to_string()
}

fn peer_public_key(jwk: &str) -> Result<PublicKey, String> {
    let value: Value = serde_json::from_str(jwk).map_err(|_| "peer public key is not JSON")?;
    if value["kty"].as_str() != Some("EC") || value["crv"].as_str() != Some("P-256") {
        return Err("peer public key is not P-256".into());
    }
    let x = URL_SAFE_NO_PAD
        .decode(value["x"].as_str().ok_or("peer public key has no x")?)
        .map_err(|_| "peer public key has invalid x")?;
    let y = URL_SAFE_NO_PAD
        .decode(value["y"].as_str().ok_or("peer public key has no y")?)
        .map_err(|_| "peer public key has invalid y")?;
    if x.len() != 32 || y.len() != 32 {
        return Err("peer public key has invalid coordinates".into());
    }
    let point =
        EncodedPoint::from_affine_coordinates(x.as_slice().into(), y.as_slice().into(), false);
    PublicKey::from_sec1_bytes(point.as_bytes()).map_err(|_| "peer public key is invalid".into())
}

fn pairing_secret(
    secret: &SecretKey,
    peer_jwk: &str,
    credential_secret: &str,
) -> Result<String, String> {
    let peer = peer_public_key(peer_jwk)?;
    let shared = diffie_hellman(secret.to_nonzero_scalar(), peer.as_affine());
    let salt = URL_SAFE_NO_PAD
        .decode(credential_secret)
        .map_err(|_| "pairing credential secret is invalid")?;
    let mut output = [0; 32];
    Hkdf::<Sha256>::new(Some(&salt), shared.raw_secret_bytes().as_slice())
        .expand(b"choruz.remote-control.pairing.v1", &mut output)
        .map_err(|_| "derive pairing key")?;
    Ok(URL_SAFE_NO_PAD.encode(output))
}

fn pairing_commitment(public_key: &str, nonce: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"choruz.remote-control.commit.v1\0");
    hasher.update(public_key.as_bytes());
    hasher.update(b"\0");
    hasher.update(nonce.as_bytes());
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

fn pairing_proof(
    secret: &str,
    role: &str,
    host_public_key: &str,
    device_public_key: &str,
) -> Result<String, String> {
    let key = URL_SAFE_NO_PAD
        .decode(secret)
        .map_err(|_| "invalid pairing key")?;
    let mut mac =
        <Hmac<Sha256> as Mac>::new_from_slice(&key).map_err(|_| "initialize pairing proof")?;
    mac.update(b"choruz.remote-control.proof.v1\0");
    mac.update(role.as_bytes());
    mac.update(b"\0");
    mac.update(host_public_key.as_bytes());
    mac.update(b"\0");
    mac.update(device_public_key.as_bytes());
    Ok(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
}

fn encrypt(session_key: &str, value: &Value) -> Result<Value, String> {
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

fn decrypt(session_key: &str, iv: &str, ciphertext: &str) -> Result<Value, String> {
    let key = URL_SAFE_NO_PAD
        .decode(session_key)
        .map_err(|error| error.to_string())?;
    let iv = URL_SAFE_NO_PAD
        .decode(iv)
        .map_err(|error| error.to_string())?;
    let ciphertext = URL_SAFE_NO_PAD
        .decode(ciphertext)
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
    use super::*;

    #[test]
    fn credential_requires_two_independent_base64url_values() {
        let parsed = parse_credential("v1.AAAAAAAAAAAAAAAAAAAAAA.BBBBBBBBBBBBBBBBBBBBBB").unwrap();
        assert_eq!(parsed.0, "AAAAAAAAAAAAAAAAAAAAAA");
        assert_eq!(parsed.1, "BBBBBBBBBBBBBBBBBBBBBB");
        assert!(parse_credential("12345678").is_err());
    }

    #[test]
    fn encrypted_wire_shape_round_trips() {
        let key = URL_SAFE_NO_PAD.encode([7u8; 32]);
        let value = json!({"kind": "http.request", "payload": {"request_id": "r1"}});
        let outer = encrypt(&key, &value).unwrap();
        assert_eq!(
            decrypt(
                &key,
                outer["iv"].as_str().unwrap(),
                outer["ciphertext"].as_str().unwrap()
            )
            .unwrap(),
            value
        );
    }

    #[test]
    fn websocket_url_never_carries_the_pairing_secret() {
        let url = socket_url(
            "https://gateway.example/path?secret=bad",
            &[("pairing_id", "safe-id"), ("role", "pair_client")],
        )
        .unwrap();
        assert_eq!(
            url,
            "wss://gateway.example/connect?pairing_id=safe-id&role=pair_client"
        );
    }
}
