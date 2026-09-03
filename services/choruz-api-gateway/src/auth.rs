use axum::{
    Json,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use choruz_common::AppError;
use choruz_domain::{Principal, PrincipalType};
use serde_json::json;

use crate::ApiState;

// ── Shared types and helpers ──────────────────────────────────────────

#[derive(Debug)]
pub(crate) struct ApiError(pub(crate) AppError);

impl From<AppError> for ApiError {
    fn from(value: AppError) -> Self {
        Self(value)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let error = sanitize_app_error(self.0);
        let status = match &error {
            AppError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::Conflict(_) => StatusCode::CONFLICT,
            AppError::Validation(_) => StatusCode::BAD_REQUEST,
            AppError::Forbidden(_) => StatusCode::FORBIDDEN,
            AppError::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            AppError::Internal(_) => {
                tracing::error!(error = %error, "internal server error");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        (
            status,
            Json(json!({"error": {"status": status.as_u16(), "detail": error.to_string()}})),
        )
            .into_response()
    }
}

// ── Auth helpers (shared across handler modules) ──────────────────────

pub(crate) async fn authenticated_principal(
    headers: &HeaderMap,
    state: &ApiState,
) -> Result<Principal, ApiError> {
    Ok(state.auth.authenticate(headers, &state.db).await?)
}

pub(crate) async fn require_actor(
    headers: &HeaderMap,
    state: &ApiState,
    actor_id: &str,
) -> Result<Principal, ApiError> {
    let principal = authenticated_principal(headers, state).await?;
    if principal.id == actor_id {
        return Ok(principal);
    }

    Err(ApiError(AppError::Unauthorized(
        "authenticated principal does not match actor_id".into(),
    )))
}

pub(crate) async fn require_self(
    headers: &HeaderMap,
    state: &ApiState,
    principal_id: &str,
) -> Result<Principal, ApiError> {
    let principal = authenticated_principal(headers, state).await?;
    if principal.id == principal_id {
        return Ok(principal);
    }

    Err(ApiError(AppError::Unauthorized(
        "authenticated principal does not match requested principal".into(),
    )))
}

/// Authenticate a person who may operate a workspace.
///
/// Agents must keep using their narrow token and conversation permissions;
/// they cannot invoke human control-plane endpoints.
pub(crate) async fn require_human_operator(
    headers: &HeaderMap,
    state: &ApiState,
) -> Result<Principal, ApiError> {
    let principal = authenticated_principal(headers, state).await?;
    if matches!(principal.principal_type, PrincipalType::Human) {
        return Ok(principal);
    }

    Err(ApiError(AppError::Forbidden(
        "only signed-in people can manage workspace agents".into(),
    )))
}

/// Extract a raw bearer token from the Authorization header, if present.
pub(crate) fn bearer_token_value(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

// ── Redaction helpers ─────────────────────────────────────────────────

pub(crate) fn redact_sensitive_text(input: &str) -> String {
    let mut redacted = input.to_owned();
    for marker in [
        "Bearer ",
        "bearer ",
        "secret=",
        "secret:",
        "token=",
        "token:",
        "password=",
        "password:",
    ] {
        redacted = redact_after_marker(&redacted, marker);
    }
    redacted
}

fn redact_after_marker(input: &str, marker: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;
    while let Some(offset) = input[cursor..].find(marker) {
        let start = cursor + offset;
        let value_start = start + marker.len();
        output.push_str(&input[cursor..value_start]);
        let rest = &input[value_start..];
        let skip_ws = rest.len() - rest.trim_start().len();
        let rest = &input[value_start + skip_ws..];
        let end = rest
            .find(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\'' | ',' | ')' | '}' | ']'))
            .map(|index| value_start + skip_ws + index)
            .unwrap_or(input.len());
        output.push_str("[REDACTED]");
        cursor = end;
    }
    output.push_str(&input[cursor..]);
    output
}

fn sanitize_app_error(error: AppError) -> AppError {
    match error {
        AppError::Unauthorized(detail) => AppError::Unauthorized(redact_sensitive_text(&detail)),
        AppError::NotFound(detail) => AppError::NotFound(redact_sensitive_text(&detail)),
        AppError::Conflict(detail) => AppError::Conflict(redact_sensitive_text(&detail)),
        AppError::Validation(detail) => AppError::Validation(redact_sensitive_text(&detail)),
        AppError::Forbidden(detail) => AppError::Forbidden(redact_sensitive_text(&detail)),
        AppError::Internal(detail) => AppError::Internal(redact_sensitive_text(&detail)),
        AppError::RateLimited { retry_after_ms } => AppError::RateLimited { retry_after_ms },
    }
}
