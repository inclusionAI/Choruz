use choruz_domain::EventEnvelope;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePrincipalRequest {
    pub workspace_id: String,
    pub principal_type: choruz_domain::PrincipalType,
    pub name: String,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateAgentRequest {
    pub actor_id: String,
    pub name: String,
    pub scopes: Vec<String>,
    /// If provided, the agent is created under this workspace instead of the actor's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// Board visibility for human/internal creation paths. Omitted user-facing
    /// creates stay visible; delegated/internal provisioning must set internal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_visibility: Option<choruz_domain::ChannelVisibility>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSecretResponse {
    pub principal: choruz_domain::Principal,
    pub secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotateAgentSecretRequest {
    pub actor_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDirectConversationRequest {
    pub actor_id: String,
    pub peer_principal_id: String,
    /// If provided, the conversation is created under this workspace instead of the actor's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateGroupRequest {
    pub actor_id: String,
    pub name: String,
    pub description: Option<String>,
    pub avatar_url: Option<String>,
    pub member_ids: Vec<String>,
    /// If provided, the group is created under this workspace instead of the actor's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddGroupMembersRequest {
    pub actor_id: String,
    pub member_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateGroupRequest {
    pub actor_id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessageRequest {
    pub actor_id: String,
    pub conversation_id: String,
    pub idempotency_key: String,
    pub content: String,
    pub content_type: String,
    pub metadata: Value,
    /// Optional FE trace correlator. When the gateway receives an
    /// `x-trace-id` header we stash it here so downstream — the message
    /// row metadata, outbox payload, pipeline, writer — can all cite the
    /// same id in their structured logs. `None` when the request is not
    /// part of a traced FE action (e.g. server-to-server).
    #[serde(default)]
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChannelTaskStatus {
    Todo,
    InProgress,
    Blocked,
    InReview,
    Done,
}

impl ChannelTaskStatus {
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

impl std::str::FromStr for ChannelTaskStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "todo" => Ok(Self::Todo),
            "in_progress" => Ok(Self::InProgress),
            "blocked" => Ok(Self::Blocked),
            "in_review" => Ok(Self::InReview),
            "done" => Ok(Self::Done),
            _ => Err(format!("invalid channel task status: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChannelTaskSourceKind {
    Agent,
    Message,
}

impl ChannelTaskSourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Message => "message",
        }
    }
}

impl std::str::FromStr for ChannelTaskSourceKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "agent" => Ok(Self::Agent),
            "message" => Ok(Self::Message),
            _ => Err(format!("invalid channel task source kind: {value}")),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum NullablePatch<T> {
    #[default]
    Unchanged,
    Clear,
    Set(T),
}

impl<T> NullablePatch<T> {
    pub fn is_unchanged(&self) -> bool {
        matches!(self, Self::Unchanged)
    }
}

impl<T> Serialize for NullablePatch<T>
where
    T: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Unchanged | Self::Clear => serializer.serialize_none(),
            Self::Set(value) => value.serialize(serializer),
        }
    }
}

impl<'de, T> Deserialize<'de> for NullablePatch<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<T>::deserialize(deserializer).map(|value| match value {
            Some(value) => Self::Set(value),
            None => Self::Clear,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CreateChannelTaskRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_key: Option<String>,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee_principal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ChannelTaskStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_label: Option<String>,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CreateChannelTaskFromMessageRequest {
    pub message_id: String,
    pub title: String,
    pub assignee_principal_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PatchChannelTaskRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ChannelTaskStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee_principal_id: Option<String>,
    #[serde(default, skip_serializing_if = "NullablePatch::is_unchanged")]
    pub blocked_reason: NullablePatch<String>,
    #[serde(default, skip_serializing_if = "NullablePatch::is_unchanged")]
    pub context_label: NullablePatch<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChannelTaskSnapshot {
    pub task_id: String,
    pub conversation_id: String,
    pub task_key: String,
    pub title: String,
    pub status: ChannelTaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee_principal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee_type: Option<choruz_domain::PrincipalType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_label: Option<String>,
    pub source_kind: ChannelTaskSourceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by_type: Option<choruz_domain::PrincipalType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_by_type: Option<choruz_domain::PrincipalType>,
    pub version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ChannelTaskEventVisibleValues {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ChannelTaskStatus>,
    #[serde(default, skip_serializing_if = "NullablePatch::is_unchanged")]
    pub assignee_principal_id: NullablePatch<String>,
    #[serde(default, skip_serializing_if = "NullablePatch::is_unchanged")]
    pub blocked_reason: NullablePatch<String>,
    #[serde(default, skip_serializing_if = "NullablePatch::is_unchanged")]
    pub context_label: NullablePatch<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_kind: Option<ChannelTaskSourceKind>,
    #[serde(default, skip_serializing_if = "NullablePatch::is_unchanged")]
    pub source_message_id: NullablePatch<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChannelTaskEventProjection {
    pub event_id: String,
    pub task_id: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_principal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_type: Option<choruz_domain::PrincipalType>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resulting_version: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<ChannelTaskEventVisibleValues>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new: Option<ChannelTaskEventVisibleValues>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_effect: Option<ChannelTaskStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChannelTaskDetailResponse {
    pub task: ChannelTaskSnapshot,
    #[serde(default)]
    pub events: Vec<ChannelTaskEventProjection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChannelTaskExport {
    pub task: ChannelTaskSnapshot,
    #[serde(default)]
    pub events: Vec<ChannelTaskEventProjection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChannelTaskCommandFailure {
    pub command_type: String,
    pub ok: bool,
    pub error_code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

impl ChannelTaskCommandFailure {
    pub fn new(
        command_type: impl Into<String>,
        error_code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            command_type: command_type.into(),
            ok: false,
            error_code: error_code.into(),
            message: message.into(),
            task_key: None,
            task_id: None,
        }
    }
}

#[cfg(test)]
mod channel_task_type_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn channel_task_enums_serialize_as_board_contract_values() {
        assert_eq!(
            serde_json::to_value(ChannelTaskStatus::InProgress).unwrap(),
            json!("in_progress")
        );
        assert_eq!(
            serde_json::to_value(ChannelTaskSourceKind::Message).unwrap(),
            json!("message")
        );
        assert_eq!(ChannelTaskStatus::Blocked.as_str(), "blocked");
        assert_eq!(ChannelTaskSourceKind::Agent.as_str(), "agent");
    }

    #[test]
    fn channel_task_create_requests_reject_undefined_public_fields() {
        let with_description = json!({
            "task_key": "TASK-1",
            "title": "Review MVP",
            "assignee_principal_id": "agent-1",
            "idempotency_key": "turn-1",
            "description": "Long form descriptions are not in MVP"
        });
        assert!(serde_json::from_value::<CreateChannelTaskRequest>(with_description).is_err());

        let with_spoofed_source = json!({
            "task_key": "TASK-1",
            "title": "Review MVP",
            "assignee_principal_id": "agent-1",
            "idempotency_key": "turn-1",
            "source_kind": "message"
        });
        assert!(serde_json::from_value::<CreateChannelTaskRequest>(with_spoofed_source).is_err());
    }

    #[test]
    fn channel_task_create_request_accepts_omitted_task_key() {
        let parsed: CreateChannelTaskRequest = serde_json::from_value(json!({
            "title": "Review MVP",
            "idempotency_key": "turn-1"
        }))
        .unwrap();
        assert_eq!(parsed.task_key, None);
        assert_eq!(parsed.title, "Review MVP");

        let with_key: CreateChannelTaskRequest = serde_json::from_value(json!({
            "task_key": "TASK-1",
            "title": "Review MVP",
            "idempotency_key": "turn-1"
        }))
        .unwrap();
        assert_eq!(with_key.task_key, Some("TASK-1".into()));

        let serialized = serde_json::to_value(CreateChannelTaskRequest {
            task_key: None,
            title: "Review MVP".into(),
            assignee_principal_id: None,
            status: None,
            context_label: None,
            idempotency_key: "turn-1".into(),
        })
        .unwrap();
        assert!(serialized.get("task_key").is_none());
    }

    #[test]
    fn patch_channel_task_request_distinguishes_omitted_null_and_set_fields() {
        let omitted: PatchChannelTaskRequest = serde_json::from_value(json!({
            "status": "blocked"
        }))
        .unwrap();
        assert_eq!(omitted.status, Some(ChannelTaskStatus::Blocked));
        assert_eq!(omitted.blocked_reason, NullablePatch::Unchanged);
        assert_eq!(omitted.context_label, NullablePatch::Unchanged);

        let cleared: PatchChannelTaskRequest = serde_json::from_value(json!({
            "blocked_reason": null,
            "context_label": "API contract"
        }))
        .unwrap();
        assert_eq!(cleared.blocked_reason, NullablePatch::Clear);
        assert_eq!(
            cleared.context_label,
            NullablePatch::Set("API contract".to_string())
        );
    }

    #[test]
    fn event_visible_values_preserve_nullable_clears() {
        let values = ChannelTaskEventVisibleValues {
            status: Some(ChannelTaskStatus::Blocked),
            blocked_reason: NullablePatch::Clear,
            context_label: NullablePatch::Set("API contract".to_string()),
            ..ChannelTaskEventVisibleValues::default()
        };

        assert_eq!(
            serde_json::to_value(values).unwrap(),
            json!({
                "status": "blocked",
                "blocked_reason": null,
                "context_label": "API contract"
            })
        );
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupWorkflowTaskParticipant {
    pub task_id: String,
    pub principal_id: String,
    pub role_key: String,
    pub responsibility: Option<String>,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupWorkflowEvent {
    pub id: String,
    pub conversation_id: String,
    pub task_id: Option<String>,
    pub source_message_id: Option<String>,
    pub actor_principal_id: Option<String>,
    pub kind: String,
    pub payload: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resulting_version: Option<i64>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupWorkflowTask {
    pub id: String,
    pub conversation_id: String,
    pub task_key: String,
    pub title: String,
    pub status: String,
    pub assignee_principal_id: String,
    pub blocked_reason: Option<String>,
    pub source_kind: String,
    pub context_label: Option<String>,
    pub idempotency_key: Option<String>,
    pub idempotency_payload_hash: Option<String>,
    pub version: i64,
    pub source_message_id: Option<String>,
    pub created_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub participants: Vec<GroupWorkflowTaskParticipant>,
    #[serde(default)]
    pub events: Vec<GroupWorkflowEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowTaskParticipantInput {
    pub principal_id: String,
    pub role_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub responsibility: Option<String>,
    #[serde(default = "default_required")]
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateGroupWorkflowTaskRequest {
    pub task_key: String,
    pub title: String,
    pub assignee_principal_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_message_id: Option<String>,
    #[serde(default)]
    pub participants: Vec<WorkflowTaskParticipantInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateGroupWorkflowTaskRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee_principal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub participants: Option<Vec<WorkflowTaskParticipantInput>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppendGroupWorkflowEventRequest {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_principal_id: Option<String>,
    #[serde(default = "empty_object")]
    pub payload: Value,
}

fn default_required() -> bool {
    true
}

fn empty_object() -> Value {
    serde_json::json!({})
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListMessagesQuery {
    pub principal_id: String,
    /// Only return messages with `server_seq > since_seq`.
    #[serde(default)]
    pub since_seq: Option<u64>,
    /// Maximum number of messages to return (newest N).
    #[serde(default)]
    pub limit: Option<u64>,
    /// When set to "timeline", threaded replies without broadcast are
    /// filtered out and the response becomes a `TimelineMessages` object
    /// carrying per-root `thread_summaries`. Default (absent / any other
    /// value) keeps today's flat array shape — old clients unaffected.
    ///
    #[serde(default)]
    pub view: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePageQuery {
    pub principal_id: String,
    #[serde(default)]
    pub before_seq: Option<u64>,
    #[serde(default)]
    pub after_seq: Option<u64>,
    #[serde(default)]
    pub limit: Option<u64>,
}

/// A keyset-paginated message page. `next_cursor` points in the requested
/// direction: to an older page for `latest`/`before`, or a newer page for
/// `after`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePage {
    pub messages: Vec<choruz_domain::Message>,
    pub direction: String,
    pub has_more: bool,
    pub next_cursor: Option<u64>,
}

/// Rollup for one thread root on a timeline page: "N replies · last at T".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadSummary {
    pub root_event_id: String,
    pub reply_count: i64,
    pub last_reply_at: DateTime<Utc>,
    /// Up to 5 distinct sender ids in unspecified (implementation:
    /// lexicographic) order — enough for avatar stacks without shipping
    /// the full participant list. NOT recency-ordered; clients must not
    /// rely on any ordering.
    pub participant_sample: Vec<String>,
}

/// Response shape for `GET …/messages?view=timeline`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineMessages {
    pub messages: Vec<choruz_domain::Message>,
    pub thread_summaries: Vec<ThreadSummary>,
}

/// Response shape for `GET …/threads/{root}`: the root message plus its
/// threaded replies in seq order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadDetail {
    pub root: choruz_domain::Message,
    pub replies: Vec<choruz_domain::Message>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListEventsQuery {
    pub cursor: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AckEventsRequest {
    pub upto_delivery_seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetEventWebhookRequest {
    pub actor_id: String,
    pub url: String,
    pub event_types: Vec<String>,
    /// Optional signing secret. Apps can bring their own; if omitted,
    /// the server generates a fresh 32-byte hex secret and returns it
    /// in the response (shown once, like an OAuth install).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportConversationResponse {
    pub conversation: choruz_domain::Conversation,
    pub messages: Vec<choruz_domain::Message>,
    pub audit_logs: Vec<choruz_domain::AuditLog>,
    #[serde(default)]
    pub channel_tasks: Vec<ChannelTaskExport>,
}

// ── Company requests ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateCompanyRequest {
    pub actor_id: String,
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub folder_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCompanyRequest {
    pub actor_id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub avatar_url: Option<String>,
    pub agents_active: Option<bool>,
    pub folder_path: Option<String>,
    pub multi_harness_accounts: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddCompanyMemberRequest {
    pub actor_id: String,
    pub principal_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseStatus {
    pub phase_0_complete: bool,
    pub phase_1_complete: bool,
    pub phase_2_in_progress: bool,
}

/// Unread message count for a single conversation.
///
/// Computed as `conversation.total_msg_count - conversation_member.msg_count`
/// (Mattermost pattern).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationUnread {
    pub conversation_id: String,
    pub unread_count: i64,
    pub mention_count: i64,
    /// Number of threads in this conversation that have replies newer
    /// than the principal's thread_read_receipt (threads never viewed
    /// count too). 0 for conversations without thread activity.
    ///
    #[serde(default)]
    pub thread_unread_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinnedConversation {
    pub conversation_id: String,
    pub pinned_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivedConversation {
    pub conversation_id: String,
    pub archived_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HiddenConversation {
    pub conversation_id: String,
    pub hidden_at: DateTime<Utc>,
}

/// One bounded dashboard-bootstrap row. The database resolves the visible
/// conversation and its latest message in a fixed number of queries; API
/// presentation fields such as unread counts are joined by the handler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationBootstrapEntry {
    pub conversation: choruz_domain::Conversation,
    pub last_message: Option<choruz_domain::Message>,
    pub last_activity_at: DateTime<Utc>,
    pub pinned_at: Option<DateTime<Utc>>,
    pub archived_at: Option<DateTime<Utc>>,
    pub hidden_at: Option<DateTime<Utc>>,
}

/// One durable dashboard mutation. `cursor` is globally monotonic but clients
/// advance it independently; gaps belonging to other principals are normal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncChange {
    pub cursor: u64,
    pub event_type: String,
    pub entity_type: String,
    pub entity_id: String,
    pub conversation_id: Option<String>,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncChangePage {
    pub changes: Vec<SyncChange>,
    pub next_cursor: u64,
    pub head_cursor: u64,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    pub principals_total: usize,
    pub conversations_total: usize,
    pub messages_total: usize,
    pub audit_logs_total: usize,
    pub event_backlog_total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventWebhookConfig {
    pub principal_id: String,
    pub url: String,
    pub event_types: Vec<String>,
    pub cursor: u64,
    pub updated_at: DateTime<Utc>,
    /// HMAC-SHA256 signing secret. Returned to the caller on
    /// `set_event_webhook` (once — just like an OAuth install) so the
    /// external app can verify `X-Choruz-Signature` on incoming
    /// deliveries.
    pub webhook_secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookDelivery {
    pub principal_id: String,
    pub url: String,
    pub event: EventEnvelope,
    /// Shared secret used to sign the outbound HTTP body.
    pub secret: String,
}
