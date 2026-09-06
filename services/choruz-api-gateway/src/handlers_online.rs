use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use choruz_application::db_service::OnlineIdentity;
use choruz_common::{AppError, new_id};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;

use crate::{ApiError, ApiState, require_human_operator};

fn service_url() -> Result<String, AppError> {
    let value = std::env::var("CHORUZ_ONLINE_URL").unwrap_or_else(|_| {
        "https://choruz-remote-control-gateway.jiachengguo778.workers.dev".into()
    });
    let url = reqwest::Url::parse(&value)
        .map_err(|_| AppError::Validation("Invalid CHORUZ_ONLINE_URL".into()))?;
    if (url.scheme() != "https"
        && !(url.scheme() == "http"
            && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(AppError::Validation(
            "CHORUZ_ONLINE_URL must be an HTTPS origin (HTTP is allowed only for loopback tests)"
                .into(),
        ));
    }
    Ok(value.trim_end_matches('/').into())
}

pub(crate) fn client() -> Result<Client, AppError> {
    Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| AppError::Internal("Create Online account client".into()))
}

async fn revoke(identity: &OnlineIdentity) -> Result<(), AppError> {
    let response = client()?
        .post(format!("{}/v1/online/auth/sign-out", identity.service_url))
        .bearer_auth(&identity.session_token)
        .json(&json!({}))
        .send()
        .await
        .map_err(|_| {
            AppError::Internal(
                "Online sign-out could not reach the account service; retry when connected".into(),
            )
        })?;
    if !response.status().is_success() && response.status() != reqwest::StatusCode::UNAUTHORIZED {
        return Err(AppError::Internal(
            "Online account service could not revoke this session".into(),
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Credentials {
    email: String,
    password: String,
    name: Option<String>,
}

async fn authenticate(
    headers: HeaderMap,
    state: ApiState,
    input: Credentials,
    signup: bool,
) -> Result<Json<Value>, ApiError> {
    let actor = require_human_operator(&headers, &state).await?;
    if state.db.online_identity(&actor).await?.is_some() {
        return Err(
            AppError::Conflict("Sign out of Online before changing accounts".into()).into(),
        );
    }
    if input.email.len() > 254
        || input.password.len() > 512
        || input.password.is_empty()
        || (signup
            && input
                .name
                .as_ref()
                .is_none_or(|n| n.trim().is_empty() || n.len() > 80))
    {
        return Err(
            AppError::Validation("Enter a valid email, password and display name".into()).into(),
        );
    }
    let origin = service_url()?;
    let action = if signup {
        "sign-up/email"
    } else {
        "sign-in/email"
    };
    let response = client()?
        .post(format!("{origin}/v1/online/auth/{action}"))
        .json(&json!({"email":input.email.trim(),"password":input.password,"name":input.name}))
        .send()
        .await
        .map_err(|_| {
            AppError::Internal("Online account service is unavailable; retry when connected".into())
        })?;
    if !response.status().is_success() {
        tracing::warn!(
            status = response.status().as_u16(),
            signup,
            "Online authentication rejected"
        );
        return Err(authentication_error(&response, signup).into());
    }
    let data: Value = response.json().await.map_err(|_| {
        AppError::Internal("Online account service returned an invalid response".into())
    })?;
    let field = |value: &Value| {
        value
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                AppError::Internal("Online account service returned an incomplete identity".into())
            })
    };
    let identity = OnlineIdentity {
        account_id: field(&data["user"]["id"])?,
        device_id: new_id(),
        service_url: origin,
        session_token: field(&data["token"])?,
        display_name: field(&data["user"]["name"])?,
    };
    if let Err(error) = state.db.connect_online_identity(&actor, &identity).await {
        if revoke(&identity).await.is_err() {
            tracing::warn!("Could not revoke unbound Online session; it remains subject to expiry");
        }
        return Err(error.into());
    }
    Ok(Json(
        json!({"state":"signed_in","account_id":identity.account_id,"device_id":identity.device_id,"display_name":identity.display_name}),
    ))
}

pub(crate) async fn sign_in(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Json(input): Json<Credentials>,
) -> Result<Json<Value>, ApiError> {
    authenticate(headers, state, input, false).await
}
pub(crate) async fn sign_up(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Json(input): Json<Credentials>,
) -> Result<Json<Value>, ApiError> {
    authenticate(headers, state, input, true).await
}

pub(crate) async fn session(
    headers: HeaderMap,
    State(state): State<ApiState>,
) -> Result<Json<Value>, ApiError> {
    let actor = require_human_operator(&headers, &state).await?;
    let Some(identity) = state.db.online_identity(&actor).await? else {
        return Ok(Json(json!({"state":"signed_out"})));
    };
    let response = client()?
        .get(format!(
            "{}/v1/online/auth/get-session",
            identity.service_url
        ))
        .bearer_auth(&identity.session_token)
        .send()
        .await
        .map_err(|_| AppError::Internal("Online account service is unreachable".into()))?;
    let active = if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        false
    } else if response.status().is_success() {
        let value: Value = response
            .json()
            .await
            .map_err(|_| AppError::Internal("Invalid Online session response".into()))?;
        value["user"]["id"].as_str() == Some(&identity.account_id)
    } else {
        return Err(AppError::Internal("Online session verification failed".into()).into());
    };
    Ok(Json(
        json!({"state":if active {"signed_in"} else {"reauth_required"},"account_id":identity.account_id,"device_id":identity.device_id,"display_name":identity.display_name,"connection":state.online.status(&actor.id)}),
    ))
}

fn authentication_error(response: &reqwest::Response, signup: bool) -> AppError {
    if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let seconds = response
            .headers()
            .get("retry-after")
            .or_else(|| response.headers().get("x-retry-after"))
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(60)
            .max(1);
        return AppError::RateLimited {
            retry_after_ms: seconds.saturating_mul(1000),
        };
    }
    if response.status().is_client_error() {
        AppError::Validation(if signup { "Online registration rejected. Use a valid email and a password of 12–128 characters; the account may already exist." } else { "Online sign-in rejected. Check your email and password, or wait before trying again." }.into())
    } else {
        AppError::Internal("Online account service is unavailable".into())
    }
}

pub(crate) async fn sign_out(
    headers: HeaderMap,
    State(state): State<ApiState>,
) -> Result<StatusCode, ApiError> {
    let actor = require_human_operator(&headers, &state).await?;
    if let Some(identity) = state.db.online_identity(&actor).await? {
        revoke(&identity).await?;
        state
            .db
            .disconnect_online_identity(&actor, &identity.device_id)
            .await?;
        state.online.stop(&actor.id);
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;

    #[test]
    fn account_rate_limit_preserves_retry_delay() {
        for header in ["retry-after", "x-retry-after"] {
            let upstream = reqwest::Response::from(
                axum::http::Response::builder()
                    .status(429)
                    .header(header, "7")
                    .body("")
                    .unwrap(),
            );
            for signup in [false, true] {
                let error = authentication_error(&upstream, signup);
                assert!(matches!(
                    error,
                    AppError::RateLimited {
                        retry_after_ms: 7000
                    }
                ));
                let response = ApiError(error).into_response();
                assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
                assert_eq!(response.headers()["retry-after"], "7");
            }
        }
    }
}
