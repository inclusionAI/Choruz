//! Executes same-origin dashboard calls on behalf of a paired remote browser.
//!
//! The remote dashboard is the local dashboard with a relay transport
//! installed: every `/api/*` fetch and every `/v1/ws/*` socket the web app
//! opens is carried across the Cloud Gateway as frames inside the bridge's
//! end-to-end encrypted `e2e` envelope, and this module turns those frames back
//! into local HTTP requests and WebSocket connections. The Cloud Gateway never
//! sees a plaintext frame; the host never lets a remote browser reach anything
//! but the allow-listed dashboard paths below.
//!
//! Frames, all inside the envelope's `payload`:
//!
//! | direction | kind | payload |
//! |---|---|---|
//! | device → host | `http.request` | `request_id`, `method`, `path`, `headers`, `body_chunks` |
//! | both | `http.body` | `request_id`, `index`, `data` |
//! | host → device | `http.response` | `request_id`, `status`, `headers`, `body_chunks`, or `request_id`, `error` |
//! | device → host | `stream.open` | `stream_id`, `path` |
//! | host → device | `stream.opened` | `stream_id` |
//! | both | `stream.data` | `stream_id`, `encoding` (`text` or `binary`), `data`, `last` |
//! | both | `stream.close` | `stream_id`, `code`, `reason` |
//!
//! `data` is base64url without padding, at most `CHUNK_BYTES` of raw bytes per
//! frame so that the encrypted frame stays under the relay's 1 MB cap; a
//! message larger than one chunk is split into consecutive frames and the
//! receiver concatenates until `last` is true.

use std::{collections::HashMap, sync::Arc, time::Duration};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Map, Value, json};
use tokio::{sync::mpsc, task::JoinHandle};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        Message,
        client::IntoClientRequest,
        http::{HeaderValue, header::AUTHORIZATION},
    },
};

/// Raw bytes per `http.body` / `stream.data` frame. Base64 in the frame and
/// base64 again around the ciphertext multiply the size by about 1.8, which
/// keeps a full chunk around 700 KB on the wire.
pub(crate) const CHUNK_BYTES: usize = 384 * 1024;
const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;
const MAX_STREAMS: usize = 32;
const MAX_PENDING_REQUESTS: usize = 64;
const MAX_ID_LEN: usize = 128;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const STREAM_COMMAND_CAPACITY: usize = 64;
/// The web session cookie the Next.js route handlers read; see
/// `local_auth::cookie_name`.
const SESSION_COOKIE: &str = "choruz_session";

const REQUEST_HEADERS_NOT_FORWARDED: &[&str] = &[
    "authorization",
    "cookie",
    "host",
    "content-length",
    "connection",
    "transfer-encoding",
    "origin",
    "referer",
];
const RESPONSE_HEADERS_NOT_FORWARDED: &[&str] = &[
    "set-cookie",
    "content-length",
    "connection",
    "transfer-encoding",
    "keep-alive",
];

/// Issues a fresh session token for the paired principal. Tokens expire in
/// hours while a bridge session can live for days, so every request and
/// stream gets its own.
pub(crate) type TokenIssuer = Arc<dyn Fn() -> Result<String, String> + Send + Sync>;

#[derive(Clone, Debug)]
pub(crate) struct ExecutorTargets {
    /// `http://` origin of the local API gateway.
    pub(crate) api_url: String,
    /// `http://` origin of the local Next.js server.
    pub(crate) web_url: String,
}

pub(crate) struct RelayExecutor {
    targets: ExecutorTargets,
    issue_token: TokenIssuer,
    client: reqwest::Client,
    outbound: mpsc::Sender<Value>,
    pending: HashMap<String, PendingRequest>,
    streams: HashMap<String, StreamHandle>,
}

struct PendingRequest {
    head: RequestHead,
    chunks: Vec<Option<Vec<u8>>>,
    received: usize,
}

struct RequestHead {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body_chunks: usize,
}

struct StreamHandle {
    commands: mpsc::Sender<StreamCommand>,
    task: JoinHandle<()>,
    inbound: Vec<u8>,
}

enum StreamCommand {
    Send(Message),
    Close,
}

enum AuthStyle {
    Bearer,
    Cookie,
}

struct Rejection {
    status: u16,
    error: &'static str,
}

