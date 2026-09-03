use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use choruz_common::AppError;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand::{Rng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::HashMap;

use crate::{
    ApiError, ApiState, authenticated_principal,
    remote_control_pairing_host::{self, PairingHost},
    require_human_operator,
};

const PAIRING_TTL_MINUTES: i64 = 5;
const MAX_DEVICE_NAME_LEN: usize = 80;
const SESSION_KEY_LEN: usize = 43;
/// The hosted Gateway is the zero-configuration default.  Operators can set
/// `CHORUZ_REMOTE_CONTROL_GATEWAY_URL` to use their own Worker instead.
const HOSTED_GATEWAY_URL: &str = "https://choruz-remote-control-gateway.jiachengguo778.workers.dev";
const WRAPPED_KEY_PREFIX: &str = "v1";
const WRAPPING_CONTEXT: &[u8] = b"choruz.remote-control.storage.v1";
type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RemoteControlSettings {
    pub(crate) gateway_url: Option<String>,
    pub(crate) gateway_ticket: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct PairingResponse {
    pairing_id: String,
    credential: String,
    expires_at: String,
    gateway_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RedeemPairingRequest {
    pub(crate) credential: String,
    pub(crate) device_name: String,
    pub(crate) session_key: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct RedeemPairingResponse {
    pub(crate) device_id: String,
    pub(crate) gateway_url: Option<String>,
    pub(crate) gateway_ticket: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RemoteControlDevice {
    id: String,
    name: String,
    paired_at: String,
    last_seen_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RemoteControlBridgeConfig {
    settings: RemoteControlSettings,
    session_key: Option<String>,
    revoked_device_ids: Vec<String>,
    transport_session_id: Option<String>,
    host_transport_ticket: Option<String>,
    device_transport_tickets: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub(crate) struct BridgeMaterial {
    pub(crate) gateway_url: String,
    pub(crate) session_key: String,
    pub(crate) host_session_ticket: String,
    pub(crate) revoked_device_ids: Vec<String>,
    pub(crate) transport_session_id: String,
    pub(crate) host_transport_ticket: String,
    pub(crate) device_transport_tickets: HashMap<String, String>,
}

struct StoredBridgeState {
    session_key: Option<String>,
    active_device_ids: Vec<String>,
    revoked_device_ids: Vec<String>,
}

fn validate_device(name: &str, session_key: &str) -> Result<(), ApiError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_DEVICE_NAME_LEN {
        return Err(ApiError(AppError::Validation(
            "device_name must contain 1 to 80 characters".into(),
        )));
    }
    if session_key.len() != SESSION_KEY_LEN
        || !session_key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(ApiError(AppError::Validation(
            "session_key must be a 256-bit base64url value".into(),
        )));
    }
    Ok(())
}

fn pairing_secret(state: &ApiState) -> String {
    std::env::var("CHORUZ_REMOTE_CONTROL_PAIRING_SECRET")
        .unwrap_or_else(|_| state.auth.session_secret.clone())
}

fn gateway_issuer(state: &ApiState) -> String {
    // The hosted Worker stores this value only to bind later capabilities for
    // the same high-entropy room. Hashing gives the Worker a fixed-length
    // capability issuer without exporting a local authentication secret.
    keyed_hash(
        &pairing_secret(state),
        "remote-control-gateway-issuer",
        HOSTED_GATEWAY_URL,
    )
}

fn wrapping_key(secret: &str) -> Result<[u8; 32], ApiError> {
    let mut key = [0u8; 32];
    Hkdf::<Sha256>::new(Some(WRAPPING_CONTEXT), secret.as_bytes())
        .expand(b"session-key", &mut key)
        .map_err(|_| {
            ApiError(AppError::Internal(
                "derive remote-control storage key".into(),
            ))
        })?;
    Ok(key)
}

fn wrap_session_key(secret: &str, session_key: &str) -> Result<String, ApiError> {
    let cipher = Aes256Gcm::new_from_slice(&wrapping_key(secret)?).map_err(|_| {
        ApiError(AppError::Internal(
            "initialize remote-control cipher".into(),
        ))
    })?;
    let mut nonce = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), session_key.as_bytes())
        .map_err(|_| ApiError(AppError::Internal("wrap remote-control session key".into())))?;
    let encoder = &base64::engine::general_purpose::URL_SAFE_NO_PAD;
    Ok(format!(
        "{WRAPPED_KEY_PREFIX}.{}.{}",
        base64::Engine::encode(encoder, nonce),
        base64::Engine::encode(encoder, ciphertext)
    ))
}

fn unwrap_session_key(secret: &str, wrapped: &str) -> Result<String, ApiError> {
    let mut parts = wrapped.split('.');
    let (Some(version), Some(nonce), Some(ciphertext), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(ApiError(AppError::Internal(
            "invalid wrapped remote-control session key".into(),
        )));
    };
    if version != WRAPPED_KEY_PREFIX {
        return Err(ApiError(AppError::Internal(
            "unsupported wrapped remote-control session key".into(),
        )));
    }
    let decoder = &base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let nonce = base64::Engine::decode(decoder, nonce)
        .map_err(|_| ApiError(AppError::Internal("decode wrapped key nonce".into())))?;
    let ciphertext = base64::Engine::decode(decoder, ciphertext)
        .map_err(|_| ApiError(AppError::Internal("decode wrapped session key".into())))?;
    if nonce.len() != 12 {
        return Err(ApiError(AppError::Internal(
            "invalid wrapped key nonce length".into(),
        )));
    }
    let cipher = Aes256Gcm::new_from_slice(&wrapping_key(secret)?).map_err(|_| {
        ApiError(AppError::Internal(
            "initialize remote-control cipher".into(),
        ))
    })?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&nonce), ciphertext.as_ref())
        .map_err(|_| {
            ApiError(AppError::Internal(
                "unwrap remote-control session key".into(),
            ))
        })?;
    String::from_utf8(plaintext).map_err(|_| {
        ApiError(AppError::Internal(
            "decode remote-control session key".into(),
        ))
    })
}

pub(crate) fn keyed_hash(secret: &str, purpose: &str, value: &str) -> String {
    let mut mac =
        <HmacSha256 as Mac>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key");
    mac.update(purpose.as_bytes());
    mac.update(&[0]);
    mac.update(value.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

pub(crate) fn generate_pairing_credential() -> (String, String) {
    let mut id = [0u8; 16];
    let mut secret = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut id);
    rand::rngs::OsRng.fill_bytes(&mut secret);
    let encoder = &base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let id = base64::Engine::encode(encoder, id);
    let secret = base64::Engine::encode(encoder, secret);
    (id.clone(), format!("v1.{id}.{secret}"))
}

pub(crate) fn generate_pairing_code() -> String {
    format!("{:08}", rand::thread_rng().gen_range(0..100_000_000u32))
}

fn valid_pairing_credential(value: &str) -> bool {
    let mut parts = value.split('.');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some("v1"), Some(id), Some(secret), None)
            if id.len() == 22
                && secret.len() == 22
                && id.bytes().chain(secret.bytes()).all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')
                })
    )
}

