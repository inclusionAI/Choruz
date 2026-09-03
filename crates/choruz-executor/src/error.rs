//! Executor error types.

use thiserror::Error;

/// Errors that can occur during executor operations.
#[derive(Debug, Error)]
pub enum ExecutorError {
    /// Sandbox creation or management failed.
    #[error("sandbox error: {0}")]
    Sandbox(String),

    /// CLI adapter injection or response reading failed.
    #[error("adapter error: {0}")]
    Adapter(String),

    /// WAL (write-ahead log) operation failed.
    #[error("wal error: {0}")]
    Wal(String),

    /// Epoch mismatch — the executor's epoch is stale.
    #[error("epoch mismatch: expected {expected}, got {actual}")]
    EpochMismatch { expected: i32, actual: i32 },

    /// Session-level error (delegated from choruz-session).
    #[error("session error: {0}")]
    Session(#[from] choruz_session::SessionError),

    /// Filesystem I/O error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization error.
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// SQLite error (local WAL).
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// The operation timed out.
    #[error("timeout: {0}")]
    Timeout(String),

    /// The response was empty or invalid.
    #[error("empty response from CLI")]
    EmptyResponse,

    /// Generic internal error.
    #[error("internal error: {0}")]
    Internal(String),
}

impl ExecutorError {
    /// Returns `true` if this error is retryable by the session manager.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Timeout(_) | Self::Io(_) | Self::Adapter(_) | Self::Sandbox(_)
        )
    }
}

/// Convenience type alias.
pub type ExecutorResult<T> = Result<T, ExecutorError>;
