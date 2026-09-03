//! Server-owned host side of the one-time Remote Control pairing handshake.
//!
//! The API process owns this socket so a caller can request a credential and
//! exit without invalidating it. After the browser proves possession of the
//! credential, the regular `remote_control_bridge` owns the durable session.

use std::time::Duration;

use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::{SinkExt, StreamExt};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use p256::{
    EncodedPoint, PublicKey, SecretKey, ecdh::diffie_hellman, elliptic_curve::sec1::ToEncodedPoint,
};
use rand::RngCore;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio_tungstenite::{WebSocketStream, connect_async, tungstenite::Message};

use crate::{
    ApiState,
    handlers_remote_control::{
        RedeemPairingRequest, existing_session_key, redeem_pairing_for_principal,
    },
};

type GatewaySocket = WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub(crate) struct PairingHost {
    pub(crate) pairing_id: String,
    pub(crate) principal_id: String,
    pub(crate) credential: String,
    pub(crate) gateway_url: String,
    pub(crate) gateway_ticket: String,
    pub(crate) expires_at: chrono::DateTime<chrono::Utc>,
}

pub(crate) async fn connect_and_spawn(state: ApiState, host: PairingHost) -> Result<(), String> {
    install_tls_provider()?;
    let endpoint = socket_url(&host.gateway_url, &host.gateway_ticket)?;
    let (socket, _) = connect_async(endpoint)
        .await
        .map_err(|error| format!("connect to Cloud Gateway: {error}"))?;
    tracing::info!(pairing_id = %host.pairing_id, "remote-control host socket opened");
    let pairing_id = host.pairing_id.clone();
    tokio::spawn(async move {
        let remaining = (host.expires_at - chrono::Utc::now())
            .to_std()
            .unwrap_or(Duration::ZERO);
        let result = tokio::time::timeout(remaining, maintain_pairing(state, host, socket)).await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::warn!(%pairing_id, %error, "remote-control pairing host failed")
            }
            Err(_) => {
                tracing::info!(%pairing_id, "remote-control pairing credential expired")
            }
        }
    });
    Ok(())
}

async fn maintain_pairing(
    state: ApiState,
    host: PairingHost,
    socket: GatewaySocket,
) -> Result<(), String> {
    let endpoint = socket_url(&host.gateway_url, &host.gateway_ticket)?;
    let mut attempt = 0u32;
    let mut socket = Some(socket);
    loop {
        if let Some(connected) = socket.take() {
            match serve_pairing(&state, &host, connected).await {
                Ok(()) => return Ok(()),
                Err(error) => {
                    tracing::warn!(pairing_id = %host.pairing_id, %error, "remote-control pairing socket disconnected");
                }
            }
        }
        tokio::time::sleep(reconnect_delay(attempt)).await;
        match connect_async(&endpoint).await {
            Ok((next_socket, _)) => {
                socket = Some(next_socket);
                attempt = 0;
                tracing::info!(pairing_id = %host.pairing_id, "remote-control pairing host reconnected");
            }
            Err(error) => {
                attempt = attempt.saturating_add(1);
                tracing::warn!(pairing_id = %host.pairing_id, %error, "remote-control pairing host reconnect failed");
            }
        }
    }
}

fn reconnect_delay(attempt: u32) -> Duration {
    Duration::from_secs(2u64.saturating_pow(attempt.min(3)).min(5))
}

fn install_tls_provider() -> Result<(), String> {
    if rustls::crypto::CryptoProvider::get_default().is_some() {
        return Ok(());
    }
    if rustls::crypto::ring::default_provider()
        .install_default()
        .is_ok()
        || rustls::crypto::CryptoProvider::get_default().is_some()
    {
        Ok(())
    } else {
        Err("initialize TLS cryptography provider".into())
    }
}

