//! Domain models for the router / policy engine.
//!
//! These types correspond to the `mailbox_visibility`, `route_decisions`, and
//! `conversation_members` tables in the control plane database.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Conversation Member (read model — the router needs membership info)
// ---------------------------------------------------------------------------

/// A conversation member as seen by the router.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMember {
    pub conversation_id: String,
    pub principal_id: String,
    pub principal_type: String,
    pub display_name: Option<String>,
    pub joined_at: DateTime<Utc>,
    pub left_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssigneeRosterEntry {
    pub principal_id: String,
    pub display_name: String,
    pub principal_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_host_name: Option<String>,
}

// ---------------------------------------------------------------------------
// Mailbox Visibility
// ---------------------------------------------------------------------------

/// A record indicating that an agent can "see" a specific message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailboxVisibility {
    pub agent_id: String,
    pub conversation_id: String,
    pub message_id: String,
    pub event_seq: i64,
}

// ---------------------------------------------------------------------------
// Route Decision
// ---------------------------------------------------------------------------

/// Outcome of a routing decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteOutcome {
    Triggered,
    Skipped,
    Suppressed,
}

impl RouteOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Triggered => "trigger",
            Self::Skipped => "skip",
            Self::Suppressed => "suppressed",
        }
    }
}

/// A route decision record for audit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteDecision {
    pub route_id: String,
    pub message_id: String,
    pub agent_id: String,
    pub conversation_id: String,
    pub decision: String,
    pub reason: String,
    pub policy_snapshot: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Agent Policy
// ---------------------------------------------------------------------------

/// The auto-trigger mode for an agent in a conversation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoMode {
    AllMessages,
    #[default]
    MentionedOnly,
    Manual,
}

/// Policy configuration for an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPolicy {
    pub agent_id: String,
    pub conversation_id: String,
    pub auto_mode: AutoMode,
    /// Optional aliases that count as mentions (e.g. "reviewer", "rev").
    #[serde(default)]
    pub mention_aliases: Vec<String>,
}