impl RelayExecutor {
    pub(crate) fn new(
        targets: ExecutorTargets,
        issue_token: TokenIssuer,
        outbound: mpsc::Sender<Value>,
    ) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            targets,
            issue_token,
            client,
            outbound,
            pending: HashMap::new(),
            streams: HashMap::new(),
        })
    }

    /// Handles one decrypted frame whose kind starts with `http.` or `stream.`.
    /// An error means the frame was malformed and is dropped; a request that
    /// is well-formed but not allowed gets an `http.response` instead.
    pub(crate) async fn handle(
        &mut self,
        kind: &str,
        payload: &Map<String, Value>,
    ) -> Result<(), String> {
        match kind {
            "http.request" => self.handle_http_request(payload).await,
            "http.body" => self.handle_http_body(payload).await,
            "stream.open" => self.handle_stream_open(payload).await,
            "stream.data" => self.handle_stream_data(payload).await,
            "stream.close" => self.handle_stream_close(payload).await,
            _ => Err(format!("unsupported relay frame kind {kind}")),
        }
    }

    /// Drops every in-flight request and stream. Called when a device says
    /// hello again: the browser that opened them is gone.
    pub(crate) fn reset(&mut self) {
        self.pending.clear();
        for (_, handle) in self.streams.drain() {
            handle.task.abort();
        }
    }

    async fn handle_http_request(&mut self, payload: &Map<String, Value>) -> Result<(), String> {
        let request_id = identifier(payload, "request_id")?;
        let method = payload
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_ascii_uppercase)
            .filter(|method| {
                !method.is_empty()
                    && method.len() <= 16
                    && method.bytes().all(|byte| byte.is_ascii_alphabetic())
            })
            .ok_or("http.request needs a method")?;
        let path = payload
            .get("path")
            .and_then(Value::as_str)
            .ok_or("http.request needs a path")?
            .to_owned();
        let headers = payload
            .get("headers")
            .and_then(Value::as_object)
            .map(|headers| {
                headers
                    .iter()
                    .filter_map(|(name, value)| {
                        value
                            .as_str()
                            .map(|value| (name.to_ascii_lowercase(), value.to_owned()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let body_chunks = payload
            .get("body_chunks")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        if body_chunks > MAX_BODY_BYTES.div_ceil(CHUNK_BYTES) {
            return self
                .respond_rejection(
                    &request_id,
                    Rejection {
                        status: 413,
                        error: "request body too large",
                    },
                )
                .await;
        }
        let head = RequestHead {
            method,
            path,
            headers,
            body_chunks,
        };
        if body_chunks == 0 {
            self.dispatch(request_id, head, Vec::new()).await;
            return Ok(());
        }
        if self.pending.len() >= MAX_PENDING_REQUESTS {
            return self
                .respond_rejection(
                    &request_id,
                    Rejection {
                        status: 429,
                        error: "too many requests in flight",
                    },
                )
                .await;
        }
        self.pending.insert(
            request_id,
            PendingRequest {
                head,
                chunks: (0..body_chunks).map(|_| None).collect(),
                received: 0,
            },
        );
        Ok(())
    }

    async fn handle_http_body(&mut self, payload: &Map<String, Value>) -> Result<(), String> {
        let request_id = identifier(payload, "request_id")?;
        let index = payload
            .get("index")
            .and_then(Value::as_u64)
            .ok_or("http.body needs an index")? as usize;
        let data = decode_chunk(payload)?;
        let pending = self
            .pending
            .get_mut(&request_id)
            .ok_or("http.body for an unknown request")?;
        let slot = pending
            .chunks
            .get_mut(index)
            .ok_or("http.body index out of range")?;
        if slot.is_none() {
            pending.received += 1;
        }
        *slot = Some(data);
        if pending.received < pending.head.body_chunks {
            return Ok(());
        }
        let pending = self
            .pending
            .remove(&request_id)
            .ok_or("http.body for an unknown request")?;
        let body: Vec<u8> = pending.chunks.into_iter().flatten().flatten().collect();
        if body.len() > MAX_BODY_BYTES {
            return self
                .respond_rejection(
                    &request_id,
                    Rejection {
                        status: 413,
                        error: "request body too large",
                    },
                )
                .await;
        }
        self.dispatch(request_id, pending.head, body).await;
        Ok(())
    }

    async fn dispatch(&self, request_id: String, head: RequestHead, body: Vec<u8>) {
        let (url, auth) = match resolve_http_target(&self.targets, &head.path) {
            Ok(target) => target,
            Err(rejection) => {
                let _ = self.respond_rejection(&request_id, rejection).await;
                return;
            }
        };
        let token = match (self.issue_token)() {
            Ok(token) => token,
            Err(error) => {
                let _ = self
                    .outbound
                    .send(json!({
                        "kind": "http.response",
                        "payload": {"request_id": request_id, "error": error},
                    }))
                    .await;
                return;
            }
        };
        let client = self.client.clone();
        let outbound = self.outbound.clone();
        tokio::spawn(async move {
            let envelopes = match execute_http(client, url, auth, token, head, body).await {
                Ok(response) => response_frames(&request_id, response),
                Err(error) => vec![json!({
                    "kind": "http.response",
                    "payload": {"request_id": request_id, "error": error},
                })],
            };
            for envelope in envelopes {
                if outbound.send(envelope).await.is_err() {
                    return;
                }
            }
        });
    }

    async fn respond_rejection(
        &self,
        request_id: &str,
        rejection: Rejection,
    ) -> Result<(), String> {
        let response = HttpResponse {
            status: rejection.status,
            headers: vec![("content-type".into(), "application/json".into())],
            body: json!({"error": rejection.error}).to_string().into_bytes(),
        };
        for envelope in response_frames(request_id, response) {
            self.outbound
                .send(envelope)
                .await
                .map_err(|_| "relay session closed".to_owned())?;
        }
        Ok(())
    }

    async fn handle_stream_open(&mut self, payload: &Map<String, Value>) -> Result<(), String> {
        let stream_id = identifier(payload, "stream_id")?;
        let path = payload
            .get("path")
            .and_then(Value::as_str)
            .ok_or("stream.open needs a path")?;
        self.streams
            .retain(|_, handle| !handle.commands.is_closed());
        if self.streams.contains_key(&stream_id) {
            return self
                .send_close(&stream_id, 1008, "stream id already in use")
                .await;
        }
        if self.streams.len() >= MAX_STREAMS {
            return self.send_close(&stream_id, 1013, "too many streams").await;
        }
        let url = match resolve_stream_target(&self.targets, path) {
            Ok(url) => url,
            Err(rejection) => return self.send_close(&stream_id, 1008, rejection.error).await,
        };
        let token = match (self.issue_token)() {
            Ok(token) => token,
            Err(error) => return self.send_close(&stream_id, 1011, &error).await,
        };
        let (commands, command_rx) = mpsc::channel(STREAM_COMMAND_CAPACITY);
        let task = tokio::spawn(run_stream(
            stream_id.clone(),
            url,
            token,
            self.outbound.clone(),
            command_rx,
        ));
        self.streams.insert(
            stream_id,
            StreamHandle {
                commands,
                task,
                inbound: Vec::new(),
            },
        );
        Ok(())
    }

    async fn handle_stream_data(&mut self, payload: &Map<String, Value>) -> Result<(), String> {
        let stream_id = identifier(payload, "stream_id")?;
        let data = decode_chunk(payload)?;
        let last = payload.get("last").and_then(Value::as_bool).unwrap_or(true);
        let binary = match payload.get("encoding").and_then(Value::as_str) {
            Some("binary") => true,
            Some("text") | None => false,
            Some(_) => return Err("stream.data encoding must be text or binary".into()),
        };
        let handle = self
            .streams
            .get_mut(&stream_id)
            .ok_or("stream.data for an unknown stream")?;
        if handle.inbound.len() + data.len() > MAX_BODY_BYTES {
            handle.inbound.clear();
            return Err("stream.data message too large".into());
        }
        handle.inbound.extend_from_slice(&data);
        if !last {
            return Ok(());
        }
        let bytes = std::mem::take(&mut handle.inbound);
        let message = if binary {
            Message::Binary(bytes.into())
        } else {
            Message::Text(
                String::from_utf8(bytes)
                    .map_err(|_| "stream.data text is not UTF-8")?
                    .into(),
            )
        };
        handle
            .commands
            .send(StreamCommand::Send(message))
            .await
            .map_err(|_| "stream already closed".to_owned())
    }

    async fn handle_stream_close(&mut self, payload: &Map<String, Value>) -> Result<(), String> {
        let stream_id = identifier(payload, "stream_id")?;
        if let Some(handle) = self.streams.remove(&stream_id) {
            let _ = handle.commands.send(StreamCommand::Close).await;
        }
        Ok(())
    }

    async fn send_close(&self, stream_id: &str, code: u16, reason: &str) -> Result<(), String> {
        self.outbound
            .send(stream_close_frame(stream_id, code, reason))
            .await
            .map_err(|_| "relay session closed".to_owned())
    }
}

impl Drop for RelayExecutor {
    fn drop(&mut self) {
        for handle in self.streams.values() {
            handle.task.abort();
        }
    }
}

fn identifier(payload: &Map<String, Value>, key: &str) -> Result<String, String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= MAX_ID_LEN)
        .map(str::to_owned)
        .ok_or_else(|| format!("frame needs a {key}"))
}

fn decode_chunk(payload: &Map<String, Value>) -> Result<Vec<u8>, String> {
    let data = payload
        .get("data")
        .and_then(Value::as_str)
        .ok_or("frame needs data")?;
    let bytes = URL_SAFE_NO_PAD
        .decode(data)
        .map_err(|error| format!("invalid chunk encoding: {error}"))?;
    if bytes.len() > CHUNK_BYTES {
        return Err("chunk larger than CHUNK_BYTES".into());
    }
    Ok(bytes)
}

fn valid_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.contains('#')
        && !path.contains("//")
        && !path.bytes().any(|byte| byte <= b' ' || byte == 0x7f)
        && !path
            .split('?')
            .next()
            .unwrap_or_default()
            .split('/')
            .any(|segment| segment == "..")
}

/// Maps a same-origin dashboard path to the local server that serves it.
/// `/api/v1/*` is the Next.js rewrite to the API gateway, everything else
/// under `/api/` is a Next.js route handler. Remote Control management stays
/// on the host: a paired browser cannot pair or revoke other browsers.
fn resolve_http_target(
    targets: &ExecutorTargets,
    path: &str,
) -> Result<(String, AuthStyle), Rejection> {
    if !valid_path(path) {
        return Err(Rejection {
            status: 400,
            error: "invalid path",
        });
    }
    if path == "/api/v1/remote-control" || path.starts_with("/api/v1/remote-control/") {
        return Err(Rejection {
            status: 403,
            error: "Remote Control management is only available on the host",
        });
    }
    if let Some(rest) = path.strip_prefix("/api/v1/") {
        return Ok((format!("{}/v1/{rest}", targets.api_url), AuthStyle::Bearer));
    }
    if path.starts_with("/api/") {
        return Ok((format!("{}{path}", targets.web_url), AuthStyle::Cookie));
    }
    Err(Rejection {
        status: 404,
        error: "path is not relayed",
    })
}

/// Only the two gateway sockets the dashboard opens cross the relay.
fn resolve_stream_target(targets: &ExecutorTargets, path: &str) -> Result<String, Rejection> {
    let allowed = valid_path(path)
        && (path == "/v1/ws/sync"
            || path.starts_with("/v1/ws/sync?")
            || path.starts_with("/v1/ws/terminals/"));
    if !allowed {
        return Err(Rejection {
            status: 404,
            error: "socket path is not relayed",
        });
    }
    let origin = targets
        .api_url
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1);
    Ok(format!("{origin}{path}"))
}

