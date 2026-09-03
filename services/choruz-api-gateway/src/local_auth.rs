use axum::http::{HeaderMap, header::AUTHORIZATION};
use choruz_auth::{
    SESSION_COOKIE_NAME, SessionClaims, issue_session_token, local_user_principal_id,
    verify_session_token,
};
use choruz_common::{AppError, AppResult};
use choruz_domain::Principal;
use chrono::{Duration, Utc};

use choruz_application::{ChatApp, DbService};

#[derive(Clone)]
pub struct LocalAuthConfig {
    pub operator_workspace_id: String,
    pub operator_display_name: String,
    pub operator_password: String,
    pub session_secret: String,
    pub session_ttl: Duration,
}

impl LocalAuthConfig {
    /// In production mode, require explicit secret configuration.
    /// Panics if CHORUZ_ENV=production and secrets use default values.
    pub fn validate_production_env() {
        if std::env::var("CHORUZ_ENV").as_deref() != Ok("production") {
            return; // Development mode: defaults are fine
        }

        if std::env::var("CHORUZ_SESSION_SECRET").is_err() {
            eprintln!("FATAL: CHORUZ_SESSION_SECRET must be set when CHORUZ_ENV=production");
            std::process::exit(1);
        }
        if std::env::var("CHORUZ_OPERATOR_PASSWORD").is_err() {
            eprintln!("FATAL: CHORUZ_OPERATOR_PASSWORD must be set when CHORUZ_ENV=production");
            std::process::exit(1);
        }
    }

    /// Construct from pre-parsed [`Config`](crate::config::Config) fields.
    ///
    /// Replaces the old `from_env()` so that all env reads happen in one place.
    pub fn new(
        session_secret: String,
        operator_password: String,
        operator_workspace_id: String,
        operator_display_name: String,
        session_ttl_hours: i64,
    ) -> Self {
        if session_secret == "choruz-local-session-secret" {
            tracing::warn!(
                "CHORUZ_SESSION_SECRET not set, using insecure default — DO NOT USE IN PRODUCTION"
            );
        }
        if operator_password == "choruz-local" {
            tracing::warn!(
                "CHORUZ_OPERATOR_PASSWORD not set, using insecure default — DO NOT USE IN PRODUCTION"
            );
        }
        Self {
            operator_workspace_id,
            operator_display_name,
            operator_password,
            session_secret,
            session_ttl: Duration::hours(session_ttl_hours),
        }
    }

    pub fn from_env() -> Self {
        let session_secret = match std::env::var("CHORUZ_SESSION_SECRET") {
            Ok(v) => v,
            Err(_) => {
                tracing::warn!(
                    "CHORUZ_SESSION_SECRET not set, using insecure default — DO NOT USE IN PRODUCTION"
                );
                "choruz-local-session-secret".into()
            }
        };
        let operator_password = match std::env::var("CHORUZ_OPERATOR_PASSWORD") {
            Ok(v) => v,
            Err(_) => {
                tracing::warn!(
                    "CHORUZ_OPERATOR_PASSWORD not set, using insecure default — DO NOT USE IN PRODUCTION"
                );
                "choruz-local".into()
            }
        };
        Self {
            operator_workspace_id: std::env::var("CHORUZ_OPERATOR_WORKSPACE")
                .unwrap_or_else(|_| "ws-local".into()),
            operator_display_name: std::env::var("CHORUZ_OPERATOR_USER")
                .unwrap_or_else(|_| "operator".into()),
            operator_password,
            session_secret,
            session_ttl: Duration::hours(
                std::env::var("CHORUZ_SESSION_TTL_HOURS")
                    .ok()
                    .and_then(|value| value.parse::<i64>().ok())
                    .unwrap_or(87600), // ~10 years, effectively never expires
            ),
        }
    }

    pub fn ensure_operator_sync(&self, app: &ChatApp) -> AppResult<Principal> {
        app.ensure_local_operator(&self.operator_workspace_id, &self.operator_display_name)
    }

    pub async fn ensure_operator(&self, db: &DbService) -> AppResult<Principal> {
        db.ensure_local_operator(&self.operator_workspace_id, &self.operator_display_name)
            .await
    }

    pub fn operator_principal_id(&self) -> String {
        local_user_principal_id(&self.operator_workspace_id, &self.operator_display_name)
    }

    /// Issue a session token for a freshly authenticated/registered human user.
    /// Gateway middleware re-resolves the principal ID on each request.
    pub fn issue_user_session_token(&self, principal: &Principal) -> AppResult<String> {
        issue_session_token(
            &SessionClaims {
                principal_id: principal.id.clone(),
                workspace_id: principal.workspace_id.clone(),
                display_name: principal.name.clone(),
                expires_at_epoch_s: (Utc::now() + self.session_ttl).timestamp(),
            },
            &self.session_secret,
        )
        .map_err(|error| AppError::Internal(format!("failed to issue session token: {error}")))
    }

    pub async fn authenticate(&self, headers: &HeaderMap, db: &DbService) -> AppResult<Principal> {
        if let Some(token) = bearer_token(headers) {
            return self.authenticate_token(&token, db).await;
        }

        if let Some(token) = cookie_value(headers, SESSION_COOKIE_NAME) {
            return self.authenticate_token(&token, db).await;
        }

        Err(AppError::Unauthorized("missing credentials".into()))
    }

    async fn authenticate_token(&self, token: &str, db: &DbService) -> AppResult<Principal> {
        if let Some(claims) =
            verify_session_token(token, &self.session_secret, Utc::now().timestamp())
        {
            return db.get_principal(&claims.principal_id).await;
        }

        db.authenticate_agent_secret(token).await
    }
}

pub fn cookie_name() -> &'static str {
    SESSION_COOKIE_NAME
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let header = headers.get(AUTHORIZATION)?.to_str().ok()?;
    header.strip_prefix("Bearer ").map(str::to_owned)
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let cookies = headers.get("cookie")?.to_str().ok()?;
    for cookie in cookies.split(';') {
        let cookie = cookie.trim();
        let (candidate_name, value) = cookie.split_once('=')?;
        if candidate_name == name {
            return Some(value.to_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{SESSION_COOKIE_NAME, cookie_value};
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn legacy_session_cookie_is_not_accepted_or_selected() {
        let mut legacy_only = HeaderMap::new();
        legacy_only.insert(
            "cookie",
            HeaderValue::from_static("echat_session=legacy-token"),
        );
        assert_eq!(cookie_value(&legacy_only, SESSION_COOKIE_NAME), None);

        let mut both = HeaderMap::new();
        both.insert(
            "cookie",
            HeaderValue::from_static("echat_session=legacy-token; choruz_session=current-token"),
        );
        assert_eq!(
            cookie_value(&both, SESSION_COOKIE_NAME).as_deref(),
            Some("current-token")
        );
    }
}
