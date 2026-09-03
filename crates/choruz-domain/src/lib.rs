use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalType {
    Human,
    Agent,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChannelVisibility {
    #[default]
    Visible,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationType {
    Direct,
    Group,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Principal {
    pub id: String,
    pub workspace_id: String,
    pub principal_type: PrincipalType,
    pub name: String,
    pub avatar_url: Option<String>,
    pub scopes: Vec<String>,
    pub secret_hash: Option<String>,
    pub disabled: bool,
    pub deleted_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub channel_visibility: ChannelVisibility,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Links human principals across companies. None for agents.
    #[serde(default)]
    pub user_id: Option<String>,
}

// ── Company (organization / workspace) ──────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Company {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub avatar_url: Option<String>,
    pub owner_id: String,
    /// When true, the system will keep all agents in this company alive
    /// by automatically re-creating PTY sessions after restarts.
    #[serde(default)]
    pub agents_active: bool,
    /// Filesystem path to the company's workspace folder (VS Code-style "Open Folder").
    /// When set, agents in this company work inside this directory and the sidebar
    /// shows a file explorer tree for it.
    #[serde(default)]
    pub folder_path: Option<String>,
    /// When true, the Harness Accounts dialog adds sign-ins beyond the login
    /// each device already has and Create Agent chooses among them.
    #[serde(default)]
    pub multi_harness_accounts: bool,
    /// When set, the company is archived (read-only, greyed out in UI).
    #[serde(default)]
    pub archived_at: Option<DateTime<Utc>>,
    /// When set, the company is soft-deleted (hidden from list, 30-day recovery).
    #[serde(default)]
    pub deleted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompanyMember {
    pub principal_id: String,
    pub joined_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct User {
    pub id: String,
    pub email: Option<String>,
    pub display_name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConversationMember {
    pub principal_id: String,
    pub joined_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Conversation {
    pub id: String,
    pub workspace_id: String,
    pub conversation_type: ConversationType,
    pub name: Option<String>,
    pub description: Option<String>,
    pub avatar_url: Option<String>,
    pub creator_id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub members: BTreeMap<String, ConversationMember>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub id: String,
    pub workspace_id: String,
    pub conversation_id: String,
    pub sender_id: String,
    pub content: String,
    pub content_type: String,
    pub metadata: Value,
    pub edited_at: Option<DateTime<Utc>>,
    pub edited_by: Option<String>,
    pub server_seq: u64,
    pub idempotency_key: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReadReceipt {
    pub principal_id: String,
    pub conversation_id: String,
    pub last_read_message_id: String,
    pub last_read_seq: u64,
    pub last_read_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditLog {
    pub id: String,
    pub workspace_id: String,
    pub actor_id: String,
    pub action: String,
    pub target_type: String,
    pub target_id: String,
    pub metadata: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EventEnvelope {
    pub delivery_seq: u64,
    pub event_id: String,
    pub principal_id: String,
    pub event_type: String,
    pub payload: Value,
    pub created_at: DateTime<Utc>,
}
