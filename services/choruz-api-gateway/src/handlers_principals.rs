use std::{collections::BTreeSet, net::SocketAddr};

use axum::{
    Json,
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    ApiError, ApiState, authenticated_principal, flush_webhooks, persist_principal_to_db,
    require_actor,
};

// ── Login ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct LocalBootstrapQuery {
    return_port: u16,
}

/// Start a local Choruz browser session without presenting a traditional
/// username/password screen. The browser must connect to the gateway through
/// loopback directly; a reverse proxy or remote client cannot mint a session.
pub(crate) async fn local_bootstrap(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    State(state): State<ApiState>,
    Query(query): Query<LocalBootstrapQuery>,
) -> Result<Response, ApiError> {
    let host_is_loopback = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|host| {
            host == "127.0.0.1"
                || host.starts_with("127.0.0.1:")
                || host == "[::1]"
                || host.starts_with("[::1]:")
        });
    let forwarded = headers.contains_key(header::FORWARDED)
        || headers.contains_key("x-forwarded-for")
        || headers.contains_key("x-real-ip");
    if !peer.ip().is_loopback() || !host_is_loopback || forwarded {
        return Err(ApiError(choruz_common::AppError::Forbidden(
            "local bootstrap requires a direct loopback connection".into(),
        )));
    }
    if query.return_port == 0 {
        return Err(ApiError(choruz_common::AppError::Validation(
            "return_port must be between 1 and 65535".into(),
        )));
    }

    let principal = state.auth.ensure_operator(&state.db).await?;
    persist_principal_to_db(&state.event_store, &principal).await;
    let session_token = state.auth.issue_user_session_token(&principal)?;
    let location = format!("http://127.0.0.1:{}/dashboard", query.return_port);

    let mut response = StatusCode::SEE_OTHER.into_response();
    response.headers_mut().insert(
        header::LOCATION,
        HeaderValue::from_str(&location).map_err(|error| {
            ApiError(choruz_common::AppError::Internal(format!(
                "failed to build local dashboard redirect: {error}"
            )))
        })?,
    );
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&format!(
            "{}={session_token}; HttpOnly; SameSite=Lax; Path=/",
            choruz_auth::SESSION_COOKIE_NAME
        ))
        .map_err(|error| {
            ApiError(choruz_common::AppError::Internal(format!(
                "failed to build local session cookie: {error}"
            )))
        })?,
    );
    Ok(response)
}

#[derive(Debug, Deserialize)]
pub(crate) struct LocalLoginRequest {
    username: String,
    password: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct LocalLoginResponse {
    principal: choruz_domain::Principal,
    session_token: String,
}

pub(crate) async fn local_login(
    State(state): State<ApiState>,
    Json(payload): Json<LocalLoginRequest>,
) -> Result<Json<LocalLoginResponse>, ApiError> {
    let username = payload.username.trim();

    // Path 1 — env-configured local user.
    if username == state.auth.operator_display_name
        && payload.password == state.auth.operator_password
    {
        let principal = state.auth.ensure_operator(&state.db).await?;
        persist_principal_to_db(&state.event_store, &principal).await;
        return Ok(Json(LocalLoginResponse {
            session_token: state.auth.issue_user_session_token(&principal)?,
            principal,
        }));
    }

    // Path 2 — an account registered through signup.
    if let Some(principal) = state.db.find_human_by_username(username).await? {
        if let Some(hash) = principal.secret_hash.as_deref() {
            if choruz_auth::verify_secret(&payload.password, hash) {
                let token = state.auth.issue_user_session_token(&principal)?;
                return Ok(Json(LocalLoginResponse {
                    principal,
                    session_token: token,
                }));
            }
        }
    }

    Err(ApiError(choruz_common::AppError::Unauthorized(
        "invalid local credentials".into(),
    )))
}

// ── Signup ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct LocalSignupRequest {
    username: String,
    password: String,
}

pub(crate) async fn local_signup(
    State(state): State<ApiState>,
    Json(payload): Json<LocalSignupRequest>,
) -> Result<(StatusCode, Json<LocalLoginResponse>), ApiError> {
    // Keep the configured operator's identity unambiguous.
    if payload
        .username
        .trim()
        .eq_ignore_ascii_case(&state.auth.operator_display_name)
    {
        return Err(ApiError(choruz_common::AppError::Conflict(format!(
            "username '{}' is reserved",
            state.auth.operator_display_name
        ))));
    }

    let principal = state
        .db
        .create_human_user(&payload.username, &payload.password)
        .await?;
    let token = state.auth.issue_user_session_token(&principal)?;
    Ok((
        StatusCode::CREATED,
        Json(LocalLoginResponse {
            principal,
            session_token: token,
        }),
    ))
}

// ── Whoami ────────────────────────────────────────────────────────────