impl Default for AgentPolicy {
    fn default() -> Self {
        Self {
            agent_id: String::new(),
            conversation_id: String::new(),
            auto_mode: AutoMode::MentionedOnly,
            mention_aliases: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Conversation Routing Policy
// ---------------------------------------------------------------------------

/// How untagged human group messages should be routed at conversation level.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UntaggedHumanMode {
    #[default]
    MentionedOnly,
    CoordinatorOnly,
    AllAgents,
}

impl UntaggedHumanMode {
    pub fn from_db_value(value: &str) -> Self {
        match value {
            "coordinator_only" => Self::CoordinatorOnly,
            "all_agents" => Self::AllAgents,
            _ => Self::MentionedOnly,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MentionedOnly => "mentioned_only",
            Self::CoordinatorOnly => "coordinator_only",
            Self::AllAgents => "all_agents",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationRoutingPolicy {
    pub conversation_id: String,
    pub default_coordinator_agent_id: Option<String>,
    pub untagged_human_mode: UntaggedHumanMode,
}

impl ConversationRoutingPolicy {
    pub fn default_for(conversation_id: &str) -> Self {
        Self {
            conversation_id: conversation_id.to_string(),
            default_coordinator_agent_id: None,
            untagged_human_mode: UntaggedHumanMode::MentionedOnly,
        }
    }
}

// ---------------------------------------------------------------------------
// Workflow Routing State
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRoutingEvent {
    pub kind: String,
    pub task_key: Option<String>,
    pub task_id: Option<String>,
    pub next_role: Option<String>,
    pub target_role: Option<String>,
    #[serde(default)]
    pub target_principal_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelTaskStatus {
    Todo,
    InProgress,
    Blocked,
    InReview,
    Done,
}

impl ChannelTaskStatus {
    pub fn from_db_value(value: &str) -> Option<Self> {
        match value {
            "todo" => Some(Self::Todo),
            "in_progress" => Some(Self::InProgress),
            "blocked" => Some(Self::Blocked),
            "in_review" => Some(Self::InReview),
            "done" => Some(Self::Done),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::InProgress => "in_progress",
            Self::Blocked => "blocked",
            Self::InReview => "in_review",
            Self::Done => "done",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupWorkflowTask {
    pub id: String,
    pub conversation_id: String,
    pub task_key: String,
    pub status: ChannelTaskStatus,
    pub assignee_principal_id: String,
    pub assignee_principal_type: Option<String>,
}

/// Minimal description of an open task assigned to an agent, embedded in the
/// `[choruz-incoming]` envelope so the agent knows which existing cards it
/// already owns (instead of fabricating task keys or creating duplicates).
///
/// Only non-`done` tasks should be surfaced; the field is omitted entirely
/// when the agent has no open assignments in the conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssignedTaskHint {
    pub task_key: String,
    pub title: String,
    pub status: ChannelTaskStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupWorkflowTaskParticipant {
    pub task_id: String,
    pub principal_id: String,
    pub role_key: String,
    pub principal_type: Option<String>,
}

// ---------------------------------------------------------------------------
// Trigger Result
// ---------------------------------------------------------------------------

/// Result of evaluating whether an agent should be triggered.
#[derive(Debug, Clone)]
pub struct TriggerResult {
    pub should_trigger: bool,
    pub reason: String,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_outcome_as_str() {
        assert_eq!(RouteOutcome::Triggered.as_str(), "trigger");
        assert_eq!(RouteOutcome::Skipped.as_str(), "skip");
        assert_eq!(RouteOutcome::Suppressed.as_str(), "suppressed");
    }

    #[test]
    fn route_outcome_serde_roundtrip() {
        let json = serde_json::to_string(&RouteOutcome::Triggered).unwrap();
        assert_eq!(json, "\"triggered\"");
        let parsed: RouteOutcome = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, RouteOutcome::Triggered);
    }

    #[test]
    fn auto_mode_default_is_mentioned_only() {
        assert_eq!(AutoMode::default(), AutoMode::MentionedOnly);
    }

    #[test]
    fn agent_policy_default() {
        let p = AgentPolicy::default();
        assert_eq!(p.auto_mode, AutoMode::MentionedOnly);
        assert!(p.mention_aliases.is_empty());
    }

    #[test]
    fn mailbox_visibility_serde_roundtrip() {
        let mv = MailboxVisibility {
            agent_id: "agent-1".into(),
            conversation_id: "conv-1".into(),
            message_id: "msg-1".into(),
            event_seq: 42,
        };
        let json = serde_json::to_string(&mv).unwrap();
        let parsed: MailboxVisibility = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.event_seq, 42);
    }

    #[test]
    fn channel_task_status_accepts_only_board_values() {
        let valid = [
            ("todo", ChannelTaskStatus::Todo),
            ("in_progress", ChannelTaskStatus::InProgress),
            ("blocked", ChannelTaskStatus::Blocked),
            ("in_review", ChannelTaskStatus::InReview),
            ("done", ChannelTaskStatus::Done),
        ];
        for (raw, expected) in valid {
            assert_eq!(ChannelTaskStatus::from_db_value(raw), Some(expected));
            assert_eq!(expected.as_str(), raw);
        }

        for legacy in ["pending", "needs_human", "needs_approval", "completed"] {
            assert_eq!(ChannelTaskStatus::from_db_value(legacy), None);
        }
    }

    #[test]
    fn route_decision_serde_roundtrip() {
        let rd = RouteDecision {
            route_id: "route-1".into(),
            message_id: "msg-1".into(),
            agent_id: "agent-1".into(),
            conversation_id: "conv-1".into(),
            decision: "trigger".into(),
            reason: "mentioned".into(),
            policy_snapshot: serde_json::json!({"mode": "mentioned_only"}),
        };
        let json = serde_json::to_string(&rd).unwrap();
        let parsed: RouteDecision = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.decision, "trigger");
    }

    #[test]
    fn conversation_member_serde_roundtrip() {
        let member = ConversationMember {
            conversation_id: "conv-1".into(),
            principal_id: "agent-1".into(),
            principal_type: "agent".into(),
            display_name: Some("backend-dev".into()),
            joined_at: chrono::Utc::now(),
            left_at: None,
        };
        let json = serde_json::to_string(&member).unwrap();
        let parsed: ConversationMember = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.principal_type, "agent");
        assert!(parsed.left_at.is_none());
    }

    #[test]
    fn auto_mode_serde_all_variants() {
        for (mode, expected) in [
            (AutoMode::AllMessages, "\"all_messages\""),
            (AutoMode::MentionedOnly, "\"mentioned_only\""),
            (AutoMode::Manual, "\"manual\""),
        ] {
            assert_eq!(serde_json::to_string(&mode).unwrap(), expected);
        }
    }
}