async fn serve_pairing(
    state: &ApiState,
    host: &PairingHost,
    mut socket: GatewaySocket,
) -> Result<(), String> {
    let secret = SecretKey::random(&mut rand::rngs::OsRng);
    let host_public_key = public_jwk(secret.public_key());
    let host_nonce = random_base64(16);
    let host_commitment = pairing_commitment(&host_public_key, &host_nonce);
    let credential_secret = pairing_credential_secret(&host.credential)?;
    let mut device: Option<(String, String)> = None;
    let mut pairing_secret_value: Option<String> = None;
    let mut device_public_key_value: Option<String> = None;

    while let Some(frame) = socket.next().await {
        let frame = frame.map_err(|error| format!("read Cloud Gateway: {error}"))?;
        let Message::Text(text) = frame else { continue };
        let message: Value = serde_json::from_str(&text)
            .map_err(|error| format!("decode Cloud Gateway frame: {error}"))?;
        match message["kind"].as_str() {
            Some("pair.commit") if device.is_none() => {
                let commitment = message["device_commitment"]
                    .as_str()
                    .ok_or("remote browser did not commit a pairing key")?;
                let name = message["device_name"].as_str().unwrap_or("Web browser");
                device = Some((name.to_owned(), commitment.to_owned()));
                socket
                    .send(Message::Text(
                        json!({ "kind": "pair.commit", "host_commitment": host_commitment })
                            .to_string()
                            .into(),
                    ))
                    .await
                    .map_err(|error| format!("send pairing commitment: {error}"))?;
            }
            Some("pair.reveal") if device.is_some() => {
                let (_, expected_commitment) = device.as_ref().expect("checked above");
                let device_public_key = message["device_public_key"]
                    .as_str()
                    .ok_or("remote browser did not reveal a public key")?;
                let device_nonce = message["device_nonce"]
                    .as_str()
                    .ok_or("remote browser did not reveal a nonce")?;
                if pairing_commitment(device_public_key, device_nonce) != *expected_commitment {
                    return Err("remote browser pairing commitment did not match".into());
                }
                let derived = pairing_secret(&secret, device_public_key, credential_secret)?;
                let host_proof =
                    pairing_proof(&derived, "host", &host_public_key, device_public_key)?;
                socket
                    .send(Message::Text(
                        json!({
                            "kind": "pair.reveal",
                            "host_public_key": host_public_key,
                            "host_nonce": host_nonce,
                            "host_proof": host_proof,
                        })
                        .to_string()
                        .into(),
                    ))
                    .await
                    .map_err(|error| format!("reveal pairing key: {error}"))?;
                pairing_secret_value = Some(derived);
                device_public_key_value = Some(device_public_key.to_owned());
            }
            Some("pair.proof")
                if pairing_secret_value.is_some() && device_public_key_value.is_some() =>
            {
                let derived = pairing_secret_value.as_ref().expect("checked above");
                let device_public_key = device_public_key_value.as_ref().expect("checked above");
                let expected =
                    pairing_proof(derived, "device", &host_public_key, device_public_key)?;
                if message["device_proof"].as_str() != Some(&expected) {
                    return Err(
                        "remote browser did not prove possession of the pairing credential".into(),
                    );
                }
                let (device_name, _) = device.take().expect("pairing device exists");
                let session_key = existing_session_key(state, &host.principal_id)
                    .await
                    .map_err(|error| error.0.to_string())?
                    .unwrap_or_else(|| derived.clone());
                let credentials = redeem_pairing_for_principal(
                    state,
                    host.principal_id.clone(),
                    RedeemPairingRequest {
                        credential: host.credential.clone(),
                        device_name,
                        session_key: session_key.clone(),
                    },
                )
                .await
                .map_err(|error| error.0.to_string())?;
                let complete = encrypt_pairing_payload(
                    derived,
                    &json!({
                        "device_id": credentials.device_id,
                        "gateway_url": credentials.gateway_url,
                        "gateway_ticket": credentials.gateway_ticket,
                        "session_key": session_key,
                    }),
                )?;
                socket
                    .send(Message::Text(
                        json!({
                            "kind": "pair.complete",
                            "iv": complete.0,
                            "ciphertext": complete.1,
                        })
                        .to_string()
                        .into(),
                    ))
                    .await
                    .map_err(|error| format!("complete pairing: {error}"))?;
                let _ = socket.close(None).await;
                tracing::info!(pairing_id = %host.pairing_id, "remote-control pairing completed");
                return Ok(());
            }
            _ => {}
        }
    }
    Err("Cloud Gateway closed before pairing completed".into())
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

fn random_base64(bytes: usize) -> String {
    let mut output = vec![0; bytes];
    rand::rngs::OsRng.fill_bytes(&mut output);
    URL_SAFE_NO_PAD.encode(output)
}

fn public_jwk(public_key: PublicKey) -> String {
    let point = public_key.to_encoded_point(false);
    serde_json::to_string(&json!({
        "kty": "EC",
        "crv": "P-256",
        "x": URL_SAFE_NO_PAD.encode(point.x().expect("uncompressed x")),
        "y": URL_SAFE_NO_PAD.encode(point.y().expect("uncompressed y")),
        "ext": true,
        "key_ops": [],
    }))
    .expect("JWK serializes")
}

fn peer_public_key(jwk: &str) -> Result<PublicKey, String> {
    let value: Value =
        serde_json::from_str(jwk).map_err(|_| "remote browser public key was not JSON")?;
    if value["kty"].as_str() != Some("EC") || value["crv"].as_str() != Some("P-256") {
        return Err("remote browser public key is not P-256".into());
    }
    let x = URL_SAFE_NO_PAD
        .decode(
            value["x"]
                .as_str()
                .ok_or("remote browser public key has no x")?,
        )
        .map_err(|_| "remote browser public key has invalid x")?;
    let y = URL_SAFE_NO_PAD
        .decode(
            value["y"]
                .as_str()
                .ok_or("remote browser public key has no y")?,
        )
        .map_err(|_| "remote browser public key has invalid y")?;
    if x.len() != 32 || y.len() != 32 {
        return Err("remote browser public key has invalid coordinates".into());
    }
    let point =
        EncodedPoint::from_affine_coordinates(x.as_slice().into(), y.as_slice().into(), false);
    PublicKey::from_sec1_bytes(point.as_bytes())
        .map_err(|_| "remote browser public key is invalid".into())
}

fn pairing_credential_secret(credential: &str) -> Result<&str, String> {
    let mut parts = credential.split('.');
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
    Ok(secret)
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

fn encrypt_pairing_payload(secret: &str, payload: &Value) -> Result<(String, String), String> {
    let key = URL_SAFE_NO_PAD
        .decode(secret)
        .map_err(|_| "invalid pairing key")?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| "initialize pairing cipher")?;
    let mut iv = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut iv);
    let plaintext = serde_json::to_vec(payload)
        .map_err(|error| format!("encode pairing credentials: {error}"))?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&iv), plaintext.as_ref())
        .map_err(|_| "encrypt pairing credentials")?;
    Ok((
        URL_SAFE_NO_PAD.encode(iv),
        URL_SAFE_NO_PAD.encode(ciphertext),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_credential_requires_two_independent_base64url_values() {
        assert_eq!(
            pairing_credential_secret("v1.AAAAAAAAAAAAAAAAAAAAAA.BBBBBBBBBBBBBBBBBBBBBB").unwrap(),
            "BBBBBBBBBBBBBBBBBBBBBB"
        );
        assert!(pairing_credential_secret("12345678").is_err());
    }

    #[test]
    fn pairing_proofs_are_bound_to_each_role() {
        let secret = URL_SAFE_NO_PAD.encode([7u8; 32]);
        let host = pairing_proof(&secret, "host", "host-key", "device-key").unwrap();
        let device = pairing_proof(&secret, "device", "host-key", "device-key").unwrap();
        assert_ne!(host, device);
    }
}
