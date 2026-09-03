//! The browser sign-in of a harness account.
//!
//! A login row moves `queued` → `authorizing` → `awaiting_browser` (link
//! published) → `authorizing` (code pasted) → `verified`, or ends `failed`,
//! `cancelled` or `expired`. The company-facing routes here start a login,
//! read it and accept the pasted authorization code. Who runs the Harness
//! depends on where the account lives: an account on a remote runtime host
//! is claimed and driven by `choruz-connector` through the host-facing
//! routes, and an account on the gateway's own device (`runtime_host_id`
//! NULL) is driven in-process by [`run_local_login`] through
//! [`DbLoginSink`]. Both executors share `choruz-harness-login`; OAuth
//! tokens never enter the database.

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use choruz_agent_runtime::headless::HeadlessDriver;
use choruz_common::{AppError, new_id};
use choruz_harness_login::{
    AccountProbe, CodexLoginLocation, DEFAULT_LOGIN_TIMEOUT, LoginJob, LoginSink, run_login,
};
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
};

const HARNESS_LOGIN_TTL_MINUTES: i64 = 15;
const MAX_LOGIN_ERROR_LEN: usize = 500;
const LOGIN_VIEW_COLUMNS: &str = "id, account_id, runtime_host_id, driver_type, state,
                                  authorization_url, user_code, error, expires_at";

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

pub(crate) async fn start_harness_account_login(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path((company_id, account_id)): Path<(String, String)>,
) -> Result<(StatusCode, Json<HarnessAccountLoginView>), ApiError> {
    let actor = require_company_access(&headers, &state, &company_id).await?;
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
            SET state = 'expired', updated_at = NOW()
          WHERE account_id = $1
            AND state IN ('queued', 'awaiting_browser', 'authorizing')
            AND expires_at <= NOW()",
        &[&account_id],
    )
    .await
    .map_err(internal("expire stale harness account logins"))?;
    let login_id = new_id();
    // A remote login waits in `queued` for the connector to claim it; the
    // gateway claims a local login itself in the same transaction.
    let (initial_state, claimed_at) = if runtime_host_id.is_some() {
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
    if runtime_host_id.is_none() {
        let job = LoginJob {
            login_id: login_id.clone(),
            account_id,
            driver,
            profile_kind,
            codex_login_location: CodexLoginLocation::Local,
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
            SET state = 'expired', updated_at = NOW()
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
