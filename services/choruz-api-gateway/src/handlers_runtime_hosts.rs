use axum::{
    Json,
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use choruz_agent_runtime::headless::validate_model;
use choruz_common::{AppError, new_id};
use choruz_harness_login::AccountProbe;
use chrono::{Duration, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::net::SocketAddr;

use crate::{
    ApiError, ApiState, authenticated_principal,
    handlers_companies::require_company_access,
    handlers_remote_control::{generate_pairing_code, keyed_hash},
};

const PAIRING_TTL_MINUTES: i64 = 10;
const MAX_HOST_NAME_LEN: usize = 80;
const MAX_ACCOUNT_NAME_LEN: usize = 80;

#[derive(Debug, Serialize)]
pub(crate) struct RuntimeHostPairing {
    code: String,
    expires_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RuntimeHostView {
    pub(crate) id: String,
    pub(crate) company_id: String,
    name: String,
    status: String,
    last_seen_at: Option<String>,
    created_at: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RedeemPairingRequest {
    code: String,
    name: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct RedeemPairingResponse {
    host: RuntimeHostView,
    host_token: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RenameHostRequest {
    name: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AssignBindingHostRequest {
    runtime_host_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ClaimedCommand {
    command_id: String,
    attempt_id: String,
    agent_id: String,
    conversation_id: String,
    turn_id: String,
    prompt: String,
    driver_type: String,
    workspace_path: String,
    model: Option<String>,
    external_session_id: Option<String>,
    harness_account: Option<ClaimedHarnessAccount>,
    metadata: Value,
}

#[derive(Debug, Clone, Serialize)]
struct ClaimedHarnessAccount {
    id: String,
    name: String,
    profile_kind: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RegisterHarnessAccountRequest {
    id: String,
    driver_type: String,
    name: String,
    profile_kind: String,
}

/// The verified account snapshot a runtime host reports after a probe or a
/// completed sign-in.
#[derive(Debug, Deserialize)]
pub(crate) struct VerifyHarnessAccountRequest {
    account_fingerprint: String,
    subscription_type: Option<String>,
    models: Value,
    usage: Value,
}

impl VerifyHarnessAccountRequest {
    pub(crate) fn into_probe(self) -> AccountProbe {
        AccountProbe {
            fingerprint: self.account_fingerprint,
            subscription_type: self.subscription_type,
            models: self.models,
            usage: self.usage,
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct CompleteCommandRequest {
    attempt_id: String,
    succeeded: bool,
    content: Option<String>,
    #[serde(default)]
    contents: Vec<String>,
    error: Option<String>,
    #[serde(default)]
    tool_calls_count: i32,
    #[serde(default)]
    execution_duration_ms: i64,
    external_session_id: Option<String>,
    #[serde(default)]
    clear_external_session: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CommandHeartbeatRequest {
    attempt_id: String,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ClaimCommandQuery {
    #[serde(default)]
    wait_ms: u64,
}

fn secret(state: &ApiState) -> &str {
    &state.auth.session_secret
}

fn validate_host_name(name: &str) -> Result<&str, ApiError> {
    let name = name.trim();
    if name.is_empty()
        || name.chars().count() > MAX_HOST_NAME_LEN
        || name.chars().any(char::is_control)
    {
        return Err(ApiError(AppError::Validation(
            "name must contain 1 to 80 printable characters".into(),
        )));
    }
    Ok(name)
}

fn validate_account_name(name: &str) -> Result<&str, ApiError> {
    let name = name.trim();
    if name.is_empty()
        || name.chars().count() > MAX_ACCOUNT_NAME_LEN
        || name.chars().any(char::is_control)
    {
        return Err(ApiError(AppError::Validation(
            "account name must contain 1 to 80 printable characters".into(),
        )));
    }
    Ok(name)
}

pub(crate) fn validate_authenticated_account(probe: &AccountProbe) -> Result<(), ApiError> {
    if probe.fingerprint.len() != 64
        || !probe
            .fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(ApiError(AppError::Validation(
            "account_fingerprint must be a SHA-256 hex digest".into(),
        )));
    }
    if probe
        .subscription_type
        .as_ref()
        .is_some_and(|value| value.chars().count() > 80 || value.chars().any(char::is_control))
    {
        return Err(ApiError(AppError::Validation(
            "subscription_type is invalid".into(),
        )));
    }
    let models = probe
        .models
        .as_array()
        .ok_or_else(|| ApiError(AppError::Validation("models must be an array".into())))?;
    if models.len() > 200 {
        return Err(ApiError(AppError::Validation(
            "models must contain at most 200 entries".into(),
        )));
    }
    for model in models {
        let id = model
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError(AppError::Validation("every model must have an id".into())))?;
        validate_model(id).map_err(|reason| ApiError(AppError::Validation(reason.to_owned())))?;
    }
    let windows = probe
        .usage
        .get("windows")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ApiError(AppError::Validation(
                "usage must contain exact quota windows".into(),
            ))
        })?;
    if windows.len() > 100 {
        return Err(ApiError(AppError::Validation(
            "usage must contain at most 100 exact quota windows".into(),
        )));
    }
    for window in windows {
        let used = window.get("usedPercent").and_then(Value::as_f64);
        let remaining = window.get("remainingPercent").and_then(Value::as_f64);
        if !matches!((used, remaining), (Some(used), Some(remaining)) if (0.0..=100.0).contains(&used) && (0.0..=100.0).contains(&remaining) && (used + remaining - 100.0).abs() < 0.001)
        {
            return Err(ApiError(AppError::Validation(
                "quota percentages must be exact and sum to 100".into(),
            )));
        }
    }
    Ok(())
}

pub(crate) fn validate_probe(probe: &AccountProbe) -> Result<(), ApiError> {
    validate_authenticated_account(probe)?;
    if probe.models.as_array().is_none_or(Vec::is_empty) {
        return Err(ApiError(AppError::Validation(
            "models must contain between 1 and 200 entries".into(),
        )));
    }
    if probe
        .usage
        .get("windows")
        .and_then(Value::as_array)
        .is_none_or(Vec::is_empty)
    {
        return Err(ApiError(AppError::Validation(
            "usage must contain between 1 and 100 exact quota windows".into(),
        )));
    }
    Ok(())
}

/// Mark an account authenticated without changing the independently fetched
/// model and quota snapshot or its `probed_at` timestamp.
pub(crate) async fn store_authenticated_account(
    client: &impl deadpool_postgres::GenericClient,
    account_id: &str,
    company_id: &str,
    runtime_host_id: Option<&str>,
    probe: &AccountProbe,
) -> Result<u64, ApiError> {
    client
        .execute(
            "UPDATE harness_account
                SET account_fingerprint = $4,
                    subscription_type = COALESCE($5, subscription_type),
                    status = 'active', last_error = NULL, updated_at = NOW()
              WHERE id = $1 AND company_id = $2 AND runtime_host_id IS NOT DISTINCT FROM $3
                AND disabled_at IS NULL",
            &[
                &account_id,
                &company_id,
                &runtime_host_id,
                &probe.fingerprint,
                &probe.subscription_type,
            ],
        )
        .await
        .map_err(internal("store authenticated harness account"))
}

/// Store a verified account snapshot on the account that `runtime_host_id`
/// (or this device, when `None`) owns. Returns how many rows changed.
pub(crate) async fn store_account_probe(
    client: &impl deadpool_postgres::GenericClient,
    account_id: &str,
    company_id: &str,
    runtime_host_id: Option<&str>,
    probe: &AccountProbe,
) -> Result<u64, ApiError> {
    client
        .execute(
            "UPDATE harness_account
                SET account_fingerprint = $4, subscription_type = $5,
                    models_json = $6::jsonb, usage_json = $7::jsonb,
                    status = 'active', last_error = NULL,
                    probed_at = NOW(), updated_at = NOW()
              WHERE id = $1 AND company_id = $2 AND runtime_host_id IS NOT DISTINCT FROM $3
                AND disabled_at IS NULL",
            &[
                &account_id,
                &company_id,
                &runtime_host_id,
                &probe.fingerprint,
                &probe.subscription_type,
                &probe.models,
                &probe.usage,
            ],
        )
        .await
        .map_err(internal("store harness account probe"))
}

pub(crate) async fn register_harness_account(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(host_id): Path<String>,
    Json(payload): Json<RegisterHarnessAccountRequest>,
) -> Result<StatusCode, ApiError> {
    let host = require_host(&headers, &state, &host_id).await?;
    uuid::Uuid::parse_str(&payload.id)
        .map_err(|_| ApiError(AppError::Validation("id must be a UUID".into())))?;
    let name = validate_account_name(&payload.name)?;
    if !matches!(
        payload.driver_type.as_str(),
        "claude_terminal" | "codex_terminal"
    ) {
        return Err(ApiError(AppError::Validation(
            "driver_type must be claude_terminal or codex_terminal".into(),
        )));
    }
    if !matches!(payload.profile_kind.as_str(), "default" | "isolated") {
        return Err(ApiError(AppError::Validation(
            "profile_kind must be default or isolated".into(),
        )));
    }
    let client = state.event_store.connect().await.map_err(ApiError::from)?;
    let inserted = client
        .execute(
            "INSERT INTO harness_account
               (id, company_id, runtime_host_id, driver_type, name, profile_kind)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (id) DO NOTHING",
            &[
                &payload.id,
                &host.company_id,
                &host.id,
                &payload.driver_type,
                &name,
                &payload.profile_kind,
            ],
        )
        .await
        .map_err(internal("register runtime host harness account"))?;
    if inserted == 0 {
        let matches_host = client
            .query_opt(
                "SELECT 1 FROM harness_account WHERE id = $1 AND company_id = $2 AND runtime_host_id = $3",
                &[&payload.id, &host.company_id, &host.id],
            )
            .await
            .map_err(internal("verify registered runtime host harness account"))?
            .is_some();
        if !matches_host {
            return Err(ApiError(AppError::Conflict(
                "harness account id belongs to another runtime host".into(),
            )));
        }
    }
    Ok(StatusCode::CREATED)
}

pub(crate) async fn verify_harness_account(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path((host_id, account_id)): Path<(String, String)>,
    Json(payload): Json<VerifyHarnessAccountRequest>,
) -> Result<StatusCode, ApiError> {
    let host = require_host(&headers, &state, &host_id).await?;
    let probe = payload.into_probe();
    validate_probe(&probe)?;
    let client = state.event_store.connect().await.map_err(ApiError::from)?;
    let updated = store_account_probe(
        &client,
        &account_id,
        &host.company_id,
        Some(&host.id),
        &probe,
    )
    .await?;
    if updated == 0 {
        return Err(ApiError(AppError::NotFound(
            "harness account not found for this runtime host".into(),
        )));
    }
    Ok(StatusCode::NO_CONTENT)
}

fn host_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("x-choruz-host-token")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(crate) async fn require_host(
    headers: &HeaderMap,
    state: &ApiState,
    host_id: &str,
) -> Result<RuntimeHostView, ApiError> {
    let token = host_token(headers).ok_or_else(|| {
        ApiError(AppError::Unauthorized(
            "missing x-choruz-host-token header".into(),
        ))
    })?;
    let token_hash = keyed_hash(secret(state), "runtime-host-token", token);
    let client = state.event_store.connect().await.map_err(ApiError::from)?;
    let row = client
        .query_opt(
            "SELECT id, company_id, name, status, last_seen_at, created_at
             FROM runtime_host
             WHERE id = $1 AND token_hash = $2 AND revoked_at IS NULL",
            &[&host_id, &token_hash],
        )
        .await
        .map_err(internal("authenticate runtime host"))?
        .ok_or_else(|| ApiError(AppError::Unauthorized("invalid runtime host token".into())))?;
    Ok(host_from_row(&row))
}

pub(crate) fn internal(context: &'static str) -> impl FnOnce(tokio_postgres::Error) -> ApiError {
    move |error| ApiError(AppError::Internal(format!("{context}: {error}")))
}

fn host_from_row(row: &tokio_postgres::Row) -> RuntimeHostView {
    RuntimeHostView {
        id: row.get("id"),
        company_id: row.get("company_id"),
        name: row.get("name"),
        status: row.get("status"),
        last_seen_at: row
            .get::<_, Option<chrono::DateTime<Utc>>>("last_seen_at")
            .map(|value| value.to_rfc3339()),
        created_at: row
            .get::<_, chrono::DateTime<Utc>>("created_at")
            .to_rfc3339(),
    }
}

pub(crate) async fn create_pairing(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(company_id): Path<String>,
) -> Result<(StatusCode, Json<RuntimeHostPairing>), ApiError> {
    let actor = require_company_access(&headers, &state, &company_id).await?;
    let expires_at = Utc::now() + Duration::minutes(PAIRING_TTL_MINUTES);
    let client = state.event_store.connect().await.map_err(ApiError::from)?;
    client
        .execute(
            "DELETE FROM runtime_host_pairing WHERE expires_at <= NOW() OR consumed_at IS NOT NULL",
            &[],
        )
        .await
        .map_err(internal("prune runtime host pairings"))?;
    for _ in 0..8 {
        let code = generate_pairing_code();
        let code_hash = keyed_hash(secret(&state), "runtime-host-pairing", &code);
        let inserted = client
            .execute(
                "INSERT INTO runtime_host_pairing
                   (id, company_id, code_hash, created_by, expires_at)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (code_hash) DO NOTHING",
                &[&new_id(), &company_id, &code_hash, &actor.id, &expires_at],
            )
            .await
            .map_err(internal("create runtime host pairing"))?;
        if inserted == 1 {
            return Ok((
                StatusCode::CREATED,
                Json(RuntimeHostPairing {
                    code,
                    expires_at: expires_at.to_rfc3339(),
                }),
            ));
        }
    }
    Err(ApiError(AppError::Internal(
        "could not allocate a runtime host pairing code".into(),
    )))
}

pub(crate) async fn redeem_pairing(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<ApiState>,
    Json(payload): Json<RedeemPairingRequest>,
) -> Result<(StatusCode, Json<RedeemPairingResponse>), ApiError> {
    let requester = peer.ip().to_string();
    state
        .db
        .check_rate_limit(&format!("runtime-host-pairing:{requester}"))?;
    if payload.code.len() != 8 || !payload.code.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ApiError(AppError::Validation(
            "code must contain exactly 8 digits".into(),
        )));
    }
    let name = validate_host_name(&payload.name)?.to_owned();
    let code_hash = keyed_hash(secret(&state), "runtime-host-pairing", &payload.code);
    let mut token_bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut token_bytes);
    let host_token = base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        token_bytes,
    );
    let token_hash = keyed_hash(secret(&state), "runtime-host-token", &host_token);
    let host_id = new_id();
    let now = Utc::now();
    let mut client = state.event_store.connect().await.map_err(ApiError::from)?;
    let tx = client
        .transaction()
        .await
        .map_err(internal("begin runtime host redemption"))?;
    let pairing = tx
        .query_opt(
            "SELECT company_id FROM runtime_host_pairing
             WHERE code_hash = $1 AND consumed_at IS NULL AND expires_at > $2
             FOR UPDATE",
            &[&code_hash, &now],
        )
        .await
        .map_err(internal("redeem runtime host pairing"))?
        .ok_or_else(|| {
            ApiError(AppError::Unauthorized(
                "pairing code is invalid or expired".into(),
            ))
        })?;
    let company_id: String = pairing.get("company_id");
    let row = tx
        .query_one(
            "INSERT INTO runtime_host
               (id, company_id, name, token_hash, status, last_seen_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, 'online', $5, $5, $5)
             RETURNING id, company_id, name, status, last_seen_at, created_at",
            &[&host_id, &company_id, &name, &token_hash, &now],
        )
        .await
        .map_err(|error| {
            if error
                .as_db_error()
                .is_some_and(|db| db.code() == &tokio_postgres::error::SqlState::UNIQUE_VIOLATION)
            {
                ApiError(AppError::Conflict(
                    "an active runtime host already uses this name".into(),
                ))
            } else {
                internal("create runtime host")(error)
            }
        })?;
    tx.execute(
        "UPDATE runtime_host_pairing SET consumed_at = $2 WHERE code_hash = $1",
        &[&code_hash, &now],
    )
    .await
    .map_err(internal("consume runtime host pairing"))?;
    tx.commit()
        .await
        .map_err(internal("commit runtime host redemption"))?;
    Ok((
        StatusCode::CREATED,
        Json(RedeemPairingResponse {
            host: host_from_row(&row),
            host_token,
        }),
    ))
}

pub(crate) async fn list_hosts(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(company_id): Path<String>,
) -> Result<Json<Vec<RuntimeHostView>>, ApiError> {
    require_company_access(&headers, &state, &company_id).await?;
    let client = state.event_store.connect().await.map_err(ApiError::from)?;
    let rows = client
        .query(
            "SELECT id, company_id, name,
                    CASE WHEN status = 'online' AND last_seen_at < NOW() - INTERVAL '45 seconds'
                         THEN 'offline' ELSE status END AS status,
                    last_seen_at, created_at
             FROM runtime_host
             WHERE company_id = $1 AND revoked_at IS NULL
             ORDER BY lower(name), id",
            &[&company_id],
        )
        .await
        .map_err(internal("list runtime hosts"))?;
    Ok(Json(rows.iter().map(host_from_row).collect()))
}

pub(crate) async fn rename_host(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(host_id): Path<String>,
    Json(payload): Json<RenameHostRequest>,
) -> Result<Json<RuntimeHostView>, ApiError> {
    let name = validate_host_name(&payload.name)?.to_owned();
    let client = state.event_store.connect().await.map_err(ApiError::from)?;
    let company_id = client
        .query_opt(
            "SELECT company_id FROM runtime_host WHERE id = $1 AND revoked_at IS NULL",
            &[&host_id],
        )
        .await
        .map_err(internal("find runtime host"))?
        .map(|row| row.get::<_, String>("company_id"))
        .ok_or_else(|| ApiError(AppError::NotFound(format!("runtime host {host_id}"))))?;
    require_company_access(&headers, &state, &company_id).await?;
    let row = client
        .query_one(
            "UPDATE runtime_host SET name = $2, updated_at = NOW()
             WHERE id = $1
             RETURNING id, company_id, name, status, last_seen_at, created_at",
            &[&host_id, &name],
        )
        .await
        .map_err(|error| {
            if error
                .as_db_error()
                .is_some_and(|db| db.code() == &tokio_postgres::error::SqlState::UNIQUE_VIOLATION)
            {
                ApiError(AppError::Conflict(
                    "an active runtime host already uses this name".into(),
                ))
            } else {
                internal("rename runtime host")(error)
            }
        })?;
    Ok(Json(host_from_row(&row)))
}

pub(crate) async fn revoke_host(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(host_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let mut client = state.event_store.connect().await.map_err(ApiError::from)?;
    let row = client
        .query_opt(
            "SELECT company_id FROM runtime_host WHERE id = $1 AND revoked_at IS NULL",
            &[&host_id],
        )
        .await
        .map_err(internal("find runtime host"))?
        .ok_or_else(|| ApiError(AppError::NotFound(format!("runtime host {host_id}"))))?;
    require_company_access(&headers, &state, row.get("company_id")).await?;
    let tx = client
        .transaction()
        .await
        .map_err(internal("begin runtime host revocation"))?;
    tx.execute(
        "UPDATE runtime_host SET status = 'revoked', revoked_at = NOW(), updated_at = NOW()
             WHERE id = $1",
        &[&host_id],
    )
    .await
    .map_err(internal("revoke runtime host"))?;
    tx.execute(
        "UPDATE agent_runtime_bindings
         SET config_json = config_json - 'runtime_host_id',
             external_session_id = NULL, external_thread_id = NULL,
             state = 'idle', updated_at = NOW()
         WHERE config_json->>'runtime_host_id' = $1",
        &[&host_id],
    )
    .await
    .map_err(internal("return host bindings to local execution"))?;
    tx.execute(
        "UPDATE session_registry
         SET epoch = epoch + 1, status = 'idle', executor_node_id = NULL,
             last_heartbeat_at = NULL, updated_at = NOW()
         WHERE session_key IN (
             SELECT session_key FROM agent_commands
             WHERE metadata->>'runtime_host_id' = $1
               AND status IN ('pending', 'retry_scheduled', 'leased', 'started', 'heartbeating')
         )",
        &[&host_id],
    )
    .await
    .map_err(internal("fence revoked host attempts"))?;
    tx.execute(
        "UPDATE agent_commands
         SET metadata = metadata - 'runtime_host_id',
             status = CASE WHEN status IN ('leased', 'started', 'heartbeating')
                           THEN 'pending' ELSE status END,
             current_attempt_id = NULL, current_epoch = NULL,
             next_retry_at = NULL, updated_at = NOW()
         WHERE metadata->>'runtime_host_id' = $1
           AND status IN ('pending', 'retry_scheduled', 'leased', 'started', 'heartbeating')",
        &[&host_id],
    )
    .await
    .map_err(internal("requeue revoked host commands"))?;
    tx.commit()
        .await
        .map_err(internal("commit runtime host revocation"))?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn heartbeat(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(host_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_host(&headers, &state, &host_id).await?;
    let client = state.event_store.connect().await.map_err(ApiError::from)?;
    client
        .execute(
            "UPDATE runtime_host SET status = 'online', last_seen_at = NOW(), updated_at = NOW()
             WHERE id = $1 AND revoked_at IS NULL",
            &[&host_id],
        )
        .await
        .map_err(internal("heartbeat runtime host"))?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn assign_binding_host(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(binding_id): Path<String>,
    Json(payload): Json<AssignBindingHostRequest>,
) -> Result<StatusCode, ApiError> {
    let actor = authenticated_principal(&headers, &state).await?;
    let binding = state.runtime.get_binding(&binding_id).await?;
    let agent = state.db.get_principal(&binding.agent_principal_id).await?;
    let companies = state.db.list_companies(&actor.id).await?;
    if !companies
        .iter()
        .any(|company| company.id == agent.workspace_id)
    {
        return Err(ApiError(AppError::Forbidden(
            "cannot move an agent runtime from another company".into(),
        )));
    }
    let target_host_id = payload
        .runtime_host_id
        .as_deref()
        .map(str::trim)
        .map(str::to_owned);
    if target_host_id.as_deref() == Some("") {
        return Err(ApiError(AppError::Validation(
            "runtime_host_id must not be empty".into(),
        )));
    }
    let mut client = state.event_store.connect().await.map_err(ApiError::from)?;
    if let Some(host_id) = target_host_id.as_deref() {
        let valid = client
            .query_opt(
                "SELECT 1 FROM runtime_host
                 WHERE id = $1 AND company_id = $2 AND revoked_at IS NULL",
                &[&host_id, &agent.workspace_id],
            )
            .await
            .map_err(internal("validate runtime host assignment"))?
            .is_some();
        if !valid {
            return Err(ApiError(AppError::Validation(
                "runtime host does not belong to the agent's company".into(),
            )));
        }
    }
    let tx = client
        .transaction()
        .await
        .map_err(internal("begin runtime host assignment"))?;
    tx.execute(
        "UPDATE session_registry sr
         SET epoch = epoch + 1, status = 'idle', executor_node_id = NULL,
             last_heartbeat_at = NULL, updated_at = NOW()
         WHERE EXISTS (
             SELECT 1 FROM agent_commands ac
             WHERE ac.session_key = sr.session_key AND ac.agent_id = $1
               AND ac.status IN ('pending', 'retry_scheduled', 'leased', 'started', 'heartbeating')
         )",
        &[&agent.id],
    )
    .await
    .map_err(internal("fence commands during runtime host assignment"))?;
    tx.execute(
        "UPDATE agent_commands
         SET metadata = CASE
               WHEN $2::text IS NULL THEN metadata - 'runtime_host_id'
               ELSE jsonb_set(metadata, '{runtime_host_id}', to_jsonb($2::text), true)
             END,
             status = 'pending', current_attempt_id = NULL, current_epoch = NULL,
             next_retry_at = NULL, updated_at = NOW()
         WHERE agent_id = $1
           AND status IN ('pending', 'retry_scheduled', 'leased', 'started', 'heartbeating')",
        &[&agent.id, &target_host_id],
    )
    .await
    .map_err(internal("requeue commands during runtime host assignment"))?;
    tx.execute(
        "UPDATE agent_runtime_bindings
             SET config_json = CASE
                   WHEN $2::text IS NULL THEN config_json - 'runtime_host_id'
                   ELSE jsonb_set(config_json, '{runtime_host_id}', to_jsonb($2::text), true)
                 END,
                 external_session_id = NULL,
                 external_thread_id = NULL,
                 state = 'idle',
                 updated_at = NOW()
             WHERE id = $1",
        &[&binding_id, &target_host_id],
    )
    .await
    .map_err(internal("assign runtime host"))?;
    tx.commit()
        .await
        .map_err(internal("commit runtime host assignment"))?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn claim_command(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(host_id): Path<String>,
    Query(query): Query<ClaimCommandQuery>,
) -> Result<Json<Option<ClaimedCommand>>, ApiError> {
    let host = require_host(&headers, &state, &host_id).await?;
    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_millis(query.wait_ms.min(25_000));
    let claimed = loop {
        let claimed = state
            .session
            .claim_runtime_host_command(&host_id)
            .await
            .map_err(session_error)?;
        if claimed.is_some() || tokio::time::Instant::now() >= deadline {
            break claimed;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    };
    let Some((command, assignment)) = claimed else {
        return Ok(Json(None));
    };
    let binding = state
        .runtime
        .list_bindings_by_agent(&command.agent_id)
        .await?
        .into_iter()
        .find(|binding| {
            binding.state != choruz_agent_runtime::BindingState::Disabled
                && binding
                    .config_json
                    .get("runtime_host_id")
                    .and_then(Value::as_str)
                    == Some(host_id.as_str())
        })
        .ok_or_else(|| {
            ApiError(AppError::Conflict(
                "remote command has no active runtime binding".into(),
            ))
        })?;
    let model = binding
        .config_json
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let external_session_id = binding.external_session_id.clone();
    let harness_account = match (
        binding
            .config_json
            .get("harness_account_id")
            .and_then(Value::as_str),
        binding
            .config_json
            .get("harness_account_name")
            .and_then(Value::as_str),
        binding
            .config_json
            .get("harness_account_profile_kind")
            .and_then(Value::as_str),
    ) {
        (Some(id), Some(name), Some(profile_kind)) => Some(ClaimedHarnessAccount {
            id: id.to_owned(),
            name: name.to_owned(),
            profile_kind: profile_kind.to_owned(),
        }),
        _ => None,
    };
    if let Some(account) = harness_account.as_ref() {
        let account_is_active = state
            .event_store
            .connect()
            .await
            .map_err(ApiError::from)?
            .query_opt(
                "SELECT 1 FROM harness_account
                  WHERE id = $1 AND company_id = $2 AND runtime_host_id = $3
                    AND driver_type = $4 AND status = 'active' AND disabled_at IS NULL
                    AND ($5::text IS NULL OR models_json @> jsonb_build_array(jsonb_build_object('id', $5)))",
                &[&account.id, &host.company_id, &host_id, &binding.driver_type.as_str(), &model],
            )
            .await
            .map_err(internal("revalidate claimed harness account"))?
            .is_some();
        if !account_is_active {
            return Err(ApiError(AppError::Conflict(
                "remote command's Harness account is no longer active".into(),
            )));
        }
    }
    Ok(Json(Some(ClaimedCommand {
        command_id: command.command_id,
        attempt_id: assignment.attempt_id,
        agent_id: command.agent_id,
        conversation_id: command.conversation_id,
        turn_id: command.turn_id,
        prompt: command.prompt,
        driver_type: binding.driver_type.as_str().to_owned(),
        workspace_path: binding.workspace_path,
        model,
        external_session_id,
        harness_account,
        metadata: command.metadata,
    })))
}

pub(crate) async fn complete_command(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path((host_id, command_id)): Path<(String, String)>,
    Json(payload): Json<CompleteCommandRequest>,
) -> Result<StatusCode, ApiError> {
    let host = require_host(&headers, &state, &host_id).await?;
    let external_session_id = payload
        .external_session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if external_session_id.is_some_and(|session_id| {
        session_id.len() > 512 || session_id.chars().any(char::is_control)
    }) {
        return Err(ApiError(AppError::Validation(
            "external_session_id contains invalid characters".into(),
        )));
    }
    let mut contents = payload.contents;
    if contents.is_empty()
        && let Some(content) = payload.content
    {
        contents.push(content);
    }
    if contents.len() > 100
        || contents.iter().any(|content| content.len() > 100_000)
        || contents.iter().map(String::len).sum::<usize>() > 1_000_000
    {
        return Err(ApiError(AppError::Validation(
            "remote replies exceed the supported limit".into(),
        )));
    }
    state
        .session
        .complete_runtime_host_command(
            &host_id,
            &host.name,
            &command_id,
            &payload.attempt_id,
            payload.succeeded,
            &contents,
            payload.error.as_deref(),
            payload.tool_calls_count,
            payload.execution_duration_ms,
            external_session_id,
            payload.clear_external_session,
        )
        .await
        .map_err(session_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn heartbeat_command(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path((host_id, command_id)): Path<(String, String)>,
    Json(payload): Json<CommandHeartbeatRequest>,
) -> Result<StatusCode, ApiError> {
    require_host(&headers, &state, &host_id).await?;
    state
        .session
        .heartbeat_runtime_host_command(&host_id, &command_id, &payload.attempt_id)
        .await
        .map_err(session_error)?;
    Ok(StatusCode::NO_CONTENT)
}

fn session_error(error: choruz_session::SessionError) -> ApiError {
    match error {
        choruz_session::SessionError::StaleAttempt { .. }
        | choruz_session::SessionError::InvalidStateTransition { .. } => {
            ApiError(AppError::Conflict(error.to_string()))
        }
        choruz_session::SessionError::CommandNotFound(_)
        | choruz_session::SessionError::SessionNotFound(_) => {
            ApiError(AppError::NotFound(error.to_string()))
        }
        _ => ApiError(AppError::Internal(error.to_string())),
    }
}
