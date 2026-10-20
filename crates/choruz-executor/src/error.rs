//! Executor error types.

use thiserror::Error;

/// Errors that can occur during executor operations.
#[derive(Debug, Error)]
pub enum ExecutorError {
    /// WAL (write-ahead log) operation failed.
    #[error("wal error: {0}")]
    Wal(String),

    /// SQLite error (local WAL).
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// Convenience type alias.
pub type ExecutorResult<T> = Result<T, ExecutorError>;