/// Verify the incoming session token and return the authenticated principal.
///
/// Used by Next.js `requireAuth()` to validate signature + expiry at the
/// gateway instead of trusting a self-decoded JWT payload. Lightweight —
/// returns only the principal record, not a full console snapshot.
pub(crate) async fn me(
    headers: HeaderMap,
    State(state): State<ApiState>,
) -> Result<Json<choruz_domain::Principal>, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    Ok(Json(principal))
}

// ── Disable principal ─────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct DisablePrincipalQuery {
    actor_id: String,
}

pub(crate) async fn disable_principal(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(principal_id): Path<String>,
    Query(query): Query<DisablePrincipalQuery>,
) -> Result<Json<choruz_domain::Principal>, ApiError> {
    require_actor(&headers, &state, &query.actor_id).await?;

    let principal = state
        .db
        .disable_principal(&query.actor_id, &principal_id)
        .await
        .map_err(ApiError)?;

    // Clean up git worktrees for all bindings owned by this principal.
    // This runs in the background so the response is not delayed.
    let runtime = state.runtime.clone();
    let pid = principal_id.clone();
    tokio::spawn(async move {
        match runtime.list_bindings_by_agent(&pid).await {
            Ok(bindings) => {
                for binding in &bindings {
                    if let Err(e) = runtime.cleanup_worktree(binding).await {
                        tracing::warn!(
                            binding_id = %binding.id,
                            error = %e,
                            "failed to cleanup worktree during principal disable"
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    principal_id = %pid,
                    error = %e,
                    "failed to list bindings for worktree cleanup"
                );
            }
        }
    });

    Ok(Json(principal))
}

// ── Batch disable agents ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct BatchDisableRequest {
    actor_id: String,
    #[serde(default)]
    agent_ids: Vec<String>,
    #[serde(default)]
    conversation_ids: Vec<String>,
}

pub(crate) async fn batch_disable_agents(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Json(payload): Json<BatchDisableRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // Human-only: the authenticated person must also be a real member of
    // every conversation selected for deletion.
    let operator = crate::require_human_operator(&headers, &state).await?;
    require_actor(&headers, &state, &payload.actor_id).await?;
    if payload.conversation_ids.len() > 100 {
        return Err(ApiError(choruz_common::AppError::Validation(
            "at most 100 conversations may be deleted at once".into(),
        )));
    }
    if payload.conversation_ids.iter().any(|id| id.is_empty()) {
        return Err(ApiError(choruz_common::AppError::Validation(
            "conversation ids must not be empty".into(),
        )));
    }
    let conversation_ids: Vec<String> = payload
        .conversation_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut disabled: u64 = 0;
    let mut failed: u64 = 0;
    for agent_id in &payload.agent_ids {
        match state
            .db
            .soft_delete_principal(&payload.actor_id, agent_id)
            .await
        {
            Ok(principal) => {
                disabled += 1;
                state.app.inject_principal(principal);
                // Clean up git worktrees in the background
                let runtime = state.runtime.clone();
                let aid = agent_id.clone();
                tokio::spawn(async move {
                    match runtime.list_bindings_by_agent(&aid).await {
                        Ok(bindings) => {
                            for binding in &bindings {
                                if let Err(e) = runtime.cleanup_worktree(binding).await {
                                    tracing::warn!(
                                        binding_id = %binding.id,
                                        error = %e,
                                        "failed to cleanup worktree during batch disable"
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                agent_id = %aid,
                                error = %e,
                                "failed to list bindings for worktree cleanup"
                            );
                        }
                    }
                });
            }
            Err(e) => {
                tracing::warn!(agent_id = %agent_id, error = %e, "batch disable: failed to disable agent");
                failed += 1;
            }
        }
    }
    // Also delete conversations if requested
    let mut conv_deleted: u64 = 0;
    let mut conv_failed: u64 = 0;
    if !conversation_ids.is_empty() {
        // Validate the complete batch before deleting anything. The SQL
        // membership row is authoritative and the transaction prevents a
        // partial cross-workspace deletion if any ID is unauthorized.
        let mut client = state.event_store.connect().await.map_err(|error| {
            ApiError(choruz_common::AppError::Internal(format!(
                "db connect: {error}"
            )))
        })?;
        let tx = client.transaction().await.map_err(|error| {
            ApiError(choruz_common::AppError::Internal(format!(
                "batch disable transaction: {error}"
            )))
        })?;
        let authorized_count: i64 = tx
            .query_one(
                "SELECT COUNT(*)
                 FROM conversation c
                 JOIN conversation_member cm ON cm.conv_id = c.id
                 WHERE c.id = ANY($1) AND cm.principal_id = $2
                   AND cm.removed_at IS NULL",
                &[&conversation_ids, &operator.id],
            )
            .await
            .map_err(|error| {
                ApiError(choruz_common::AppError::Internal(format!(
                    "batch disable authorization: {error}"
                )))
            })?
            .get(0);
        if authorized_count != conversation_ids.len() as i64 {
            return Err(ApiError(choruz_common::AppError::Forbidden(
                "not a member of every requested conversation".into(),
            )));
        }

        tx.execute(
            "DELETE FROM group_template_role_assignment
             WHERE group_conversation_id = ANY($1)",
            &[&conversation_ids],
        )
        .await
        .map_err(|error| {
            ApiError(choruz_common::AppError::Internal(format!(
                "batch disable delete group role assignments: {error}"
            )))
        })?;
        tx.execute(
            "DELETE FROM group_template_instance
             WHERE group_conversation_id = ANY($1)",
            &[&conversation_ids],
        )
        .await
        .map_err(|error| {
            ApiError(choruz_common::AppError::Internal(format!(
                "batch disable delete group template instances: {error}"
            )))
        })?;
        tx.execute(
            "UPDATE group_provisioning_job SET created_group_id = NULL
             WHERE created_group_id = ANY($1)",
            &[&conversation_ids],
        )
        .await
        .map_err(|error| {
            ApiError(choruz_common::AppError::Internal(format!(
                "batch disable detach provisioning jobs: {error}"
            )))
        })?;
        tx.execute(
            "DELETE FROM conversation_events WHERE conversation_id = ANY($1)",
            &[&conversation_ids],
        )
        .await
        .map_err(|error| {
            ApiError(choruz_common::AppError::Internal(format!(
                "batch disable delete events: {error}"
            )))
        })?;
        let deleted_rows = tx
            .execute(
                "DELETE FROM conversation WHERE id = ANY($1)",
                &[&conversation_ids],
            )
            .await
            .map_err(|error| {
                ApiError(choruz_common::AppError::Internal(format!(
                    "batch disable delete conversation: {error}"
                )))
            })?;
        tx.commit().await.map_err(|error| {
            ApiError(choruz_common::AppError::Internal(format!(
                "batch disable commit: {error}"
            )))
        })?;
        // PostgreSQL is authoritative for this persistent deletion. The
        // in-memory mirror can legitimately miss conversations that were
        // absent from its process-local mirror, so its miss count must not turn
        // an already-committed database deletion into a reported failure.
        let _ = state
            .app
            .delete_conversations_batch(&operator.id, &conversation_ids);
        conv_deleted = deleted_rows;
        conv_failed = (conversation_ids.len() as u64).saturating_sub(deleted_rows);
    }

    Ok(Json(json!({
        "disabled": disabled,
        "failed": failed,
        "conversations_deleted": conv_deleted,
        "conversations_failed": conv_failed,
    })))
}

// ── Create agent ──────────────────────────────────────────────────────

pub(crate) async fn create_agent(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Json(payload): Json<choruz_application::CreateAgentRequest>,
) -> Result<(StatusCode, Json<choruz_application::AgentSecretResponse>), ApiError> {
    require_actor(&headers, &state, &payload.actor_id).await?;
    state.db.check_rate_limit(&payload.actor_id)?;

    let response = state.db.create_agent(payload).await.map_err(ApiError)?;

    // Mirror the newly-created agent into the in-memory `ChatApp` state so
    // immediately-following calls that still read from memory (e.g.
    // `set_event_webhook` for webhook-driver agents) can find it without
    // requiring a gateway restart. Without this the install flow 404s on
    // the webhook registration step.
    state.app.inject_principal(response.principal.clone());

    let _ = flush_webhooks(&state.app).await;

    Ok((StatusCode::CREATED, Json(response)))
}

// ── Rotate agent secret ──────────────────────────────────────────────

pub(crate) async fn rotate_agent_secret(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(agent_id): Path<String>,
    Json(payload): Json<choruz_application::RotateAgentSecretRequest>,
) -> Result<Json<choruz_application::AgentSecretResponse>, ApiError> {
    require_actor(&headers, &state, &payload.actor_id).await?;

    let response = state
        .db
        .rotate_agent_secret(&agent_id, payload)
        .await
        .map_err(ApiError)?;
    let _ = flush_webhooks(&state.app).await;

    Ok(Json(response))
}

// ── Migrate principal workspace ───────────────────────────────────────

pub(crate) async fn migrate_principal_workspace(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(principal_id): Path<String>,
    Json(payload): Json<crate::handlers_conversations::MigrateWorkspaceRequest>,
) -> Result<Json<choruz_domain::Principal>, ApiError> {
    let operator = crate::require_human_operator(&headers, &state).await?;
    if payload.actor_id != operator.id {
        return Err(ApiError(choruz_common::AppError::Forbidden(
            "authenticated principal does not match actor_id".into(),
        )));
    }
    let principal = state.app.migrate_principal_workspace(
        &principal_id,
        &payload.workspace_id,
        &operator.id,
    )?;

    // Persist workspace_id change to DB
    persist_principal_to_db(&state.event_store, &principal).await;

    Ok(Json(principal))
}
