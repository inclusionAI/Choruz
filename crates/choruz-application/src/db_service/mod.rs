//! Database-backed service layer for stateless API gateway.
//!
//! This module progressively replaces the in-memory ChatApp methods
//! with direct PostgreSQL queries. During the migration, both ChatApp
//! (in-memory) and DbService (DB-backed) coexist in ApiState.

mod audit;
mod companies;
mod conversations;
mod events;
mod group_workflow_tasks;
pub(crate) mod helpers;
mod messages;
mod principals;
mod sync;

use std::collections::HashMap;
use std::sync::Mutex;

use choruz_common::AppError;
use choruz_store::EventStore;
use chrono::{DateTime, Duration, Utc};

// ── In-memory rate limiter (Phase 3A) ──────────────────────────────────
//
// Rate limiting is ephemeral by design: it does not need to survive
// restarts and adding a DB round-trip to every write would hurt latency.
// Each gateway instance maintains its own rate limiter — this is the
// industry-standard approach. For multi-instance coordination, switch
// to Redis in the future.

/// Per-principal sliding-window rate limiter (in-memory, per-instance).
pub struct RateLimiter {
    windows: Mutex<HashMap<String, Vec<DateTime<Utc>>>>,
    limit_per_minute: usize,
}

impl RateLimiter {
    pub fn new(limit_per_minute: usize) -> Self {
        Self {
            windows: Mutex::new(HashMap::new()),
            limit_per_minute,
        }
    }

    /// Check (and record) a rate-limit hit for `principal_id`.
    ///
    /// Returns `Ok(())` if under the limit, or `Err(RateLimited)` if the
    /// principal has exceeded `limit_per_minute` requests in the last 60s.
    pub fn check(&self, principal_id: &str) -> Result<(), AppError> {
        let window_start = Utc::now() - Duration::minutes(1);
        let mut windows = self.windows.lock().expect("rate limiter lock");
        let entries = windows.entry(principal_id.to_owned()).or_default();
        entries.retain(|ts| *ts > window_start);
        if entries.len() >= self.limit_per_minute {
            return Err(AppError::RateLimited {
                retry_after_ms: 1000,
            });
        }
        entries.push(Utc::now());
        Ok(())
    }
}

// Intentionally NOT implementing Clone. An earlier version produced a fresh
// empty limiter on clone, which was a footgun — any caller that embedded
// `RateLimiter` in a `#[derive(Clone)]` struct silently got zero rate
// limiting because each request's state clone reset the window map.
// Sharing is the only correct model: `DbService` holds an
// `Arc<RateLimiter>`, so Arc::clone bumps the refcount and every clone
// observes the same window state.

/// Stateless database service that replaces the in-memory ChatApp.
///
/// All methods are async and query PostgreSQL directly.
/// The only in-memory state is the per-instance rate limiter (ephemeral).
#[derive(Clone)]
pub struct DbService {
    store: EventStore,
    rate_limiter: std::sync::Arc<RateLimiter>,
}

impl DbService {
    pub fn new(store: EventStore) -> Self {
        Self::new_with_rate_limit(store, 600)
    }

    pub fn new_with_rate_limit(store: EventStore, rate_limit_per_minute: usize) -> Self {
        group_workflow_tasks::register_channel_task_metrics();
        Self {
            store,
            rate_limiter: std::sync::Arc::new(RateLimiter::new(rate_limit_per_minute)),
        }
    }

    /// Access the underlying EventStore for raw DB operations.
    pub fn store(&self) -> &EventStore {
        &self.store
    }

    /// Check rate limit for a principal (in-memory, per-instance).
    pub fn check_rate_limit(&self, principal_id: &str) -> Result<(), AppError> {
        self.rate_limiter.check(principal_id)
    }
}