fn gateway_url() -> Option<String> {
    std::env::var("CHORUZ_REMOTE_CONTROL_GATEWAY_URL")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_owned())
        .filter(|value| !value.is_empty())
        .or_else(|| Some(HOSTED_GATEWAY_URL.to_owned()))
}

struct PairingCapability<'a> {
    id: &'a str,
}

async fn gateway_ticket(
    state: &ApiState,
    room: &str,
    role: &str,
    scope: &str,
    expires_at: chrono::DateTime<chrono::Utc>,
    device_id: Option<&str>,
    pairing: Option<PairingCapability<'_>>,
) -> Option<String> {
    let secret = std::env::var("CHORUZ_REMOTE_CONTROL_GATEWAY_SECRET")
        .ok()
        .filter(|secret| secret.len() >= 32);
    let mut payload = serde_json::json!({
        "room": room,
        "role": role,
        "scope": scope,
        "exp": expires_at.timestamp(),
    });
    if let Some(device_id) = device_id {
        payload["device_id"] = serde_json::Value::String(device_id.to_owned());
    }
    if let Some(pairing) = &pairing {
        payload["pairing_id"] = serde_json::Value::String(pairing.id.to_owned());
    }
    match secret {
        Some(secret) => {
            let encoded = base64::Engine::encode(
                &base64::engine::general_purpose::URL_SAFE_NO_PAD,
                serde_json::to_vec(&payload).ok()?,
            );
            let signature = keyed_hash(&secret, "gateway-ticket", &encoded);
            Some(format!("{encoded}.{signature}"))
        }
        None if gateway_url().as_deref() == Some(HOSTED_GATEWAY_URL) => {
            let mut body = serde_json::json!({"issuer": gateway_issuer(state), "payload": payload});
            if let Some(pairing) = pairing {
                body["pairing_id"] = serde_json::Value::String(pairing.id.to_owned());
            }
            let response = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .ok()?
                .post(format!("{HOSTED_GATEWAY_URL}/v1/capabilities"))
                .json(&body)
                .send()
                .await
                .ok()?;
            if !response.status().is_success() {
                return None;
            }
            response
                .json::<serde_json::Value>()
                .await
                .ok()?
                .get("ticket")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        }
        None => None,
    }
}

