//! Account-scoped sign-in: private official Claude terminals and Codex browser login.
//!
//! Claude terminal bytes are transient transport, not application authentication
//! fields or activity records. Done verifies identity independently of catalogs.
//! Codex retains its local/connector LoginSink browser flow.

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use choruz_agent_runtime::headless::HeadlessDriver;
use choruz_common::{AppError, new_id};
use choruz_harness_login::{AccountProbe, DEFAULT_LOGIN_TIMEOUT, LoginJob, LoginSink, run_login};
use choruz_host_runtime::{HarnessProbeResult, HostRequest};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    ApiError, ApiState,
    handlers_companies::require_company_access,
    handlers_runtime_hosts::{
        VerifyHarnessAccountRequest, internal, require_host, store_account_probe,
        store_authenticated_account, validate_authenticated_account, validate_probe,
    },
    host_runtime::RuntimeHost,
};

const HARNESS_LOGIN_TTL_MINUTES: i64 = 15;

#[derive(Deserialize)]
pub(crate) struct DriverModelsQuery {
    driver_type: String,
}

pub(crate) async fn local_driver_models(
    headers: HeaderMap,
    State(state): State<ApiState>,
    axum::extract::Query(query): axum::extract::Query<DriverModelsQuery>,
) -> Result<Json<Value>, ApiError> {
    crate::auth::require_human_operator(&headers, &state).await?;
    Ok(Json(
        RuntimeHost::local(&state)
            .call(HostRequest::DriverCatalog {
                driver_type: Some(query.driver_type),
            })
            .await?,
    ))
}
const MAX_LOGIN_ERROR_LEN: usize = 500;
const LOGIN_VIEW_COLUMNS: &str = "id, account_id, runtime_host_id, driver_type, state,
                                  authorization_url, user_code, error, expires_at";

#[derive(Deserialize)]
pub(crate) struct LoginTerminalQuery {
    token: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
}

async fn owned_claude_login(
    state: &ApiState,
    headers: &HeaderMap,
    company_id: &str,
    account_id: &str,
    login_id: &str,
) -> Result<(RuntimeHost, LoginScope, String), ApiError> {
    let actor = crate::auth::require_human_operator(headers, state).await?;
    require_company_access(headers, state, company_id).await?;
    let client = state.event_store.connect().await.map_err(ApiError::from)?;
    let row = client.query_opt(
        "SELECT login.runtime_host_id, account.profile_kind
         FROM harness_account_login login JOIN harness_account account ON account.id = login.account_id
         WHERE login.id = $1 AND login.company_id = $2 AND login.account_id = $3
           AND login.created_by = $4 AND login.driver_type = 'claude_terminal'
           AND login.state = 'authorizing' AND login.expires_at > NOW()
           AND account.disabled_at IS NULL",
        &[&login_id, &company_id, &account_id, &actor.id],
    ).await.map_err(internal("authorize Claude sign-in terminal"))?
     .ok_or_else(|| ApiError(AppError::NotFound("active Claude sign-in not found".into())))?;
    let runtime_host_id: Option<String> = row.get("runtime_host_id");
    let host = match runtime_host_id.as_deref() {
        Some(id) => RuntimeHost::for_host(state, id)?,
        None => RuntimeHost::local(state),
    };
    Ok((
        host,
        LoginScope {
            login_id: login_id.into(),
            company_id: company_id.into(),
            runtime_host_id,
        },
        row.get("profile_kind"),
    ))
}