struct HttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

async fn execute_http(
    client: reqwest::Client,
    url: String,
    auth: AuthStyle,
    token: String,
    head: RequestHead,
    body: Vec<u8>,
) -> Result<HttpResponse, String> {
    let method = reqwest::Method::from_bytes(head.method.as_bytes())
        .map_err(|error| format!("invalid method: {error}"))?;
    let mut headers = reqwest::header::HeaderMap::new();
    for (name, value) in &head.headers {
        if REQUEST_HEADERS_NOT_FORWARDED.contains(&name.as_str()) {
            continue;
        }
        if let (Ok(name), Ok(value)) = (
            reqwest::header::HeaderName::from_bytes(name.as_bytes()),
            reqwest::header::HeaderValue::from_str(value),
        ) {
            headers.append(name, value);
        }
    }
    let auth_value = match auth {
        AuthStyle::Bearer => (reqwest::header::AUTHORIZATION, format!("Bearer {token}")),
        AuthStyle::Cookie => (reqwest::header::COOKIE, format!("{SESSION_COOKIE}={token}")),
    };
    headers.insert(
        auth_value.0,
        reqwest::header::HeaderValue::from_str(&auth_value.1).map_err(|error| error.to_string())?,
    );
    let mut request = client.request(method, url).headers(headers);
    if !body.is_empty() {
        request = request.body(body);
    }
    let response = request.send().await.map_err(|error| error.to_string())?;
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .filter(|(name, _)| !RESPONSE_HEADERS_NOT_FORWARDED.contains(&name.as_str()))
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_owned(), value.to_owned()))
        })
        .collect();
    if response
        .content_length()
        .is_some_and(|length| length > MAX_BODY_BYTES as u64)
    {
        return Err("response body too large".into());
    }
    let body = response.bytes().await.map_err(|error| error.to_string())?;
    if body.len() > MAX_BODY_BYTES {
        return Err("response body too large".into());
    }
    Ok(HttpResponse {
        status,
        headers,
        body: body.to_vec(),
    })
}