fn session_room(session_key: &str) -> String {
    // Hex HMAC output is URL-safe and has 256 bits of entropy.  It avoids
    // exposing the E2E session key itself in the Gateway room capability.
    keyed_hash("choruz.remote-control.session-room.v1", "room", session_key)
}

async fn session_gateway_ticket(
    state: &ApiState,
    session_key: &str,
    role: &str,
    expires_at: chrono::DateTime<chrono::Utc>,
    device_id: Option<&str>,
) -> Option<String> {
    gateway_ticket(
        state,
        &session_room(session_key),
        role,
        "session",
        expires_at,
        device_id,
        None,
    )
    .await
}

async fn transport_tickets(
    state: &ApiState,
    device_ids: &[String],
) -> (Option<String>, Option<String>, HashMap<String, String>) {
    if gateway_url().is_none() {
        return (None, None, HashMap::new());
    }
    let session_id = choruz_common::new_id();
    let expires_at = chrono::Utc::now() + chrono::Duration::hours(12);
    let host = gateway_ticket(
        state,
        &session_id,
        "host",
        "transport",
        expires_at,
        None,
        None,
    )
    .await;
    if host.is_none() {
        return (None, None, HashMap::new());
    }
    let mut devices = HashMap::new();
    for device_id in device_ids {
        if let Some(ticket) = gateway_ticket(
            state,
            &session_id,
            "device",
            "transport",
            expires_at,
            Some(device_id),
            None,
        )
        .await
        {
            devices.insert(device_id.clone(), ticket);
        }
    }
    (Some(session_id), host, devices)
}

pub(crate) fn load_settings(_principal_id: &str) -> RemoteControlSettings {
    RemoteControlSettings {
        gateway_url: gateway_url(),
        // A session room is derived from the paired device key, never from a
        // predictable principal id.  It is therefore populated only once a
        // bridge has loaded that key.
        gateway_ticket: None,
    }
}

pub(crate) async fn load_bridge_material(
    state: &ApiState,
    principal_id: &str,
) -> Result<Option<BridgeMaterial>, ApiError> {
    let stored = load_stored_bridge_state(state, principal_id).await?;
    if stored.active_device_ids.is_empty() {
        return Ok(None);
    }
    let (transport_session_id, host_transport_ticket, device_transport_tickets) =
        transport_tickets(state, &stored.active_device_ids).await;
    let gateway_url = gateway_url();
    let Some((gateway_url, session_key, transport_session_id, host_transport_ticket)) = gateway_url
        .zip(stored.session_key)
        .zip(transport_session_id.zip(host_transport_ticket))
        .map(|((gateway_url, session_key), (session_id, ticket))| {
            (gateway_url, session_key, session_id, ticket)
        })
    else {
        return Ok(None);
    };
    Ok(Some(BridgeMaterial {
        gateway_url,
        host_session_ticket: session_gateway_ticket(
            state,
            &session_key,
            "host",
            chrono::Utc::now() + chrono::Duration::hours(12),
            None,
        )
        .await
        .ok_or_else(|| ApiError(AppError::Internal("create hosted session ticket".into())))?,
        session_key,
        revoked_device_ids: stored.revoked_device_ids,
        transport_session_id,
        host_transport_ticket,
        device_transport_tickets,
    }))
}

