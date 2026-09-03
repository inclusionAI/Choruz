//! Configuration for the choruz-pipeline process.
//!
//! All values are read from environment variables with sensible defaults.

use std::time::Duration;

pub(crate) const DISPATCH_HEARTBEAT_INTERVAL_SECS: i64 = 15;
const MIN_LEASE_TIMEOUT_SECS: i64 = DISPATCH_HEARTBEAT_INTERVAL_SECS * 3;

/// Top-level configuration for the pipeline process.
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// libpq-style connection string for the event store / session store.
    pub database_url: String,

    /// How often the CDC poller checks for new outbox entries (ms).
    pub cdc_poll_interval_ms: u64,
    /// Maximum outbox entries claimed per CDC poll cycle.
    pub cdc_batch_size: i64,

    /// Interval for the dispatch loop to pick up pending commands (ms).
    pub dispatch_interval_ms: u64,
    /// Maximum pending commands claimed per dispatch cycle.
    pub dispatch_batch_size: i64,

    /// Interval for the lease monitor to check expired leases (ms).
    pub lease_check_interval_ms: u64,
    /// Heartbeat timeout in seconds: a lease is expired if the session's
    /// last_heartbeat_at is older than this.
    pub lease_timeout_secs: i64,

    /// Interval for the retry scheduler to check retriable commands (ms).
    pub retry_check_interval_ms: u64,
    /// Maximum retriable commands claimed per cycle.
    pub retry_batch_size: i64,

    /// Port for the /metrics and /health HTTP server.
    pub metrics_port: u16,
    /// Bind address for the metrics server.
    pub metrics_host: String,

    /// This node's executor ID (registered in the in-memory executor registry).
    pub node_id: String,

    /// Base URL of the API Gateway.
    /// Kept for backward compatibility; the executor no longer uses gateway PTY.
    #[allow(dead_code)]
    pub gateway_base_url: String,

    /// Maximum time (seconds) to wait for an agent response before timing out.
    pub executor_timeout_secs: u64,

    /// Base directory for agent sandboxes (workspace directories).
    pub sandbox_base_dir: String,

    /// Path to the `claude` CLI binary (default `claude`).
    pub claude_cli_path: String,

    /// Path to the `codex` CLI binary (default `codex`).
    pub codex_cli_path: String,

    /// Path to the `pi` CLI binary (default `pi`).
    pub pi_cli_path: String,

    /// Path to the `grok` CLI binary (default `grok`).
    pub grok_cli_path: String,

    /// Path to the `opencode` CLI binary (default `opencode`).
    pub opencode_cli_path: String,

    /// Path to the `mathcode` CLI binary (default `mathcode`).
    pub mathcode_cli_path: String,
}

impl PipelineConfig {
    /// Build configuration from environment variables.
    pub fn from_env() -> Self {
        let database_url = env_or("CHORUZ_DATABASE_URL", &database_url_from_components());

        Self {
            database_url,
            cdc_poll_interval_ms: env_u64("CHORUZ_PIPELINE_CDC_POLL_MS", 500),
            cdc_batch_size: env_i64("CHORUZ_PIPELINE_CDC_BATCH", 100),
            dispatch_interval_ms: env_u64("CHORUZ_PIPELINE_DISPATCH_MS", 1000),
            dispatch_batch_size: env_i64("CHORUZ_PIPELINE_DISPATCH_BATCH", 50),
            lease_check_interval_ms: env_u64("CHORUZ_PIPELINE_LEASE_CHECK_MS", 10_000),
            lease_timeout_secs: env_i64("CHORUZ_PIPELINE_LEASE_TIMEOUT_SECS", 60),
            retry_check_interval_ms: env_u64("CHORUZ_PIPELINE_RETRY_CHECK_MS", 5_000),
            retry_batch_size: env_i64("CHORUZ_PIPELINE_RETRY_BATCH", 20),
            metrics_port: env_u64("CHORUZ_PIPELINE_METRICS_PORT", 3020) as u16,
            metrics_host: env_or("CHORUZ_PIPELINE_METRICS_HOST", "127.0.0.1"),
            node_id: env_or("CHORUZ_PIPELINE_NODE_ID", "pipeline-local-1"),
            gateway_base_url: env_or("CHORUZ_API_BASE_URL", "http://127.0.0.1:3000"),
            // Hard kill timeout for one headless coding-agent invocation.
            // 30 min matches claude-code's
            // REMOTE_REVIEW_TIMEOUT_MS — a long-running tool turn easily
            // takes 5-10 min, but anything past 30 min is almost certainly
            // a hung process (e.g. claude's internal retry loop on Anthropic
            // 429s pinning a slot). Override via env for workloads with
            // genuinely longer tool turns.
            executor_timeout_secs: env_u64("CHORUZ_PIPELINE_EXECUTOR_TIMEOUT_SECS", 1800),
            sandbox_base_dir: env_or("CHORUZ_PIPELINE_SANDBOX_DIR", "/tmp/choruz-sandboxes"),
            claude_cli_path: env_or("CHORUZ_CLAUDE_CLI_PATH", "claude"),
            codex_cli_path: env_or("CHORUZ_CODEX_CLI_PATH", "codex"),
            pi_cli_path: env_or("CHORUZ_PI_CLI_PATH", "pi"),
            grok_cli_path: env_or("CHORUZ_GROK_CLI_PATH", "grok"),
            opencode_cli_path: env_or("CHORUZ_OPENCODE_CLI_PATH", "opencode"),
            mathcode_cli_path: env_or("CHORUZ_MATHCODE_BINARY", "mathcode"),
        }
    }

