//! Session Manager error types.

use thiserror::Error;

/// Errors that can occur in session management operations.
#[derive(Debug, Error)]
pub enum SessionError {
    /// The requested session was not found.
    #[error("session not found: {0}")]
    SessionNotFound(String),

    /// The requested command was not found.
    #[error("command not found: {0}")]
    CommandNotFound(String),

    /// A result or state transition belongs to an execution attempt that is
    /// no longer current for the command.
    #[error("stale attempt {attempt_id} for command {command_id}: another attempt is current")]
    StaleAttempt {
        command_id: String,
        attempt_id: String,
    },

    /// The session still has the expected epoch but no longer has an active lease.
    #[error("session {session_key} is not active: current status is '{status}'")]
    SessionInactive { session_key: String, status: String },

    /// Epoch mismatch — the provided epoch does not match the current session epoch.
    #[error("epoch mismatch: expected {expected}, got {actual}")]
    EpochMismatch { expected: i32, actual: i32 },

    /// The command is not in a valid state for the requested transition.
    #[error(
        "invalid state transition: command {command_id} is in state '{current}', cannot transition to '{target}'"
    )]
    InvalidStateTransition {
        command_id: String,
        current: String,
        target: String,
    },

    /// No available executor found for the given agent.
    #[error("no available executor for agent: {0}")]
    NoAvailableExecutor(String),

    /// Maximum retry attempts exceeded.
    #[error("max attempts exceeded for command: {0}")]
    MaxAttemptsExceeded(String),

    /// A unique-constraint conflict (e.g. duplicate insert).
    #[error("conflict: {0}")]
    Conflict(String),

    /// Database error.
    #[error("database error: {0}")]
    Database(String),

    /// Generic internal error.
    #[error("internal error: {0}")]
    Internal(String),
}

/// Convenience type alias.
pub type SessionResult<T> = Result<T, SessionError>;

impl From<tokio_postgres::Error> for SessionError {
    fn from(e: tokio_postgres::Error) -> Self {
        if let Some(db_err) = e.as_db_error()
            && db_err.code() == &tokio_postgres::error::SqlState::UNIQUE_VIOLATION
        {
            return SessionError::Conflict(db_err.message().to_string());
        }
        let detail = e
            .as_db_error()
            .map(|db| format!("{} ({})", db.message(), db.code().code()))
            .unwrap_or_else(|| e.to_string());
        SessionError::Database(detail)
    }
}
