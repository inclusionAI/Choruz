//! Connection pool wrapper for the event store.

use choruz_common::{AppError, AppResult};
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};

/// Shared database connection pool for the event store.
#[derive(Clone)]
pub struct EventStore {
    pool: Pool,
    database_url: String,
}

impl EventStore {
    /// Create a new event store from a PostgreSQL connection string.
    ///
    /// The connection string uses libpq key=value format, e.g.
    /// `"host=127.0.0.1 port=5432 user=me dbname=choruz"`.
    pub fn new(database_url: impl Into<String>) -> Self {
        let url = database_url.into();
        Self {
            pool: build_pool(&url),
            database_url: url,
        }
    }

    /// Obtain a connection from the pool.
    pub async fn connect(&self) -> AppResult<deadpool_postgres::Client> {
        self.pool
            .get()
            .await
            .map_err(|error| AppError::Internal(format!("event store pool error: {error}")))
    }

    /// Connection string for dedicated PostgreSQL connections such as
    /// LISTEN/NOTIFY consumers. Those connections must not occupy pool slots.
    pub fn database_url(&self) -> &str {
        &self.database_url
    }

    /// Run a lightweight query to verify database connectivity.
    pub async fn health_check(&self) -> AppResult<()> {
        let client = self.connect().await?;
        client.execute("SELECT 1", &[]).await.map_err(|error| {
            AppError::Internal(format!("event store health check failed: {error}"))
        })?;
        Ok(())
    }
}

fn build_pool(database_url: &str) -> Pool {
    let mut pg_config: tokio_postgres::Config = database_url
        .parse()
        .expect("invalid database connection string");
    pg_config.connect_timeout(std::time::Duration::from_secs(5));
    let mgr_config = ManagerConfig {
        // Verified: run "SELECT 1" before handing out a connection.
        // Prevents "db error" from stale/dead connections in the pool.
        recycling_method: RecyclingMethod::Verified,
    };
    let mgr = Manager::from_config(pg_config, tokio_postgres::NoTls, mgr_config);
    // 32 covers realistic concurrent load (multiple users + agents + CDC poller
    // + LISTEN + outbox publishers). The earlier cap of 8 could not absorb
    // even a 10-way concurrent send_message burst — requests queued on
    // `pool.get()` until `connect_timeout` expired.
    let max_size = std::env::var("CHORUZ_DB_POOL_SIZE")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(32);
    Pool::builder(mgr)
        .max_size(max_size)
        .build()
        .expect("failed to build event store connection pool")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_does_not_panic() {
        // Just verifies that building the pool from env defaults doesn't panic.
        let url = choruz_common::PgConfig::from_env().to_connect_string();
        let _store = EventStore::new(url);
    }
}