async fn load_stored_bridge_state(
    state: &ApiState,
    principal_id: &str,
) -> Result<StoredBridgeState, ApiError> {
    let client = state
        .event_store
        .connect()
        .await
        .map_err(|error| ApiError(AppError::Internal(format!("db connect: {error}"))))?;
    let secret = pairing_secret(state);
    let key_rows = client
        .query(
            "SELECT id, session_key_wrapped FROM remote_control_device
             WHERE principal_id = $1 AND revoked_at IS NULL",
            &[&principal_id],
        )
        .await
        .map_err(|error| ApiError(AppError::Internal(format!("load bridge key: {error}"))))?;
    let mut session_key: Option<String> = None;
    let mut active_device_ids = Vec::with_capacity(key_rows.len());
    for row in key_rows {
        active_device_ids.push(row.get(0));
        let wrapped: String = row.get(1);
        let candidate = unwrap_session_key(&secret, &wrapped)?;
        if session_key
            .as_ref()
            .is_some_and(|existing| existing != &candidate)
        {
            return Err(ApiError(AppError::Internal(
                "active remote-control devices have inconsistent session keys".into(),
            )));
        }
        session_key = Some(candidate);
    }
    let revoked_device_ids = client
        .query(
            "SELECT id FROM remote_control_device
             WHERE principal_id = $1 AND revoked_at IS NOT NULL",
            &[&principal_id],
        )
        .await
        .map_err(|error| ApiError(AppError::Internal(format!("load revoked devices: {error}"))))?
        .into_iter()
        .map(|row| row.get(0))
        .collect();
    Ok(StoredBridgeState {
        session_key,
        revoked_device_ids,
        active_device_ids,
    })
}

pub(crate) async fn existing_session_key(
    state: &ApiState,
    principal_id: &str,
) -> Result<Option<String>, ApiError> {
    Ok(load_stored_bridge_state(state, principal_id)
        .await?
        .session_key)
}

pub(crate) async fn get_settings(
    headers: HeaderMap,
    State(state): State<ApiState>,
) -> Result<Json<RemoteControlSettings>, ApiError> {
    let principal = require_human_operator(&headers, &state).await?;
    Ok(Json(load_settings(&principal.id)))
}