fn response_frames(request_id: &str, response: HttpResponse) -> Vec<Value> {
    let chunks: Vec<&[u8]> = response.body.chunks(CHUNK_BYTES).collect();
    let headers: Map<String, Value> = response
        .headers
        .into_iter()
        .map(|(name, value)| (name, Value::String(value)))
        .collect();
    let mut frames = vec![json!({
        "kind": "http.response",
        "payload": {
            "request_id": request_id,
            "status": response.status,
            "headers": headers,
            "body_chunks": chunks.len(),
        },
    })];
    frames.extend(chunks.iter().enumerate().map(|(index, chunk)| {
        json!({
            "kind": "http.body",
            "payload": {
                "request_id": request_id,
                "index": index,
                "data": URL_SAFE_NO_PAD.encode(chunk),
            },
        })
    }));
    frames
}

fn stream_close_frame(stream_id: &str, code: u16, reason: &str) -> Value {
    json!({
        "kind": "stream.close",
        "payload": {"stream_id": stream_id, "code": code, "reason": reason},
    })
}

fn stream_data_frames(stream_id: &str, binary: bool, bytes: &[u8]) -> Vec<Value> {
    let encoding = if binary { "binary" } else { "text" };
    if bytes.is_empty() {
        return vec![json!({
            "kind": "stream.data",
            "payload": {"stream_id": stream_id, "encoding": encoding, "data": "", "last": true},
        })];
    }
    let chunks: Vec<&[u8]> = bytes.chunks(CHUNK_BYTES).collect();
    let count = chunks.len();
    chunks
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| {
            json!({
                "kind": "stream.data",
                "payload": {
                    "stream_id": stream_id,
                    "encoding": encoding,
                    "data": URL_SAFE_NO_PAD.encode(chunk),
                    "last": index + 1 == count,
                },
            })
        })
        .collect()
}

