//! Redis cache layer with graceful fallback.
//!
//! All operations return Ok(None) / Ok(()) on Redis failures instead of
//! propagating errors, because Redis is a cache — DB is source of truth.

use deadpool_redis::{Config, Pool, Runtime};
use redis::AsyncCommands;

const DEFAULT_TTL_SECS: u64 = 3600; // 1 hour

pub struct RedisCache {
    pool: Pool,
}

impl RedisCache {
    /// Create a new Redis cache. Panics if the URL is invalid.
    pub fn new(redis_url: &str) -> Self {
        let cfg = Config::from_url(redis_url);
        let pool = cfg.create_pool(Some(Runtime::Tokio1)).expect("redis pool");
        Self { pool }
    }

    /// Get a value by key. Returns None on miss or Redis failure.
    pub async fn get(&self, key: &str) -> Option<String> {
        let mut conn = self.pool.get().await.ok()?;
        conn.get(key).await.ok()
    }

    /// Set a value with default TTL. Silently fails on Redis error.
    pub async fn set(&self, key: &str, value: &str) {
        if let Ok(mut conn) = self.pool.get().await {
            let _: Result<(), _> = conn.set_ex(key, value, DEFAULT_TTL_SECS).await;
        }
    }

    /// Delete a key. Silently fails on Redis error.
    pub async fn del(&self, key: &str) {
        if let Ok(mut conn) = self.pool.get().await {
            let _: Result<(), _> = conn.del(key).await;
        }
    }

    /// Check if Redis is reachable.
    pub async fn ping(&self) -> bool {
        if let Ok(mut conn) = self.pool.get().await {
            redis::cmd("PING")
                .query_async::<String>(&mut *conn)
                .await
                .is_ok()
        } else {
            false
        }
    }
}

impl Clone for RedisCache {
    fn clone(&self) -> Self {
        Self {
            pool: self.pool.clone(),
        }
    }
}