/// Authentication bytes stay in the live PTY transport, never in activity or trace storage.
pub(crate) async fn websocket_claude_login(
    ws: axum::extract::ws::WebSocketUpgrade,
    mut headers: HeaderMap,
    State(state): State<ApiState>,
    Path((company_id, account_id, login_id)): Path<(String, String, String)>,
    axum::extract::Query(query): axum::extract::Query<LoginTerminalQuery>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    if crate::bearer_token_value(&headers).is_none()
        && let Some(token) = query.token
    {
        headers.insert(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {token}")
                .parse()
                .map_err(|_| ApiError(AppError::Unauthorized("invalid token".into())))?,
        );
    }
    let (host, _, profile_kind) =
        owned_claude_login(&state, &headers, &company_id, &account_id, &login_id).await?;
    let spec = choruz_host_runtime::TerminalSpec {
        authentication: true,
        terminal_id: format!("login-{login_id}"),
        driver_type: "claude_terminal".into(),
        binary_path: None,
        workspace_path: String::new(),
        cols: query.cols.unwrap_or(100),
        rows: query.rows.unwrap_or(30),
        resume_session_id: None,
        codex_home: None,
        model: None,
        harness_account: serde_json::json!({"harness_account_id": account_id, "harness_account_profile_kind": profile_kind}),
    };
    Ok(ws.max_message_size(64 * 1024).on_upgrade(move |mut socket| async move {
        use axum::extract::ws::Message;
        let id = spec.terminal_id.clone();
        if host.ensure_terminal(spec).await.is_err() {
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
        let run = async {
            let mut attachment = host.attach_terminal(&id).await?;
            for data in attachment.replay {
                socket.send(Message::Binary(data.into())).await.map_err(|_| AppError::Conflict("login terminal disconnected".into()))?;
            }
            let mut check = tokio::time::interval(std::time::Duration::from_secs(1));
            loop {
                tokio::select! {
                    _ = check.tick() => {
                        if owned_claude_login(&state, &headers, &company_id, &account_id, &login_id).await.is_err() { break; }
                    }
                    frame = attachment.output.recv() => match frame {
                        Some(data) => if socket.send(Message::Binary(data.into())).await.is_err() { break; },
                        None => break,
                    },
                    frame = socket.recv() => match frame {
                        Some(Ok(Message::Binary(data))) => host.write_terminal(&id, &data).await?,
                        Some(Ok(Message::Text(data))) => {
                            if let Ok(value) = serde_json::from_str::<Value>(&data)
                                && value["type"] == "resize"
                                && let (Some(cols), Some(rows)) = (value["cols"].as_u64(), value["rows"].as_u64()) {
                                    if let (Ok(cols), Ok(rows)) = (u16::try_from(cols), u16::try_from(rows)) {
                                        host.resize_terminal(&id, cols, rows).await?;
                                    }
                            } else { host.write_terminal(&id, data.as_bytes()).await?; }
                        }
                        Some(Ok(Message::Ping(data))) => { let _ = socket.send(Message::Pong(data)).await; }
                        Some(Ok(Message::Pong(_))) => {},
                        _ => break,
                    }
                }
            }
            Ok::<_, AppError>(())
        };
        // Do not log CLI output or errors that could contain authentication material.
        let _ = tokio::time::timeout(DEFAULT_LOGIN_TIMEOUT, run).await;
        let _ = host.close_terminal(&id).await;
        let _ = socket.send(Message::Close(None)).await;
    }))
}

pub(crate) async fn complete_claude_login(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path((company_id, account_id, login_id)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    let (host, scope, profile_kind) =
        owned_claude_login(&state, &headers, &company_id, &account_id, &login_id).await?;
    let result: HarnessProbeResult = host.call(HostRequest::HarnessProbe {
        driver_type: "claude_terminal".into(), account_id, profile_kind, identity_only: true,
    }).await.map_err(|_| ApiError(AppError::Validation("Claude Code is not signed in; finish signing in inside the terminal and try Done again".into())))?;
    let probe = AccountProbe {
        fingerprint: result.account_fingerprint,
        subscription_type: result.subscription_type,
        models: result.models,
        usage: result.usage,
    };
    let mut client = state.event_store.connect().await.map_err(ApiError::from)?;
    complete_login(&mut client, &scope, &probe).await?;
    // Identity is already committed. The socket also stops the terminal when
    // it observes completion; delayed process exit must not report login failure.
    let _ = host.close_terminal(&format!("login-{login_id}")).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize)]
pub(crate) struct HarnessAccountLoginView {
    id: String,
    account_id: String,
    /// `None` when the API gateway runs the sign-in on its own device.
    runtime_host_id: Option<String>,
    driver_type: String,
    state: String,
    authorization_url: Option<String>,
    user_code: Option<String>,
    error: Option<String>,
    expires_at: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ClaimedHarnessAccountLogin {
    login_id: String,
    account_id: String,
    driver_type: String,
    profile_kind: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ClaimedHarnessAccountLoginCallback {
    code: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SubmitHarnessAccountLoginCallbackRequest {
    code: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PublishHarnessAccountLoginRequest {
    authorization_url: String,
    user_code: Option<String>,
}

/// One login row as its executor addresses it: the connector for the host in
/// `runtime_host_id`, the gateway itself when that is `None`.
#[derive(Clone, Debug)]
struct LoginScope {
    login_id: String,
    company_id: String,
    runtime_host_id: Option<String>,
}

fn validate_login_url(value: &str) -> Result<&str, ApiError> {
    let value = value.trim();
    let url = reqwest::Url::parse(value).map_err(|_| {
        ApiError(AppError::Validation(
            "authorization_url must be a valid HTTPS URL".into(),
        ))
    })?;
    if url.scheme() != "https" || value.len() > 8_000 {
        return Err(ApiError(AppError::Validation(
            "authorization_url must be a valid HTTPS URL".into(),
        )));
    }
    Ok(value)
}

fn validate_callback_code(value: &str) -> Result<&str, ApiError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 8_000 || value.chars().any(char::is_control) {
        return Err(ApiError(AppError::Validation(
            "authorization code is invalid".into(),
        )));
    }
    Ok(value)
}

fn sanitize_login_error(error: &str) -> &str {
    let error = error.trim();
    if error.is_empty() || error.len() > MAX_LOGIN_ERROR_LEN || error.chars().any(char::is_control)
    {
        "Harness login failed"
    } else {
        error
    }
}

fn login_from_row(row: &tokio_postgres::Row) -> HarnessAccountLoginView {
    HarnessAccountLoginView {
        id: row.get("id"),
        account_id: row.get("account_id"),
        runtime_host_id: row.get("runtime_host_id"),
        driver_type: row.get("driver_type"),
        state: row.get("state"),
        authorization_url: row.get("authorization_url"),
        user_code: row.get("user_code"),
        error: row.get("error"),
        expires_at: row
            .get::<_, chrono::DateTime<Utc>>("expires_at")
            .to_rfc3339(),
    }
}

fn describe(error: ApiError) -> String {
    error.0.to_string()
}

/// Record the authorization step. Returns how many rows changed: zero when
/// the login is no longer `authorizing` or has expired.
async fn publish_login(
    client: &deadpool_postgres::Client,
    scope: &LoginScope,
    authorization_url: &str,
    user_code: Option<&str>,
) -> Result<u64, ApiError> {
    client
        .execute(
            "UPDATE harness_account_login
                SET authorization_url = $4, user_code = $5, state = 'awaiting_browser',
                    updated_at = NOW()
              WHERE id = $1 AND company_id = $2 AND runtime_host_id IS NOT DISTINCT FROM $3
                AND driver_type = 'codex_terminal'
                AND state = 'authorizing' AND expires_at > NOW()",
            &[
                &scope.login_id,
                &scope.company_id,
                &scope.runtime_host_id,
                &authorization_url,
                &user_code,
            ],
        )
        .await
        .map_err(internal("publish harness login link"))
}

/// Take the pasted authorization code out of the row so it is delivered to
/// the Harness exactly once.
async fn consume_callback(
    tx: &tokio_postgres::Transaction<'_>,
    scope: &LoginScope,
) -> Result<Option<String>, ApiError> {
    let row = tx
        .query_opt(
            "SELECT callback_code FROM harness_account_login
              WHERE id = $1 AND company_id = $2 AND runtime_host_id IS NOT DISTINCT FROM $3
                AND state = 'authorizing' AND callback_code IS NOT NULL AND expires_at > NOW()
              FOR UPDATE",
            &[&scope.login_id, &scope.company_id, &scope.runtime_host_id],
        )
        .await
        .map_err(internal("read harness authorization code"))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let code: String = row.get("callback_code");
    tx.execute(
        "UPDATE harness_account_login SET callback_code = NULL, updated_at = NOW() WHERE id = $1",
        &[&scope.login_id],
    )
    .await
    .map_err(internal("consume harness authorization code"))?;
    Ok(Some(code))
}

/// Store the verified account identity and close the login. Model and quota
/// discovery may complete independently after this transaction.
async fn complete_login(
    client: &mut deadpool_postgres::Client,
    scope: &LoginScope,
    probe: &AccountProbe,
) -> Result<(), ApiError> {
    validate_authenticated_account(probe)?;
    let tx = client
        .transaction()
        .await
        .map_err(internal("begin complete harness login"))?;
    let row = tx
        .query_opt(
            "SELECT account_id FROM harness_account_login
              WHERE id = $1 AND company_id = $2 AND runtime_host_id IS NOT DISTINCT FROM $3
                AND state IN ('awaiting_browser', 'authorizing') AND expires_at > NOW()
              FOR UPDATE",
            &[&scope.login_id, &scope.company_id, &scope.runtime_host_id],
        )
        .await
        .map_err(internal("find harness login"))?
        .ok_or_else(|| {
            ApiError(AppError::Conflict(
                "harness login is no longer active".into(),
            ))
        })?;
    let account_id: String = row.get("account_id");
    store_authenticated_account(
        &tx,
        &account_id,
        &scope.company_id,
        scope.runtime_host_id.as_deref(),
        probe,
    )
    .await?;
    tx.execute(
        "UPDATE harness_account_login
            SET state = 'verified', completed_at = NOW(), updated_at = NOW(),
                authorization_url = NULL, user_code = NULL, callback_code = NULL
          WHERE id = $1",
        &[&scope.login_id],
    )
    .await
    .map_err(internal("finish harness login"))?;
    tx.commit().await.map_err(internal("commit harness login"))
}

async fn fail_login(
    client: &deadpool_postgres::Client,
    scope: &LoginScope,
    error: &str,
) -> Result<(), ApiError> {
    let error = sanitize_login_error(error);
    client
        .execute(
            "WITH failed_login AS (
                UPDATE harness_account_login
                   SET state = 'failed', error = $4, callback_code = NULL, updated_at = NOW()
                 WHERE id = $1 AND company_id = $2
                   AND runtime_host_id IS NOT DISTINCT FROM $3
                   AND state IN ('authorizing', 'awaiting_browser')
                 RETURNING account_id
             )
             UPDATE harness_account
                SET status = 'error', last_error = $4, updated_at = NOW()
              WHERE id IN (SELECT account_id FROM failed_login)
                AND status <> 'active' AND disabled_at IS NULL",
            &[
                &scope.login_id,
                &scope.company_id,
                &scope.runtime_host_id,
                &error,
            ],
        )
        .await
        .map_err(internal("fail harness login"))?;
    Ok(())
}

/// The gateway's own executor reports straight into the login row.
struct DbLoginSink {
    store: choruz_store::EventStore,
    scope: LoginScope,
    account_id: String,
}

impl LoginSink for DbLoginSink {
    async fn publish(
        &self,
        authorization_url: &str,
        user_code: Option<&str>,
    ) -> Result<(), String> {
        let client = self
            .store
            .connect()
            .await
            .map_err(|error| error.to_string())?;
        let updated = publish_login(&client, &self.scope, authorization_url, user_code)
            .await
            .map_err(describe)?;
        if updated == 0 {
            return Err("harness login is no longer active".into());
        }
        Ok(())
    }

    async fn take_callback(&self) -> Result<Option<String>, String> {
        let mut client = self
            .store
            .connect()
            .await
            .map_err(|error| error.to_string())?;
        let tx = client
            .transaction()
            .await
            .map_err(|error| format!("begin harness authorization code claim: {error}"))?;
        let code = consume_callback(&tx, &self.scope).await.map_err(describe)?;
        tx.commit()
            .await
            .map_err(|error| format!("commit harness authorization code claim: {error}"))?;
        if code.is_none()
            && !login_is_open(&client, &self.scope)
                .await
                .map_err(describe)?
        {
            return Err("harness login was cancelled".into());
        }
        Ok(code)
    }

    async fn complete_authentication(&self, probe: &AccountProbe) -> Result<(), String> {
        let mut client = self
            .store
            .connect()
            .await
            .map_err(|error| error.to_string())?;
        complete_login(&mut client, &self.scope, probe)
            .await
            .map_err(describe)
    }

    async fn publish_snapshot(&self, probe: &AccountProbe) -> Result<(), String> {
        validate_probe(probe).map_err(describe)?;
        let client = self
            .store
            .connect()
            .await
            .map_err(|error| error.to_string())?;
        let updated = store_account_probe(
            &client,
            &self.account_id,
            &self.scope.company_id,
            self.scope.runtime_host_id.as_deref(),
            probe,
        )
        .await
        .map_err(describe)?;
        if updated == 0 {
            return Err("harness account no longer exists".into());
        }
        Ok(())
    }
}

/// Whether the login still waits for the browser; a cancelled or expired
/// row tells the in-process driver to stop.
async fn login_is_open(
    client: &deadpool_postgres::Client,
    scope: &LoginScope,
) -> Result<bool, ApiError> {
    let row = client
        .query_opt(
            "SELECT 1 FROM harness_account_login
              WHERE id = $1 AND company_id = $2 AND runtime_host_id IS NOT DISTINCT FROM $3
                AND state IN ('queued', 'awaiting_browser', 'authorizing') AND expires_at > NOW()",
            &[&scope.login_id, &scope.company_id, &scope.runtime_host_id],
        )
        .await
        .map_err(internal("read harness login state"))?;
    Ok(row.is_some())
}

/// Drive a sign-in for an account on this device to its terminal state.
async fn run_local_login(store: choruz_store::EventStore, job: LoginJob, scope: LoginScope) {
    let sink = DbLoginSink {
        store: store.clone(),
        scope: scope.clone(),
        account_id: job.account_id.clone(),
    };
    let outcome = match run_login(&job, &sink, DEFAULT_LOGIN_TIMEOUT).await {
        Ok(outcome) => {
            if let Some(reason) = outcome.snapshot_error {
                tracing::warn!(login_id = %scope.login_id, %reason, "Harness account snapshot refresh failed after login");
            }
            Ok(())
        }
        Err(reason) => Err(reason),
    };
    if let Err(reason) = outcome {
        tracing::warn!(login_id = %scope.login_id, %reason, "local Harness login failed");
        match store.connect().await {
            Ok(client) => {
                if let Err(error) = fail_login(&client, &scope, &reason).await {
                    tracing::warn!(login_id = %scope.login_id, error = %describe(error), "could not record local Harness login failure");
                }
            }
            Err(error) => {
                tracing::warn!(login_id = %scope.login_id, %error, "could not record local Harness login failure")
            }
        }
    }
}

/// `POST /v1/companies/{company_id}/harness-accounts/{account_id}/probe`:
/// read the account's identity, models and exact quota on the device that
/// holds its login, and store the snapshot. A probe that fails records the
/// reason on the account (`reauth_required` for a missing login, `error`
/// otherwise) and answers 409.
pub(crate) async fn probe_harness_account(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path((company_id, account_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    require_company_access(&headers, &state, &company_id).await?;
    let client = state.event_store.connect().await.map_err(ApiError::from)?;
    let account = client
        .query_opt(
            "SELECT driver_type, profile_kind, runtime_host_id FROM harness_account
              WHERE id = $1 AND company_id = $2 AND disabled_at IS NULL",
            &[&account_id, &company_id],
        )
        .await
        .map_err(internal("find harness account for probe"))?
        .ok_or_else(|| ApiError(AppError::NotFound("harness account not found".into())))?;
    let driver_type: String = account.get("driver_type");
    let profile_kind: String = account.get("profile_kind");
    let runtime_host_id: Option<String> = account.get("runtime_host_id");
    let host = match runtime_host_id.as_deref() {
        Some(host_id) => RuntimeHost::for_host(&state, host_id).map_err(ApiError)?,
        None => RuntimeHost::local(&state),
    };
    let probed: Result<HarnessProbeResult, AppError> = host
        .call(HostRequest::HarnessProbe {
            identity_only: false,
            driver_type,
            account_id: account_id.clone(),
            profile_kind,
        })
        .await;
    match probed {
        Ok(result) => {
            let probe = AccountProbe {
                fingerprint: result.account_fingerprint,
                subscription_type: result.subscription_type,
                models: result.models,
                usage: result.usage,
            };
            crate::handlers_runtime_hosts::validate_probe(&probe)?;
            crate::handlers_runtime_hosts::store_account_probe(
                &client,
                &account_id,
                &company_id,
                runtime_host_id.as_deref(),
                &probe,
            )
            .await?;
            Ok(StatusCode::NO_CONTENT)
        }
        Err(error) => {
            let message = probe_failure_message(&error.to_string());
            let status = if message.contains("sign in") {
                "reauth_required"
            } else {
                "error"
            };
            client
                .execute(
                    "UPDATE harness_account SET status = $2, last_error = $3, updated_at = NOW()
                      WHERE id = $1 AND disabled_at IS NULL",
                    &[&account_id, &status, &message],
                )
                .await
                .map_err(internal("record harness account probe failure"))?;
            Err(ApiError(AppError::Conflict(message)))
        }
    }
}

/// The user-facing reason a probe failed; Harness output never reaches the
/// dashboard verbatim.
fn probe_failure_message(detail: &str) -> String {
    let lower = detail.to_ascii_lowercase();
    if lower.contains("not connected") {
        return detail.trim_start_matches("conflict: ").to_owned();
    }
    if lower.contains("no selectable models") {
        return "This account returned no selectable models; sign in again and verify it".into();
    }
    if lower.contains("auth") || lower.contains("login") || lower.contains("credential") {
        return "Harness login is invalid; sign in to this profile and verify again".into();
    }
    if lower.contains("timed out") {
        return "Harness account probe timed out".into();
    }
    if lower.contains("start ")
        || lower.contains("no such file")
        || lower.contains("not configured")
    {
        return "Harness binary is not configured on this device".into();
    }
    if lower.contains("rate limits") || lower.contains("quota") {
        return "Harness did not return exact quota data for this account".into();
    }
    if lower.contains("identity is unavailable") || lower.contains("account is unavailable") {
        return "Harness account identity is unavailable; sign in and verify again".into();
    }
    "Harness account probe failed".into()
}

pub(crate) async fn start_harness_account_login(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path((company_id, account_id)): Path<(String, String)>,
) -> Result<(StatusCode, Json<HarnessAccountLoginView>), ApiError> {
    let actor = require_company_access(&headers, &state, &company_id).await?;
    crate::auth::require_human_operator(&headers, &state).await?;
    uuid::Uuid::parse_str(&account_id)
        .map_err(|_| ApiError(AppError::Validation("account_id must be a UUID".into())))?;
    let now = Utc::now();
    let expires_at = now + Duration::minutes(HARNESS_LOGIN_TTL_MINUTES);
    let mut client = state.event_store.connect().await.map_err(ApiError::from)?;
    let tx = client
        .transaction()
        .await
        .map_err(internal("begin harness account login"))?;
    let account = tx
        .query_opt(
            "SELECT driver_type, profile_kind, runtime_host_id FROM harness_account
              WHERE id = $1 AND company_id = $2 AND disabled_at IS NULL FOR UPDATE",
            &[&account_id, &company_id],
        )
        .await
        .map_err(internal("find harness account for login"))?
        .ok_or_else(|| ApiError(AppError::NotFound("harness account not found".into())))?;
    let driver_type: String = account.get("driver_type");
    let profile_kind: String = account.get("profile_kind");
    let runtime_host_id: Option<String> = account.get("runtime_host_id");
    let driver = HeadlessDriver::from_driver_type(&driver_type)
        .filter(|driver| matches!(driver, HeadlessDriver::Claude | HeadlessDriver::Codex))
        .ok_or_else(|| {
            ApiError(AppError::Validation(
                "browser sign-in is unsupported for this Harness".into(),
            ))
        })?;
    tx.execute(
        "UPDATE harness_account_login
            SET state = 'expired', authorization_url = NULL, user_code = NULL, callback_code = NULL, updated_at = NOW()
          WHERE account_id = $1
            AND state IN ('queued', 'awaiting_browser', 'authorizing')
            AND (expires_at <= NOW() OR
                 (driver_type = 'claude_terminal' AND
                  (authorization_url IS NOT NULL OR callback_code IS NOT NULL OR state = 'queued')))",
        &[&account_id],
    )
    .await
    .map_err(internal("expire stale harness account logins"))?;
    if let Some(row) = tx
        .query_opt(
            &format!(
                "SELECT {LOGIN_VIEW_COLUMNS} FROM harness_account_login
                  WHERE account_id = $1 AND company_id = $2
                    AND state IN ('queued', 'awaiting_browser', 'authorizing')"
            ),
            &[&account_id, &company_id],
        )
        .await
        .map_err(internal("resume harness account login"))?
    {
        tx.commit()
            .await
            .map_err(internal("commit resumed harness account login"))?;
        return Ok((StatusCode::OK, Json(login_from_row(&row))));
    }
    let login_id = new_id();
    // A remote login waits in `queued` for the connector to claim it; the
    // gateway claims a local login itself in the same transaction.
    let (initial_state, claimed_at) =
        if runtime_host_id.is_some() && driver != HeadlessDriver::Claude {
            ("queued", None)
        } else {
            ("authorizing", Some(now))
        };
    let row = tx
        .query_one(
            &format!(
                "INSERT INTO harness_account_login
                   (id, account_id, company_id, runtime_host_id, driver_type, created_by,
                    expires_at, state, claimed_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                 RETURNING {LOGIN_VIEW_COLUMNS}"
            ),
            &[
                &login_id,
                &account_id,
                &company_id,
                &runtime_host_id,
                &driver_type,
                &actor.id,
                &expires_at,
                &initial_state,
                &claimed_at,
            ],
        )
        .await
        .map_err(|error| {
            if error
                .as_db_error()
                .is_some_and(|db| db.code() == &tokio_postgres::error::SqlState::UNIQUE_VIOLATION)
            {
                ApiError(AppError::Conflict(
                    "this account already has a login in progress".into(),
                ))
            } else {
                internal("queue harness account login")(error)
            }
        })?;
    tx.commit()
        .await
        .map_err(internal("commit harness account login"))?;
    if runtime_host_id.is_none() && driver != HeadlessDriver::Claude {
        let job = LoginJob {
            login_id: login_id.clone(),
            account_id,
            driver,
            profile_kind,
        };
        let scope = LoginScope {
            login_id,
            company_id,
            runtime_host_id: None,
        };
        tokio::spawn(run_local_login(state.event_store.clone(), job, scope));
    }
    Ok((StatusCode::CREATED, Json(login_from_row(&row))))
}

pub(crate) async fn get_harness_account_login(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path((company_id, account_id, login_id)): Path<(String, String, String)>,
) -> Result<Json<HarnessAccountLoginView>, ApiError> {
    require_company_access(&headers, &state, &company_id).await?;
    let client = state.event_store.connect().await.map_err(ApiError::from)?;
    let row = client
        .query_opt(
            "SELECT id, account_id, runtime_host_id, driver_type,
                    CASE WHEN state IN ('queued', 'awaiting_browser', 'authorizing') AND expires_at <= NOW()
                         THEN 'expired' ELSE state END AS state,
                    authorization_url, user_code, error, expires_at
               FROM harness_account_login
              WHERE id = $1 AND company_id = $2 AND account_id = $3",
            &[&login_id, &company_id, &account_id],
        )
        .await
        .map_err(internal("read harness account login"))?
        .ok_or_else(|| ApiError(AppError::NotFound("harness account login not found".into())))?;
    Ok(Json(login_from_row(&row)))
}

pub(crate) async fn submit_harness_account_login_callback(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path((company_id, account_id, login_id)): Path<(String, String, String)>,
    Json(payload): Json<SubmitHarnessAccountLoginCallbackRequest>,
) -> Result<StatusCode, ApiError> {
    require_company_access(&headers, &state, &company_id).await?;
    let code = validate_callback_code(&payload.code)?;
    let client = state.event_store.connect().await.map_err(ApiError::from)?;
    let updated = client
        .execute(
            "UPDATE harness_account_login
                SET callback_code = $4, state = 'authorizing', updated_at = NOW()
              WHERE id = $1 AND company_id = $2 AND account_id = $3
                AND driver_type = 'codex_terminal'
                AND state = 'awaiting_browser' AND expires_at > NOW()",
            &[&login_id, &company_id, &account_id, &code],
        )
        .await
        .map_err(internal("submit harness account callback"))?;
    if updated == 0 {
        return Err(ApiError(AppError::Conflict(
            "this login is no longer waiting for an authorization code".into(),
        )));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Close a sign-in the user gave up on, so the account accepts a fresh one
/// at once instead of after the 15-minute expiry.
pub(crate) async fn cancel_harness_account_login(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path((company_id, account_id, login_id)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    require_company_access(&headers, &state, &company_id).await?;
    let client = state.event_store.connect().await.map_err(ApiError::from)?;
    let updated = client
        .execute(
            "UPDATE harness_account_login
                SET state = 'cancelled', callback_code = NULL, authorization_url = NULL,
                    user_code = NULL, completed_at = NOW(), updated_at = NOW()
              WHERE id = $1 AND company_id = $2 AND account_id = $3
                AND state IN ('queued', 'awaiting_browser', 'authorizing')",
            &[&login_id, &company_id, &account_id],
        )
        .await
        .map_err(internal("cancel harness account login"))?;
    if updated == 0 {
        return Err(ApiError(AppError::Conflict(
            "this login is not open".into(),
        )));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn claim_harness_account_login(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(host_id): Path<String>,
) -> Result<Json<Option<ClaimedHarnessAccountLogin>>, ApiError> {
    let host = require_host(&headers, &state, &host_id).await?;
    let mut client = state.event_store.connect().await.map_err(ApiError::from)?;
    let tx = client
        .transaction()
        .await
        .map_err(internal("begin remote harness login claim"))?;
    tx.execute(
        "UPDATE harness_account_login
            SET state = 'expired', authorization_url = NULL, user_code = NULL, callback_code = NULL, updated_at = NOW()
          WHERE runtime_host_id = $1 AND state IN ('queued', 'awaiting_browser', 'authorizing')
            AND expires_at <= NOW()",
        &[&host.id],
    )
    .await
    .map_err(internal("expire remote harness logins"))?;
    let row = tx
        .query_opt(
            "SELECT login.id, login.account_id, login.driver_type, account.profile_kind
               FROM harness_account_login login
               JOIN harness_account account ON account.id = login.account_id
              WHERE login.runtime_host_id = $1 AND login.company_id = $2
                AND login.driver_type = 'codex_terminal'
                AND login.state = 'queued' AND login.expires_at > NOW()
              ORDER BY login.created_at
              FOR UPDATE SKIP LOCKED
              LIMIT 1",
            &[&host.id, &host.company_id],
        )
        .await
        .map_err(internal("select remote harness login"))?;
    let claimed = if let Some(row) = row {
        let login_id: String = row.get("id");
        tx.execute(
            "UPDATE harness_account_login
                SET claimed_at = NOW(), state = 'authorizing', updated_at = NOW()
              WHERE id = $1",
            &[&login_id],
        )
        .await
        .map_err(internal("claim remote harness login"))?;
        Some(ClaimedHarnessAccountLogin {
            login_id,
            account_id: row.get("account_id"),
            driver_type: row.get("driver_type"),
            profile_kind: row.get("profile_kind"),
        })
    } else {
        None
    };
    tx.commit()
        .await
        .map_err(internal("commit remote harness login claim"))?;
    Ok(Json(claimed))
}

fn host_scope(
    host: &crate::handlers_runtime_hosts::RuntimeHostView,
    login_id: String,
) -> LoginScope {
    LoginScope {
        login_id,
        company_id: host.company_id.clone(),
        runtime_host_id: Some(host.id.clone()),
    }
}

pub(crate) async fn publish_harness_account_login(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path((host_id, login_id)): Path<(String, String)>,
    Json(payload): Json<PublishHarnessAccountLoginRequest>,
) -> Result<StatusCode, ApiError> {
    let host = require_host(&headers, &state, &host_id).await?;
    let url = validate_login_url(&payload.authorization_url)?;
    let user_code = payload
        .user_code
        .as_deref()
        .map(validate_callback_code)
        .transpose()?;
    let client = state.event_store.connect().await.map_err(ApiError::from)?;
    let updated = publish_login(&client, &host_scope(&host, login_id), url, user_code).await?;
    if updated == 0 {
        return Err(ApiError(AppError::Conflict(
            "remote harness login is no longer active".into(),
        )));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn claim_harness_account_login_callback(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path((host_id, login_id)): Path<(String, String)>,
) -> Result<Json<Option<ClaimedHarnessAccountLoginCallback>>, ApiError> {
    let host = require_host(&headers, &state, &host_id).await?;
    let mut client = state.event_store.connect().await.map_err(ApiError::from)?;
    let tx = client
        .transaction()
        .await
        .map_err(internal("begin remote authorization code claim"))?;
    let code = consume_callback(&tx, &host_scope(&host, login_id)).await?;
    tx.commit()
        .await
        .map_err(internal("commit remote authorization code claim"))?;
    Ok(Json(
        code.map(|code| ClaimedHarnessAccountLoginCallback { code }),
    ))
}

pub(crate) async fn complete_harness_account_login(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path((host_id, login_id)): Path<(String, String)>,
    Json(payload): Json<VerifyHarnessAccountRequest>,
) -> Result<StatusCode, ApiError> {
    let host = require_host(&headers, &state, &host_id).await?;
    let probe = payload.into_probe();
    validate_authenticated_account(&probe)?;
    let mut client = state.event_store.connect().await.map_err(ApiError::from)?;
    complete_login(&mut client, &host_scope(&host, login_id), &probe).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn fail_harness_account_login(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path((host_id, login_id)): Path<(String, String)>,
    Json(payload): Json<Value>,
) -> Result<StatusCode, ApiError> {
    let host = require_host(&headers, &state, &host_id).await?;
    let error = payload
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("Harness login failed");
    let client = state.event_store.connect().await.map_err(ApiError::from)?;
    fail_login(&client, &host_scope(&host, login_id), error).await?;
    Ok(StatusCode::NO_CONTENT)
}
