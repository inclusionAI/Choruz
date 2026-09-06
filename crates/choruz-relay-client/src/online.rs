//! Authenticated Online mail transport. It carries sealed messages, not HTTP or terminal commands.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};

use super::{decrypt, encrypt, install_tls_provider};

pub struct Credentials {
    pub origin: String,
    pub account: String,
    pub token: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Connecting,
    Connected,
    Reconnecting,
    Expired,
    Replaced,
    Stopped,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Delivery {
    pub id: String,
    pub channel: String,
    pub sender: String,
    pub envelope: Value,
}

#[derive(Debug)]
pub enum Event {
    Delivery(Delivery),
    Accepted(String),
    Rejected { id: String, reason: String },
    Revoked(String),
}

#[derive(Clone)]
pub struct Client {
    outbound: mpsc::Sender<Value>,
    pub status: watch::Receiver<Status>,
}

impl Client {
    /// The receiver must durably process a delivery before calling `acknowledge`.
    pub fn start(credentials: Credentials) -> (Self, mpsc::Receiver<Event>) {
        let (outbound, receiver) = mpsc::channel(64);
        let (events, inbound) = mpsc::channel(64);
        let (status, state) = watch::channel(Status::Connecting);
        tokio::spawn(run(credentials, receiver, events, status));
        (
            Self {
                outbound,
                status: state,
            },
            inbound,
        )
    }

    pub async fn send(
        &self,
        id: &str,
        channel: &str,
        recipient: &str,
        sender: &str,
        key: &str,
        body: Value,
    ) -> Result<(), String> {
        let inner =
            json!({"id":id,"channel":channel,"recipient":recipient,"sender":sender,"body":body});
        if inner.to_string().len() > 600_000 {
            return Err("Online message is too large".into());
        }
        let envelope = encrypt(key, &inner)?;
        self.outbound.send(json!({"kind":"send","id":id,"channel":channel,"recipient":recipient,"envelope":envelope})).await.map_err(|_| "Online connection stopped".into())
    }