async fn run_stream(
    stream_id: String,
    url: String,
    token: String,
    outbound: mpsc::Sender<Value>,
    mut commands: mpsc::Receiver<StreamCommand>,
) {
    let socket = match open_stream(&url, &token).await {
        Ok(socket) => socket,
        Err(error) => {
            let _ = outbound
                .send(stream_close_frame(&stream_id, 1006, &error))
                .await;
            return;
        }
    };
    if outbound
        .send(json!({"kind": "stream.opened", "payload": {"stream_id": stream_id}}))
        .await
        .is_err()
    {
        return;
    }
    let (mut sink, mut source) = socket.split();
    let mut close = (1006u16, "socket closed".to_owned());
    loop {
        tokio::select! {
            incoming = source.next() => {
                let frames = match incoming {
                    Some(Ok(Message::Text(text))) => stream_data_frames(&stream_id, false, text.as_bytes()),
                    Some(Ok(Message::Binary(bytes))) => stream_data_frames(&stream_id, true, &bytes),
                    Some(Ok(Message::Ping(bytes))) => {
                        if sink.send(Message::Pong(bytes)).await.is_err() { break; }
                        continue;
                    }
                    Some(Ok(Message::Close(frame))) => {
                        if let Some(frame) = frame {
                            close = (u16::from(frame.code), frame.reason.to_string());
                        } else {
                            close = (1000, String::new());
                        }
                        break;
                    }
                    Some(Ok(_)) => continue,
                    Some(Err(error)) => { close = (1006, error.to_string()); break; }
                    None => { close = (1000, String::new()); break; }
                };
                for frame in frames {
                    if outbound.send(frame).await.is_err() { return; }
                }
            }
            command = commands.recv() => {
                match command {
                    Some(StreamCommand::Send(message)) => {
                        if sink.send(message).await.is_err() { break; }
                    }
                    Some(StreamCommand::Close) | None => {
                        let _ = sink.send(Message::Close(None)).await;
                        return;
                    }
                }
            }
        }
    }
    let _ = outbound
        .send(stream_close_frame(&stream_id, close.0, &close.1))
        .await;
}

