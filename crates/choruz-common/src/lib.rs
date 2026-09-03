use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub mod metrics;
pub mod plugins;

pub type AppResult<T> = Result<T, AppError>;

/// Version of the local host-service readiness contract used by supervisors
/// and SSH clients to reject unrelated or incompatible listeners.
pub const HOST_SERVICE_PROTOCOL_VERSION: u32 = 1;

/// Versioned service identity returned by local lifecycle endpoints.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct HostServiceStatus {
    pub status: String,
    pub service: String,
    pub protocol_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<bool>,
}

impl HostServiceStatus {
    pub fn new(service: impl Into<String>, status: impl Into<String>) -> Self {
        Self {
            status: status.into(),
            service: service.into(),
            protocol_version: HOST_SERVICE_PROTOCOL_VERSION,
            database: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Shared PostgreSQL connection configuration
// ---------------------------------------------------------------------------

/// Shared PostgreSQL connection configuration.
///
/// Reads from environment variables with sensible defaults so that every crate
/// in the workspace resolves database coordinates identically.
#[derive(Debug, Clone)]
pub struct PgConfig {
    pub database_url: Option<String>,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub db: String,
    pub password: Option<String>,
}

impl PgConfig {
    /// Build config from `CHORUZ_PG_*` environment variables.
    ///
    /// Falls back to `CHORUZ_DATABASE_URL` if set (returns a config whose
    /// [`Self::to_connect_string`] reproduces that URL verbatim).
    ///
    /// Individual variable defaults:
    /// - `CHORUZ_PG_HOST`     → `127.0.0.1`
    /// - `CHORUZ_PG_PORT`     → `5432`
    /// - `CHORUZ_PG_USER`     → `$USER` env → `postgres`
    /// - `CHORUZ_PG_DB`       → `choruz`
    /// - `CHORUZ_PG_PASSWORD` → `None`
    pub fn from_env() -> Self {
        let database_url = std::env::var("CHORUZ_DATABASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let host = std::env::var("CHORUZ_PG_HOST").unwrap_or_else(|_| "127.0.0.1".into());
        let port: u16 = std::env::var("CHORUZ_PG_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5432);
        let user = std::env::var("CHORUZ_PG_USER")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| std::env::var("USER").ok())
            .unwrap_or_else(|| "postgres".into());
        let db = std::env::var("CHORUZ_PG_DB").unwrap_or_else(|_| "choruz".into());
        let password = std::env::var("CHORUZ_PG_PASSWORD")
            .ok()
            .filter(|value| !value.trim().is_empty());

        Self {
            database_url,
            host,
            port,
            user,
            db,
            password,
        }
    }

    /// Format as `postgres://user[:pass]@host:port/db`.
    pub fn to_url(&self) -> String {
        if let Some(database_url) = &self.database_url {
            return database_url.clone();
        }
        match &self.password {
            Some(pw) => {
                format!(
                    "postgres://{}:{}@{}:{}/{}",
                    self.user, pw, self.host, self.port, self.db
                )
            }
            None => {
                format!(
                    "postgres://{}@{}:{}/{}",
                    self.user, self.host, self.port, self.db
                )
            }
        }
    }

    /// Format as `host=… port=… user=… dbname=… [password=…]` for
    /// `tokio-postgres` / libpq key-value format.
    pub fn to_connect_string(&self) -> String {
        if let Some(database_url) = &self.database_url {
            return database_url.clone();
        }
        let mut dsn = format!(
            "host={} port={} user={} dbname={}",
            self.host, self.port, self.user, self.db
        );
        if let Some(ref pw) = self.password {
            dsn.push_str(" password=");
            dsn.push_str(pw);
        }
        dsn
    }
}

#[cfg(test)]
mod tests {
    use super::{HOST_SERVICE_PROTOCOL_VERSION, HostServiceStatus, PgConfig};
    use std::sync::{Mutex, MutexGuard};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn host_service_status_uses_the_shared_protocol_version() {
        let status = HostServiceStatus::new("choruz-api-gateway", "ready");
        assert_eq!(status.protocol_version, HOST_SERVICE_PROTOCOL_VERSION);
        assert_eq!(status.database, None);
    }

    struct EnvGuard<'a> {
        _lock: MutexGuard<'a, ()>,
        saved_database_url: Option<String>,
        saved_pg_port: Option<String>,
        saved_legacy_pg_port: Option<String>,
    }

    impl EnvGuard<'_> {
        fn new() -> Self {
            let lock = ENV_LOCK.lock().expect("env lock poisoned");
            let saved_database_url = std::env::var("CHORUZ_DATABASE_URL").ok();
            let saved_pg_port = std::env::var("CHORUZ_PG_PORT").ok();
            let saved_legacy_pg_port = std::env::var("ECHAT_PG_PORT").ok();
            Self {
                _lock: lock,
                saved_database_url,
                saved_pg_port,
                saved_legacy_pg_port,
            }
        }
    }

    impl Drop for EnvGuard<'_> {
        fn drop(&mut self) {
            restore_env_var("CHORUZ_DATABASE_URL", self.saved_database_url.as_deref());
            restore_env_var("CHORUZ_PG_PORT", self.saved_pg_port.as_deref());
            restore_env_var("ECHAT_PG_PORT", self.saved_legacy_pg_port.as_deref());
        }
    }

    fn set_env_var(key: &str, value: &str) {
        // SAFETY: tests that mutate process environment hold ENV_LOCK for
        // their full duration, preventing concurrent access within this crate.
        unsafe {
            std::env::set_var(key, value);
        }
    }

    fn restore_env_var(key: &str, value: Option<&str>) {
        // SAFETY: tests that mutate process environment hold ENV_LOCK for
        // their full duration, preventing concurrent access within this crate.
        unsafe {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }

    #[test]
    fn database_url_takes_precedence_over_component_vars() {
        let _env = EnvGuard::new();
        set_env_var(
            "CHORUZ_DATABASE_URL",
            "postgres://e2e@127.0.0.1:55433/echat",
        );
        set_env_var("CHORUZ_PG_PORT", "5432");

        let config = PgConfig::from_env();

        assert_eq!(
            config.to_connect_string(),
            "postgres://e2e@127.0.0.1:55433/echat"
        );
        assert_eq!(config.to_url(), "postgres://e2e@127.0.0.1:55433/echat");
    }

    #[test]
    fn legacy_component_environment_is_ignored() {
        let _env = EnvGuard::new();
        restore_env_var("CHORUZ_PG_PORT", None);
        set_env_var("ECHAT_PG_PORT", "55433");

        assert_eq!(PgConfig::from_env().port, 5432);
    }
}

#[derive(Debug, Clone, Error, Serialize, PartialEq, Eq)]
#[serde(tag = "code", content = "detail")]
pub enum AppError {
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("rate limited")]
    RateLimited { retry_after_ms: u64 },
    #[error("internal error: {0}")]
    Internal(String),
}

pub fn now() -> DateTime<Utc> {
    Utc::now()
}

pub fn new_id() -> String {
    Uuid::now_v7().to_string()
}