pub(crate) async fn create_pairing(
    headers: HeaderMap,
    State(state): State<ApiState>,
) -> Result<Json<PairingResponse>, ApiError> {
    let principal = require_human_operator(&headers, &state).await?;
    let secret = pairing_secret(&state);
    let (pairing_id, credential) = generate_pairing_credential();
    let credential_hash = keyed_hash(&secret, "pairing", &credential);
    let expires_at = chrono::Utc::now() + chrono::Duration::minutes(PAIRING_TTL_MINUTES);
    let settings = load_settings(&principal.id);
    let pairing_room = choruz_common::new_id();
    let gateway_ticket = gateway_ticket(
        &state,
        &pairing_room,
        "host",
        "pair",
        expires_at,
        None,
        Some(PairingCapability { id: &pairing_id }),
    )
    .await
    .ok_or_else(|| {
        ApiError(AppError::Internal(
            "create hosted pairing capability".into(),
        ))
    })?;
    let gateway_url = settings.gateway_url.ok_or_else(|| {
        ApiError(AppError::Internal(
            "Cloud Gateway is not configured for pairing".into(),
        ))
    })?;
    let client = state
        .event_store
        .connect()
        .await
        .map_err(|error| ApiError(AppError::Internal(format!("db connect: {error}"))))?;
    client
        .execute(
            "INSERT INTO remote_control_pairing (id, principal_id, credential_hash, expires_at)
         VALUES ($1, $2, $3, $4)",
            &[&pairing_id, &principal.id, &credential_hash, &expires_at],
        )
        .await
        .map_err(|error| ApiError(AppError::Internal(format!("create pairing: {error}"))))?;
    if let Err(error) = remote_control_pairing_host::connect_and_spawn(
        state.clone(),
        PairingHost {
            pairing_id: pairing_id.clone(),
            principal_id: principal.id.clone(),
            credential: credential.clone(),
            gateway_url: gateway_url.clone(),
            gateway_ticket: gateway_ticket.clone(),
            expires_at,
        },
    )
    .await
    {
        let _ = client
            .execute(
                "DELETE FROM remote_control_pairing WHERE id = $1 AND consumed_at IS NULL",
                &[&pairing_id],
            )
            .await;
        return Err(ApiError(AppError::Internal(format!(
            "connect pairing host: {error}"
        ))));
    }
    tracing::info!(
        pairing_id = %pairing_id,
        expires_at = %expires_at,
        gateway_configured = true,
        "remote-control pairing issued"
    );
    Ok(Json(PairingResponse {
        pairing_id,
        credential,
        expires_at: expires_at.to_rfc3339(),
        gateway_url: Some(gateway_url),
    }))
}

pub(crate) async fn redeem_pairing(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Json(body): Json<RedeemPairingRequest>,
) -> Result<Json<RedeemPairingResponse>, ApiError> {
    let principal = require_human_operator(&headers, &state).await?;
    Ok(Json(
        redeem_pairing_for_principal(&state, principal.id, body).await?,
    ))
}

pub(crate) async fn redeem_pairing_for_principal(
    state: &ApiState,
    principal_id: String,
    body: RedeemPairingRequest,
) -> Result<RedeemPairingResponse, ApiError> {
    validate_device(&body.device_name, &body.session_key)?;
    if !valid_pairing_credential(&body.credential) {
        tracing::warn!(
            credential_length = body.credential.len(),
            "remote-control pairing redemption rejected: malformed credential"
        );
        return Err(ApiError(AppError::Validation(
            "pairing credential is malformed".into(),
        )));
    }
    let secret = pairing_secret(state);
    let credential_hash = keyed_hash(&secret, "pairing", &body.credential);
    let mut client = state
        .event_store
        .connect()
        .await
        .map_err(|error| ApiError(AppError::Internal(format!("db connect: {error}"))))?;
    let transaction = client
        .transaction()
        .await
        .map_err(|error| ApiError(AppError::Internal(format!("begin pairing: {error}"))))?;
    let row = transaction
        .query_opt(
            "SELECT id FROM remote_control_pairing
         WHERE credential_hash = $1 AND principal_id = $2
           AND consumed_at IS NULL AND expires_at > NOW() FOR UPDATE",
            &[&credential_hash, &principal_id],
        )
        .await
        .map_err(|error| ApiError(AppError::Internal(format!("find pairing: {error}"))))?;
    let Some(row) = row else {
        tracing::warn!(
            credential_length = body.credential.len(),
            "remote-control pairing redemption rejected: invalid, expired, or consumed credential"
        );
        return Err(ApiError(AppError::Unauthorized(
            "invalid or expired pairing credential".into(),
        )));
    };
    let pairing_id: String = row.get(0);
    let active_keys = transaction
        .query(
            "SELECT session_key_wrapped FROM remote_control_device
             WHERE principal_id = $1 AND revoked_at IS NULL",
            &[&principal_id],
        )
        .await
        .map_err(|error| {
            ApiError(AppError::Internal(format!(
                "load active remote keys: {error}"
            )))
        })?;
    for row in active_keys {
        let wrapped: String = row.get(0);
        if unwrap_session_key(&secret, &wrapped)? != body.session_key {
            return Err(ApiError(AppError::Conflict(
                "remote-control session key does not match active devices".into(),
            )));
        }
    }
    let wrapped_session_key = wrap_session_key(&secret, &body.session_key)?;
    let device_id = choruz_common::new_id();
    transaction
        .execute(
            "INSERT INTO remote_control_device (id, principal_id, name, session_key_wrapped)
         VALUES ($1, $2, $3, $4)",
            &[
                &device_id,
                &principal_id,
                &body.device_name.trim(),
                &wrapped_session_key,
            ],
        )
        .await
        .map_err(|error| ApiError(AppError::Internal(format!("create remote device: {error}"))))?;
    transaction
        .execute(
            "UPDATE remote_control_pairing SET consumed_at = NOW() WHERE id = $1",
            &[&pairing_id],
        )
        .await
        .map_err(|error| ApiError(AppError::Internal(format!("consume pairing: {error}"))))?;
    transaction
        .commit()
        .await
        .map_err(|error| ApiError(AppError::Internal(format!("commit pairing: {error}"))))?;
    tracing::info!(
        pairing_id = %pairing_id,
        device_id = %device_id,
        "remote-control pairing redeemed"
    );
    state.remote_control_bridges.refresh(&principal_id);
    Ok(RedeemPairingResponse {
        gateway_ticket: session_gateway_ticket(
            state,
            &body.session_key,
            "device",
            chrono::Utc::now() + chrono::Duration::hours(12),
            Some(&device_id),
        )
        .await,
        device_id,
        gateway_url: gateway_url(),
    })
}

