use std::path::PathBuf;

/// Top-level configuration for the choruz-api-gateway process.
///
/// All values are read from environment variables once at startup via
/// [`Config::from_env()`].  Defaults match the previous inline behaviour.
///
/// # Environment variables
///
/// - `CHORUZ_API_PORT`           — HTTP listen port (default `3000`)
/// - `CHORUZ_API_HOST`           — HTTP bind address (default `127.0.0.1`)
/// - `CHORUZ_ATTACHMENT_DIR`     — Directory for file attachments (default `.choruz-runtime/attachments`)
/// - `CHORUZ_AGENT_TOKENS_FILE`  — Path to agent_tokens.json (default `.choruz-runtime/agent_tokens.json`)
/// - `CHORUZ_ENV`                — Environment name; `production` enforces strict secret validation
/// - `CHORUZ_SESSION_SECRET`     — HMAC secret for session tokens (insecure default in dev)
/// - `CHORUZ_OPERATOR_PASSWORD`  — Local user password (insecure default in dev)
/// - `CHORUZ_OPERATOR_WORKSPACE` — Local user workspace ID (default `ws-local`)
/// - `CHORUZ_OPERATOR_USER`      — Local user display name (default `operator`)
/// - `CHORUZ_SESSION_TTL_HOURS`  — Session token lifetime in hours (default `87600` ≈ 10 years)
/// - `CHORUZ_DATABASE_URL`       — libpq connection string (falls back to component vars)
#[derive(Debug, Clone)]
pub struct Config {
    // ── Network ──
    pub api_port: u16,
    pub api_host: String,

    // ── Paths ──
    pub attachment_dir: PathBuf,
    pub agent_tokens_file: PathBuf,

    // ── Auth / sessions ──
    pub env_name: String,
    pub session_secret: String,
    pub operator_password: String,
    pub operator_workspace: String,
    pub operator_user: String,
    pub session_ttl_hours: i64,
}

impl Config {
    /// Build configuration from environment variables.
    ///
    /// Returns `Err` only if a truly invalid value is found (e.g. a port
    /// that is not a number).  Missing optional vars use their defaults.
    pub fn from_env() -> Result<Self, String> {
        let env_name = std::env::var("CHORUZ_ENV").unwrap_or_else(|_| "development".into());

        let session_secret =
            required_or_default("CHORUZ_SESSION_SECRET", "choruz-local-session-secret")?;

        let operator_password = required_or_default("CHORUZ_OPERATOR_PASSWORD", "choruz-local")?;

        let session_ttl_hours: i64 = match std::env::var("CHORUZ_SESSION_TTL_HOURS") {
            Ok(v) => v
                .parse::<i64>()
                .map_err(|e| format!("CHORUZ_SESSION_TTL_HOURS is not a valid integer: {e}"))?,
            Err(_) => 87600, // ~10 years
        };

        let api_port: u16 = match std::env::var("CHORUZ_API_PORT") {
            Ok(v) => v
                .parse::<u16>()
                .map_err(|e| format!("CHORUZ_API_PORT is not a valid u16: {e}"))?,
            Err(_) => 3000,
        };

        Ok(Self {
            api_port,
            api_host: std::env::var("CHORUZ_API_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
            attachment_dir: PathBuf::from(
                std::env::var("CHORUZ_ATTACHMENT_DIR")
                    .unwrap_or_else(|_| ".choruz-runtime/attachments".into()),
            ),
            agent_tokens_file: PathBuf::from(
                std::env::var("CHORUZ_AGENT_TOKENS_FILE")
                    .unwrap_or_else(|_| ".choruz-runtime/agent_tokens.json".into()),
            ),
            env_name,
            session_secret,
            operator_password,
            operator_workspace: std::env::var("CHORUZ_OPERATOR_WORKSPACE")
                .unwrap_or_else(|_| "ws-local".into()),
            operator_user: std::env::var("CHORUZ_OPERATOR_USER")
                .unwrap_or_else(|_| "operator".into()),
            session_ttl_hours,
        })
    }

    /// Validate that production mode has required secrets set.
    /// Panics if `CHORUZ_ENV=production` and secrets are still at defaults.
    pub fn validate_production(&self) {
        if self.env_name != "production" {
            return;
        }
        if self.session_secret == "choruz-local-session-secret" {
            eprintln!("FATAL: CHORUZ_SESSION_SECRET must be set when CHORUZ_ENV=production");
            std::process::exit(1);
        }
        if self.operator_password == "choruz-local" {
            eprintln!("FATAL: CHORUZ_OPERATOR_PASSWORD must be set when CHORUZ_ENV=production");
            std::process::exit(1);
        }
    }
}

fn required_or_default(name: &str, default: &str) -> Result<String, String> {
    required_or_default_value(name, std::env::var(name), default)
}

fn required_or_default_value(
    name: &str,
    value: Result<String, std::env::VarError>,
    default: &str,
) -> Result<String, String> {
    match value {
        Ok(value) if value.trim().is_empty() => Err(format!("{name} must not be empty")),
        Ok(value) => Ok(value),
        Err(std::env::VarError::NotPresent) => Ok(default.to_string()),
        Err(error) => Err(format!("unable to read {name}: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::required_or_default_value;

    #[test]
    fn explicit_empty_secret_is_rejected() {
        let error = required_or_default_value(
            "CHORUZ_SESSION_SECRET",
            Ok("  ".to_string()),
            "choruz-local-session-secret",
        )
        .expect_err("empty secret must not silently select the default");

        assert_eq!(error, "CHORUZ_SESSION_SECRET must not be empty");
    }

    #[test]
    fn missing_secret_uses_the_characterized_default() {
        let value = required_or_default_value(
            "CHORUZ_SESSION_SECRET",
            Err(std::env::VarError::NotPresent),
            "choruz-local-session-secret",
        )
        .expect("unset secret should use the development default");

        assert_eq!(value, "choruz-local-session-secret");
    }
}
