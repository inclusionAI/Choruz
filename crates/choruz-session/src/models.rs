//! Domain models for session management.
//!
//! These types correspond to the `session_registry`, `agent_commands`,
//! `dead_letters`, and executor registry tables defined in
//! `V001__message_pipeline_schema.sql`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

/// Status of a session in the session_registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Idle,
    Active,
    Draining,
    Dead,
}

impl SessionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Active => "active",
            Self::Draining => "draining",
            Self::Dead => "dead",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "idle" => Some(Self::Idle),
            "active" => Some(Self::Active),
            "draining" => Some(Self::Draining),
            "dead" => Some(Self::Dead),
            _ => None,
        }
    }
}

/// A row from `session_registry`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub session_key: String,
    pub agent_id: String,
    pub conversation_id: String,
    pub executor_node_id: Option<String>,
    pub epoch: i32,
    pub status: SessionStatus,
    pub workspace_snapshot_id: Option<String>,
    pub memory_context_id: Option<String>,
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Fields that can be updated on a session.
#[derive(Debug, Clone)]
pub struct SessionUpdate {
    pub session_key: String,
    pub executor_node_id: Option<Option<String>>,
    pub status: Option<SessionStatus>,
    pub last_heartbeat_at: Option<Option<DateTime<Utc>>>,
}

// ---------------------------------------------------------------------------
// Command
// ---------------------------------------------------------------------------

/// Status of an agent command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandStatus {
    #[default]
    Pending,
    Leased,
    Started,
    Heartbeating,
    Succeeded,
    Committed,
    RetryScheduled,
    DeadLetter,
}

impl CommandStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Leased => "leased",
            Self::Started => "started",
            Self::Heartbeating => "heartbeating",
            Self::Succeeded => "succeeded",
            Self::Committed => "committed",
            Self::RetryScheduled => "retry_scheduled",
            Self::DeadLetter => "dead_letter",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "leased" => Some(Self::Leased),
            "started" => Some(Self::Started),
            "heartbeating" => Some(Self::Heartbeating),
            "succeeded" => Some(Self::Succeeded),
            "committed" => Some(Self::Committed),
            "retry_scheduled" => Some(Self::RetryScheduled),
            "dead_letter" => Some(Self::DeadLetter),
            _ => None,
        }
    }

    /// Returns `true` if the command is in an "active" state (leased, started,
    /// heartbeating).
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Leased | Self::Started | Self::Heartbeating)
    }

    /// Returns `true` if the command is in a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Succeeded | Self::Committed | Self::DeadLetter)
    }
}

/// A row from `agent_commands`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCommand {
    pub command_id: String,
    pub route_id: String,
    pub session_key: String,
    pub agent_id: String,
    pub conversation_id: String,
    pub message_id: String,
    pub turn_id: String,
    pub status: CommandStatus,
    pub current_attempt_id: Option<String>,
    pub current_epoch: Option<i32>,
    pub attempt_count: i32,
    pub max_attempts: i32,
    pub prompt: String,
    pub metadata: serde_json::Value,
    pub next_retry_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Runtime status projection for an active command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeStatusCommand {
    pub command_id: String,
    pub message_id: String,
    pub turn_id: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub lease_age_seconds: i64,
    pub attempt_count: i32,
    pub last_error: Option<String>,
}

/// Derived runtime status for an agent in a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRuntimeStatus {
    pub conversation_id: String,
    pub agent_id: String,
    pub status: String,
    pub active_command: Option<RuntimeStatusCommand>,
    pub queued_count: i64,
    pub last_error: Option<String>,
}

/// Input for inserting a new command.
///
/// # ID type mapping (see `choruz_ids`)
///
/// | Field          | choruz-ids type              |
/// |----------------|-----------------------------|
/// | `command_id`   | [`choruz_ids::CommandId`]    |
/// | `route_id`     | [`choruz_ids::RouteId`]      |
/// | `message_id`   | [`choruz_ids::MessageId`]    |
/// | `turn_id`      | [`choruz_ids::TurnId`]       |
#[derive(Debug, Clone)]
pub struct InsertCommand {
    pub command_id: String,
    pub route_id: String,
    pub session_key: String,
    pub agent_id: String,
    pub conversation_id: String,
    pub message_id: String,
    pub turn_id: String,
    pub prompt: String,
    pub max_attempts: i32,
    pub metadata: serde_json::Value,
}

impl InsertCommand {
    /// Typed constructor using `choruz_ids` newtypes.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        command_id: choruz_ids::CommandId,
        route_id: choruz_ids::RouteId,
        session_key: String,
        agent_id: String,
        conversation_id: String,
        message_id: String,
        turn_id: choruz_ids::TurnId,
        prompt: String,
        max_attempts: i32,
        metadata: serde_json::Value,
    ) -> Self {
        Self {
            command_id: command_id.to_string(),
            route_id: route_id.to_string(),
            session_key,
            agent_id,
            conversation_id,
            message_id,
            turn_id: turn_id.to_string(),
            prompt,
            max_attempts,
            metadata,
        }
    }
}

/// Fields to update on a command status transition.
#[derive(Debug, Clone, Default)]
pub struct CommandStatusUpdate {
    pub command_id: String,
    pub status: CommandStatus,
    pub current_attempt_id: Option<String>,
    pub current_epoch: Option<i32>,
    pub attempt_count: Option<i32>,
    pub next_retry_at: Option<Option<DateTime<Utc>>>,
    pub last_error: Option<String>,
}

// ---------------------------------------------------------------------------
// Lease
// ---------------------------------------------------------------------------

/// Result of a lease assignment.
#[derive(Debug, Clone)]
pub struct LeaseAssignment {
    pub epoch: i32,
    pub attempt_id: String,
    /// Attempt number after the lease assignment increments the command.
    pub attempt_count: i32,
}

/// An expired lease detected by the heartbeat monitor.
///
/// Attempt counts are detection-time logging snapshots. Expiry handling
/// rereads both values from the locked command row before making decisions.
#[derive(Debug, Clone)]
pub struct ExpiredLease {
    pub session_key: String,
    pub command_id: String,
    pub epoch: i32,
    pub attempt_count: i32,
    pub max_attempts: i32,
}

// ---------------------------------------------------------------------------
// Dead Letter
// ---------------------------------------------------------------------------

/// A row from `dead_letters`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadLetter {
    pub id: String,
    pub source_type: String,
    pub source_id: String,
    pub payload: serde_json::Value,
    pub error: String,
    pub attempt_count: i32,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub resolved_by: Option<String>,
}

/// Input for inserting a dead letter.
#[derive(Debug, Clone)]
pub struct InsertDeadLetter {
    pub source_type: String,
    pub source_id: String,
    pub payload: serde_json::Value,
    pub error: String,
    pub attempt_count: i32,
}

// ---------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------

/// An executor node registered in the executor registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Executor {
    pub node_id: String,
    pub capabilities: serde_json::Value,
    pub status: ExecutorStatus,
    pub last_heartbeat_at: DateTime<Utc>,
    pub registered_at: DateTime<Utc>,
}

/// Executor status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorStatus {
    Available,
    Busy,
    Draining,
    Offline,
}

impl ExecutorStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Busy => "busy",
            Self::Draining => "draining",
            Self::Offline => "offline",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "available" => Some(Self::Available),
            "busy" => Some(Self::Busy),
            "draining" => Some(Self::Draining),
            "offline" => Some(Self::Offline),
            _ => None,
        }
    }
}