async fn open_stream(
    url: &str,
    token: &str,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    String,
> {
    let mut request = url
        .into_client_request()
        .map_err(|error| error.to_string())?;
    request.headers_mut().insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).map_err(|error| error.to_string())?,
    );
    let (socket, _) = connect_async(request)
        .await
        .map_err(|error| error.to_string())?;
    Ok(socket)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::Bytes,
        extract::{
            Query, WebSocketUpgrade,
            ws::{self, WebSocket},
        },
        http::HeaderMap,
        response::IntoResponse,
        routing::{get, post},
    };
    use std::sync::Arc;

    async fn echo_headers(
        headers: HeaderMap,
        Query(query): Query<Map<String, Value>>,
    ) -> axum::Json<Value> {
        let header = |name: &str| {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        axum::Json(json!({
            "authorization": header("authorization"),
            "cookie": header("cookie"),
            "x-trace-id": header("x-trace-id"),
            "content-type": header("content-type"),
            "query": query,
        }))
    }

    async fn echo_body(body: Bytes) -> impl IntoResponse {
        (
            [
                ("content-type", "application/octet-stream"),
                ("set-cookie", "leak=1"),
            ],
            body,
        )
    }

    async fn ws_echo(headers: HeaderMap, upgrade: WebSocketUpgrade) -> impl IntoResponse {
        let authorization = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        upgrade.on_upgrade(move |mut socket: WebSocket| async move {
            if socket
                .send(ws::Message::Text(format!("ready {authorization}").into()))
                .await
                .is_err()
            {
                return;
            }
            while let Some(Ok(message)) = socket.recv().await {
                match message {
                    ws::Message::Text(text) => {
                        if socket.send(ws::Message::Text(text)).await.is_err() {
                            return;
                        }
                    }
                    ws::Message::Binary(bytes) => {
                        if socket.send(ws::Message::Binary(bytes)).await.is_err() {
                            return;
                        }
                    }
                    ws::Message::Close(_) => return,
                    _ => {}
                }
            }
        })
    }

    async fn spawn_local_server() -> String {
        let app = Router::new()
            .route("/v1/echo", get(echo_headers))
            .route("/v1/big", post(echo_body))
            .route("/api/web", get(echo_headers))
            .route("/v1/ws/sync", get(ws_echo))
            .route("/v1/ws/terminals/{binding_id}", get(ws_echo));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let address = listener.local_addr().expect("local address");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        format!("http://{address}")
    }

    async fn executor() -> (RelayExecutor, mpsc::Receiver<Value>) {
        let origin = spawn_local_server().await;
        let (tx, rx) = mpsc::channel(256);
        let executor = RelayExecutor::new(
            ExecutorTargets {
                api_url: origin.clone(),
                web_url: origin,
            },
            Arc::new(|| Ok("t0k".to_owned())),
            tx,
        )
        .expect("executor");
        (executor, rx)
    }

    async fn next_frame(rx: &mut mpsc::Receiver<Value>) -> Value {
        tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("frame within 5s")
            .expect("channel open")
    }

    struct Collected {
        status: Option<u16>,
        error: Option<String>,
        headers: Map<String, Value>,
        body: Vec<u8>,
    }

    async fn collect_response(rx: &mut mpsc::Receiver<Value>, request_id: &str) -> Collected {
        let head = next_frame(rx).await;
        assert_eq!(head["kind"], "http.response");
        assert_eq!(head["payload"]["request_id"], request_id);
        if let Some(error) = head["payload"]["error"].as_str() {
            return Collected {
                status: None,
                error: Some(error.to_owned()),
                headers: Map::new(),
                body: Vec::new(),
            };
        }
        let chunks = head["payload"]["body_chunks"].as_u64().unwrap() as usize;
        let mut body = Vec::new();
        for index in 0..chunks {
            let frame = next_frame(rx).await;
            assert_eq!(frame["kind"], "http.body");
            assert_eq!(frame["payload"]["index"], index);
            body.extend(
                URL_SAFE_NO_PAD
                    .decode(frame["payload"]["data"].as_str().unwrap())
                    .unwrap(),
            );
        }
        Collected {
            status: head["payload"]["status"]
                .as_u64()
                .map(|status| status as u16),
            error: None,
            headers: head["payload"]["headers"]
                .as_object()
                .cloned()
                .unwrap_or_default(),
            body,
        }
    }

    fn request(request_id: &str, method: &str, path: &str, body: &[u8]) -> Vec<Map<String, Value>> {
        let chunks: Vec<&[u8]> = body.chunks(CHUNK_BYTES).collect();
        let mut frames = vec![
            json!({
                "request_id": request_id,
                "method": method,
                "path": path,
                "headers": {"Content-Type": "application/json", "Authorization": "Bearer forged", "Cookie": "x=y", "X-Trace-Id": "trace-1"},
                "body_chunks": chunks.len(),
            })
            .as_object()
            .cloned()
            .unwrap(),
        ];
        frames.extend(chunks.iter().enumerate().map(|(index, chunk)| {
            json!({"request_id": request_id, "index": index, "data": URL_SAFE_NO_PAD.encode(chunk)})
                .as_object()
                .cloned()
                .unwrap()
        }));
        frames
    }

    async fn send_request(executor: &mut RelayExecutor, frames: Vec<Map<String, Value>>) {
        let mut frames = frames.into_iter();
        executor
            .handle("http.request", &frames.next().unwrap())
            .await
            .expect("request accepted");
        for frame in frames {
            executor
                .handle("http.body", &frame)
                .await
                .expect("body accepted");
        }
    }

    #[tokio::test]
    async fn api_calls_carry_the_host_token_and_drop_client_credentials() {
        let (mut executor, mut rx) = executor().await;
        send_request(&mut executor, request("r1", "get", "/api/v1/echo?x=1", b"")).await;
        let response = collect_response(&mut rx, "r1").await;
        assert_eq!(response.status, Some(200));
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(body["authorization"], "Bearer t0k");
        assert_eq!(body["cookie"], Value::Null);
        assert_eq!(body["x-trace-id"], "trace-1");
        assert_eq!(body["query"]["x"], "1");
        assert_eq!(response.headers["content-type"], "application/json");
    }

    #[tokio::test]
    async fn web_route_handlers_get_the_session_cookie() {
        let (mut executor, mut rx) = executor().await;
        send_request(&mut executor, request("r2", "GET", "/api/web", b"")).await;
        let response = collect_response(&mut rx, "r2").await;
        assert_eq!(response.status, Some(200));
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(body["cookie"], "choruz_session=t0k");
        assert_eq!(body["authorization"], Value::Null);
    }

    #[tokio::test]
    async fn bodies_larger_than_a_chunk_round_trip_in_both_directions() {
        let (mut executor, mut rx) = executor().await;
        let payload: Vec<u8> = (0..(CHUNK_BYTES * 2 + 12345))
            .map(|i| (i % 251) as u8)
            .collect();
        send_request(
            &mut executor,
            request("r3", "POST", "/api/v1/big", &payload),
        )
        .await;
        let response = collect_response(&mut rx, "r3").await;
        assert_eq!(response.status, Some(200));
        assert_eq!(response.body, payload);
        assert_eq!(response.headers["content-type"], "application/octet-stream");
        assert!(!response.headers.contains_key("set-cookie"));
        assert!(executor.pending.is_empty());
    }

    #[tokio::test]
    async fn paths_outside_the_dashboard_are_rejected_without_a_request() {
        let (mut executor, mut rx) = executor().await;
        for (id, path, status) in [
            ("d1", "/v1/echo", 404),
            ("d2", "/api/v1/remote-control/devices", 403),
            ("d3", "/api/v1/../v1/echo", 400),
            ("d4", "api/v1/echo", 400),
            ("d5", "/api/v1/echo\r\nX: y", 400),
        ] {
            send_request(&mut executor, request(id, "GET", path, b"")).await;
            let response = collect_response(&mut rx, id).await;
            assert_eq!(response.status, Some(status), "{path}");
            assert_eq!(response.error, None);
        }
    }

    #[tokio::test]
    async fn unreachable_targets_report_an_error_instead_of_a_status() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut executor = RelayExecutor::new(
            ExecutorTargets {
                api_url: "http://127.0.0.1:9".into(),
                web_url: "http://127.0.0.1:9".into(),
            },
            Arc::new(|| Ok("t0k".to_owned())),
            tx,
        )
        .unwrap();
        send_request(&mut executor, request("u1", "GET", "/api/v1/echo", b"")).await;
        let response = collect_response(&mut rx, "u1").await;
        assert_eq!(response.status, None);
        assert!(response.error.is_some());
    }

    async fn open_stream_and_wait_ready(
        executor: &mut RelayExecutor,
        rx: &mut mpsc::Receiver<Value>,
        stream_id: &str,
        path: &str,
    ) {
        executor
            .handle(
                "stream.open",
                json!({"stream_id": stream_id, "path": path})
                    .as_object()
                    .unwrap(),
            )
            .await
            .unwrap();
        let opened = next_frame(rx).await;
        assert_eq!(opened["kind"], "stream.opened");
        assert_eq!(opened["payload"]["stream_id"], stream_id);
        let ready = next_frame(rx).await;
        assert_eq!(ready["kind"], "stream.data");
        assert_eq!(ready["payload"]["encoding"], "text");
        assert_eq!(ready["payload"]["last"], true);
        let text = URL_SAFE_NO_PAD
            .decode(ready["payload"]["data"].as_str().unwrap())
            .unwrap();
        assert_eq!(text, b"ready Bearer t0k");
    }

    #[tokio::test]
    async fn sockets_are_multiplexed_with_text_and_binary_chunks() {
        let (mut executor, mut rx) = executor().await;
        open_stream_and_wait_ready(
            &mut executor,
            &mut rx,
            "s1",
            "/v1/ws/sync?device_id=d&cursor=0",
        )
        .await;
        open_stream_and_wait_ready(
            &mut executor,
            &mut rx,
            "s2",
            "/v1/ws/terminals/b1?cols=80&rows=24",
        )
        .await;

        executor
            .handle(
                "stream.data",
                json!({"stream_id": "s1", "encoding": "text", "data": URL_SAFE_NO_PAD.encode(b"{\"type\":\"sync_ack\"}")})
                    .as_object()
                    .unwrap(),
            )
            .await
            .unwrap();
        let echoed = next_frame(&mut rx).await;
        assert_eq!(echoed["payload"]["stream_id"], "s1");
        assert_eq!(echoed["payload"]["encoding"], "text");
        assert_eq!(
            URL_SAFE_NO_PAD
                .decode(echoed["payload"]["data"].as_str().unwrap())
                .unwrap(),
            b"{\"type\":\"sync_ack\"}"
        );

        let big: Vec<u8> = (0..(CHUNK_BYTES + 7)).map(|i| (i % 253) as u8).collect();
        for (index, chunk) in big.chunks(CHUNK_BYTES).enumerate() {
            executor
                .handle(
                    "stream.data",
                    json!({
                        "stream_id": "s2",
                        "encoding": "binary",
                        "data": URL_SAFE_NO_PAD.encode(chunk),
                        "last": index == 1,
                    })
                    .as_object()
                    .unwrap(),
                )
                .await
                .unwrap();
        }
        let mut echoed = Vec::new();
        loop {
            let frame = next_frame(&mut rx).await;
            assert_eq!(frame["kind"], "stream.data");
            assert_eq!(frame["payload"]["stream_id"], "s2");
            assert_eq!(frame["payload"]["encoding"], "binary");
            echoed.extend(
                URL_SAFE_NO_PAD
                    .decode(frame["payload"]["data"].as_str().unwrap())
                    .unwrap(),
            );
            if frame["payload"]["last"] == true {
                break;
            }
        }
        assert_eq!(echoed, big);

        executor
            .handle(
                "stream.close",
                json!({"stream_id": "s1"}).as_object().unwrap(),
            )
            .await
            .unwrap();
        assert!(!executor.streams.contains_key("s1"));
        let error = executor
            .handle(
                "stream.data",
                json!({"stream_id": "s1", "data": ""}).as_object().unwrap(),
            )
            .await
            .unwrap_err();
        assert!(error.contains("unknown stream"));
    }

    #[tokio::test]
    async fn only_the_dashboard_sockets_are_relayed() {
        let (mut executor, mut rx) = executor().await;
        for path in [
            "/v1/ws/other",
            "/api/v1/ws/sync",
            "/v1/ws/sync/../terminals/x",
        ] {
            executor
                .handle(
                    "stream.open",
                    json!({"stream_id": "bad", "path": path})
                        .as_object()
                        .unwrap(),
                )
                .await
                .unwrap();
            let closed = next_frame(&mut rx).await;
            assert_eq!(closed["kind"], "stream.close", "{path}");
            assert_eq!(closed["payload"]["code"], 1008);
            assert!(!executor.streams.contains_key("bad"));
        }
    }

    #[tokio::test]
    async fn reset_drops_streams_and_pending_bodies() {
        let (mut executor, mut rx) = executor().await;
        open_stream_and_wait_ready(&mut executor, &mut rx, "s1", "/v1/ws/sync").await;
        let frames = request("p1", "POST", "/api/v1/big", &[1u8; CHUNK_BYTES + 1]);
        executor.handle("http.request", &frames[0]).await.unwrap();
        assert_eq!(executor.pending.len(), 1);
        executor.reset();
        assert!(executor.pending.is_empty());
        assert!(executor.streams.is_empty());
        let error = executor.handle("http.body", &frames[1]).await.unwrap_err();
        assert!(error.contains("unknown request"));
    }
}