pub(crate) async fn list_devices(
    headers: HeaderMap,
    State(state): State<ApiState>,
) -> Result<Json<Vec<RemoteControlDevice>>, ApiError> {
    let principal = require_human_operator(&headers, &state).await?;
    let client = state
        .event_store
        .connect()
        .await
        .map_err(|error| ApiError(AppError::Internal(format!("db connect: {error}"))))?;
    let rows = client
        .query(
            "SELECT id, name, paired_at, last_seen_at FROM remote_control_device
         WHERE principal_id = $1 AND revoked_at IS NULL ORDER BY paired_at DESC",
            &[&principal.id],
        )
        .await
        .map_err(|error| ApiError(AppError::Internal(format!("list remote devices: {error}"))))?;
    Ok(Json(
        rows.into_iter()
            .map(|row| {
                let paired_at: chrono::DateTime<chrono::Utc> = row.get(2);
                let last_seen_at: Option<chrono::DateTime<chrono::Utc>> = row.get(3);
                RemoteControlDevice {
                    id: row.get(0),
                    name: row.get(1),
                    paired_at: paired_at.to_rfc3339(),
                    last_seen_at: last_seen_at.map(|value| value.to_rfc3339()),
                }
            })
            .collect(),
    ))
}

pub(crate) async fn get_bridge_config(
    headers: HeaderMap,
    State(state): State<ApiState>,
) -> Result<Json<RemoteControlBridgeConfig>, ApiError> {
    let principal = require_human_operator(&headers, &state).await?;
    let stored = load_stored_bridge_state(&state, &principal.id).await?;
    // A transport room is deliberately scoped to this bridge lifecycle rather
    // than to the principal. The host is the first client to connect, so
    // Cloudflare can place the new Durable Object near the host's current
    // network. The principal room remains only as a lightweight rendezvous for
    // already-paired browsers.
    let (transport_session_id, host_transport_ticket, device_transport_tickets) =
        transport_tickets(&state, &stored.active_device_ids).await;
    Ok(Json(RemoteControlBridgeConfig {
        settings: load_settings(&principal.id),
        session_key: stored.session_key,
        revoked_device_ids: stored.revoked_device_ids,
        transport_session_id,
        host_transport_ticket,
        device_transport_tickets,
    }))
}