    pub async fn acknowledge(&self, id: &str) -> Result<(), String> {
        self.outbound
            .send(json!({"kind":"ack","id":id}))
            .await
            .map_err(|_| "Online connection stopped".into())
    }
}

impl Delivery {
    /// Bind the ciphertext to the authenticated route, preventing cross-link replay.
    pub fn open(&self, key: &str, recipient: &str) -> Result<Value, String> {
        let inner = decrypt(
            key,
            self.envelope["iv"].as_str().ok_or("Missing IV")?,
            self.envelope["ciphertext"]
                .as_str()
                .ok_or("Missing ciphertext")?,
        )?;
        if inner["id"] != self.id
            || inner["channel"] != self.channel
            || inner["sender"] != self.sender
            || inner["recipient"] != recipient
        {
            return Err("Online encrypted route mismatch".into());
        }
        Ok(inner["body"].clone())
    }
}

async fn run(
    credentials: Credentials,
    mut outbound: mpsc::Receiver<Value>,
    events: mpsc::Sender<Event>,
    status: watch::Sender<Status>,
) {
    if install_tls_provider().is_err() {
        let _ = status.send(Status::Stopped);
        return;
    }
    let mut attempt = 0u32;
    let mut pending = None;
    loop {
        if events.is_closed() || outbound.is_closed() {
            break;
        }
        let result = connection(&credentials, &mut outbound, &events, &status, &mut pending).await;
        match result {
            Ok(Some(terminal)) => {
                let _ = status.send(terminal);
                return;
            }
            Ok(None) => break,
            Err(()) => {
                let _ = status.send(Status::Reconnecting);
                let delay = Duration::from_secs(1u64 << attempt.min(5));
                tokio::select! { _ = events.closed() => break, _ = tokio::time::sleep(delay) => {} }
                attempt = attempt.saturating_add(1);
            }
        }
    }
    let _ = status.send(Status::Stopped);
}

async fn connection(
    credentials: &Credentials,
    outbound: &mut mpsc::Receiver<Value>,
    events: &mpsc::Sender<Event>,
    status: &watch::Sender<Status>,
    pending: &mut Option<Value>,
) -> Result<Option<Status>, ()> {
    let mut url = reqwest::Url::parse(&credentials.origin).map_err(|_| ())?;
    let scheme = match url.scheme() {
        "https" => "wss",
        "http" if matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")) => "ws",
        _ => return Ok(Some(Status::Stopped)),
    };
    url.set_scheme(scheme).map_err(|_| ())?;
    url.set_path("/v1/online/connect");
    url.set_query(None);
    let mut request = url.as_str().into_client_request().map_err(|_| ())?;
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {}", credentials.token)
            .parse()
            .map_err(|_| ())?,
    );
    let socket = tokio::time::timeout(Duration::from_secs(15), connect_async(request))
        .await
        .map_err(|_| ())?;
    let (mut socket, _) = match socket {
        Ok(value) => value,
        Err(tokio_tungstenite::tungstenite::Error::Http(response))
            if response.status().as_u16() == 401 =>
        {
            return Ok(Some(Status::Expired));
        }
        Err(_) => return Err(()),
    };
    let mut heartbeat = tokio::time::interval(Duration::from_secs(20));
    let mut last_received = tokio::time::Instant::now();
    let mut ready = false;
    loop {
        if ready && let Some(frame) = pending.as_ref() {
            socket
                .send(Message::Text(frame.to_string().into()))
                .await
                .map_err(|_| ())?;
            *pending = None;
        }
        tokio::select! {
            _ = events.closed() => return Ok(None),
            frame = outbound.recv(), if ready => match frame { Some(frame) => *pending = Some(frame), None => return Ok(None) },
            _ = heartbeat.tick() => {
                if last_received.elapsed() > Duration::from_secs(60) { return Err(()); }
                socket.send(Message::Text(json!({"kind":"ping"}).to_string().into())).await.map_err(|_| ())?;
            },
            frame = socket.next() => {
                let Some(Ok(frame)) = frame else { return Err(()) };
                last_received = tokio::time::Instant::now();
                match frame {
                    Message::Close(Some(close)) if u16::from(close.code) == 4001 => return Ok(Some(Status::Expired)),
                    Message::Close(Some(close)) if u16::from(close.code) == 4009 => return Ok(Some(Status::Replaced)),
                    Message::Close(_) => return Err(()),
                    Message::Text(text) => {
                        let value: Value = serde_json::from_str(&text).map_err(|_| ())?;
                        let event = match value["kind"].as_str() {
                            Some("ready") if value["account_id"] == credentials.account => { ready = true; let _ = status.send(Status::Connected); None },
                            Some("pong") => None,
                            Some("delivery") if ready => Some(Event::Delivery(serde_json::from_value(value).map_err(|_| ())?)),
                            Some("accepted") => Some(Event::Accepted(value["id"].as_str().ok_or(())?.into())),
                            Some("rejected") => Some(Event::Rejected { id: value["id"].as_str().ok_or(())?.into(), reason: value["reason"].as_str().ok_or(())?.into() }),
                            Some("revoked") => Some(Event::Revoked(value["channel"].as_str().ok_or(())?.into())),
                            _ => return Err(()),
                        };
                        if let Some(event) = event { events.send(event).await.map_err(|_| ())?; }
                    },
                    _ => {},
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

    #[test]
    fn sealed_delivery_authenticates_every_routing_field() {
        let key = URL_SAFE_NO_PAD.encode([3u8; 32]);
        let mut delivery = Delivery { id: "message".into(), channel: "channel".into(), sender: "alice".into(), envelope: encrypt(&key, &json!({"id":"message","channel":"channel","sender":"alice","recipient":"bob","body":"hello"})).unwrap() };
        assert_eq!(delivery.open(&key, "bob").unwrap(), "hello");
        assert!(delivery.open(&key, "mallory").is_err());
        assert!(
            delivery
                .open(&URL_SAFE_NO_PAD.encode([4u8; 32]), "bob")
                .is_err()
        );
        delivery.channel = "another-channel".into();
        assert!(delivery.open(&key, "bob").is_err());
    }

    #[tokio::test]
    #[allow(clippy::result_large_err)] // tungstenite fixes the handshake callback's error type.
    async fn reconnects_without_a_browser_and_stops_when_the_session_is_revoked() {
        use tokio_tungstenite::tungstenite::protocol::{CloseFrame, frame::coding::CloseCode};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let key = URL_SAFE_NO_PAD.encode([3u8; 32]);
        let sealed = encrypt(&key, &json!({"id":"message","channel":"channel","sender":"alice","recipient":"bob","body":"hello"})).unwrap();
        let server = tokio::spawn(async move {
            for attempt in 0..2 {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_hdr_async(
                    stream,
                    |request: &tokio_tungstenite::tungstenite::handshake::server::Request,
                     response| {
                        assert_eq!(request.uri().path(), "/v1/online/connect");
                        assert!(request.uri().query().is_none());
                        assert_eq!(request.headers()["authorization"], "Bearer private-session");
                        Ok(response)
                    },
                )
                .await
                .unwrap();
                socket
                    .send(Message::Text(
                        json!({"kind":"ready","account_id":"bob"})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
                if attempt == 0 {
                    socket.close(None).await.unwrap();
                    continue;
                }
                socket.send(Message::Text(json!({"kind":"delivery","id":"message","channel":"channel","sender":"alice","envelope":sealed}).to_string().into())).await.unwrap();
                while let Some(Ok(Message::Text(text))) = socket.next().await {
                    let frame: Value = serde_json::from_str(&text).unwrap();
                    if frame["kind"] == "ack" {
                        assert_eq!(frame["id"], "message");
                        socket
                            .close(Some(CloseFrame {
                                code: CloseCode::Library(4001),
                                reason: "expired".into(),
                            }))
                            .await
                            .unwrap();
                        break;
                    }
                }
            }
        });
        let (mut client, mut events) = Client::start(Credentials {
            origin: format!("http://{address}"),
            account: "bob".into(),
            token: "private-session".into(),
        });
        let event = tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await
            .unwrap()
            .unwrap();
        let Event::Delivery(delivery) = event else {
            panic!("Expected delivery")
        };
        assert_eq!(delivery.open(&key, "bob").unwrap(), "hello");
        client.acknowledge(&delivery.id).await.unwrap();
        tokio::time::timeout(
            Duration::from_secs(5),
            client.status.wait_for(|status| *status == Status::Expired),
        )
        .await
        .unwrap()
        .unwrap();
        server.await.unwrap();
    }
}