    pub fn cdc_poll_interval(&self) -> Duration {
        Duration::from_millis(self.cdc_poll_interval_ms)
    }

    pub fn dispatch_interval(&self) -> Duration {
        Duration::from_millis(self.dispatch_interval_ms)
    }

    pub fn lease_check_interval(&self) -> Duration {
        Duration::from_millis(self.lease_check_interval_ms)
    }

    pub fn retry_check_interval(&self) -> Duration {
        Duration::from_millis(self.retry_check_interval_ms)
    }

    pub fn executor_timeout(&self) -> Duration {
        Duration::from_secs(self.executor_timeout_secs)
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.lease_timeout_secs < MIN_LEASE_TIMEOUT_SECS {
            return Err(format!(
                "CHORUZ_PIPELINE_LEASE_TIMEOUT_SECS must be at least {MIN_LEASE_TIMEOUT_SECS} seconds (three heartbeat intervals)"
            ));
        }
        Ok(())
    }
}

/// Build a libpq connection string from individual component env vars.
fn database_url_from_components() -> String {
    choruz_common::PgConfig::from_env().to_connect_string()
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.into())
}

fn env_u64(key: &str, default: u64) -> u64 {
    match std::env::var(key) {
        Err(_) => default,
        Ok(v) => v.parse().unwrap_or_else(|e| {
            tracing::warn!(
                key,
                value = %v,
                error = %e,
                default,
                "invalid numeric env var; using default (previously silently fell back)"
            );
            default
        }),
    }
}

fn env_i64(key: &str, default: i64) -> i64 {
    match std::env::var(key) {
        Err(_) => default,
        Ok(v) => v.parse().unwrap_or_else(|e| {
            tracing::warn!(
                key,
                value = %v,
                error = %e,
                default,
                "invalid numeric env var; using default (previously silently fell back)"
            );
            default
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        let config = PipelineConfig::from_env();
        assert!(!config.database_url.is_empty());
        assert!(config.cdc_batch_size > 0);
        assert!(config.dispatch_batch_size > 0);
        assert!(config.lease_timeout_secs > 0);
        assert!(config.metrics_port > 0);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn lease_timeout_requires_heartbeat_safety_margin() {
        let mut config = PipelineConfig::from_env();
        config.lease_timeout_secs = MIN_LEASE_TIMEOUT_SECS - 1;
        assert!(config.validate().is_err());
        config.lease_timeout_secs = MIN_LEASE_TIMEOUT_SECS;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn intervals_produce_durations() {
        let config = PipelineConfig::from_env();
        assert!(config.cdc_poll_interval().as_millis() > 0);
        assert!(config.dispatch_interval().as_millis() > 0);
        assert!(config.lease_check_interval().as_millis() > 0);
        assert!(config.retry_check_interval().as_millis() > 0);
    }
}