pub(crate) async fn revoke_device(
    headers: HeaderMap,
    Path(device_id): Path<String>,
    State(state): State<ApiState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let principal = require_human_operator(&headers, &state).await?;
    let client = state
        .event_store
        .connect()
        .await
        .map_err(|error| ApiError(AppError::Internal(format!("db connect: {error}"))))?;
    let changed = client
        .execute(
            "UPDATE remote_control_device SET revoked_at = NOW()
         WHERE id = $1 AND principal_id = $2 AND revoked_at IS NULL",
            &[&device_id, &principal.id],
        )
        .await
        .map_err(|error| ApiError(AppError::Internal(format!("revoke remote device: {error}"))))?;
    if changed == 0 {
        return Err(ApiError(AppError::NotFound(
            "remote device not found".into(),
        )));
    }
    state.remote_control_bridges.refresh(&principal.id);
    Ok(Json(serde_json::json!({"ok": true})))
}

pub(crate) async fn mark_device_seen(
    headers: HeaderMap,
    Path(device_id): Path<String>,
    State(state): State<ApiState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    let client = state
        .event_store
        .connect()
        .await
        .map_err(|error| ApiError(AppError::Internal(format!("db connect: {error}"))))?;
    let changed = client
        .execute(
            "UPDATE remote_control_device SET last_seen_at = NOW()
         WHERE id = $1 AND principal_id = $2 AND revoked_at IS NULL",
            &[&device_id, &principal.id],
        )
        .await
        .map_err(|error| ApiError(AppError::Internal(format!("update remote device: {error}"))))?;
    if changed == 0 {
        return Err(ApiError(AppError::NotFound(
            "remote device not found".into(),
        )));
    }
    Ok(Json(serde_json::json!({"ok": true})))
}

#[cfg(test)]
mod tests {
    use super::{
        SESSION_KEY_LEN, generate_pairing_credential, keyed_hash, session_room, unwrap_session_key,
        valid_pairing_credential, validate_device, wrap_session_key,
    };

    #[test]
    fn pairing_credentials_contain_two_independent_128_bit_values() {
        for _ in 0..100 {
            let (id, credential) = generate_pairing_credential();
            assert_eq!(id.len(), 22);
            assert!(valid_pairing_credential(&credential));
            assert!(credential.starts_with(&format!("v1.{id}.")));
        }
        assert!(!valid_pairing_credential("12345678"));
        assert!(!valid_pairing_credential("v1.short.short"));
    }

    #[test]
    fn hosted_session_room_is_opaque_to_the_session_key() {
        let key = "A".repeat(SESSION_KEY_LEN);
        let room = session_room(&key);
        assert_ne!(room, key);
        assert_eq!(room.len(), 64);
        assert!(!room.contains(&key));
    }

    #[test]
    fn hashes_are_purpose_bound() {
        assert_ne!(
            keyed_hash("secret", "pairing", "value"),
            keyed_hash("secret", "gateway-ticket", "value")
        );
    }

    #[test]
    fn validates_device_material() {
        let key = "a".repeat(SESSION_KEY_LEN);
        assert!(validate_device("My browser", &key).is_ok());
        assert!(validate_device("", &key).is_err());
        assert!(validate_device("x".repeat(81).as_str(), &key).is_err());
        assert!(validate_device("Browser", "short").is_err());
    }

    #[test]
    fn session_keys_are_wrapped_and_authenticated_at_rest() {
        let session_key = "a".repeat(SESSION_KEY_LEN);
        let wrapped = wrap_session_key("storage-secret", &session_key).unwrap();
        assert!(!wrapped.contains(&session_key));
        assert_eq!(
            unwrap_session_key("storage-secret", &wrapped).unwrap(),
            session_key
        );
        assert!(unwrap_session_key("wrong-secret", &wrapped).is_err());
    }

    #[test]
    fn transport_rooms_are_not_derived_from_long_lived_identity() {
        let first = choruz_common::new_id();
        let second = choruz_common::new_id();
        assert_ne!(first, second);
        assert!(!first.is_empty());
        assert!(!second.is_empty());
    }
}
