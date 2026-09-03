// ---------------------------------------------------------------------------
// Shared type definitions for Choruz web components.
// These are mirrored from choruz-api.ts to avoid importing Node-only code
// into client components. Keep in sync with the API types.
// ---------------------------------------------------------------------------

export type Principal = {
  id: string;
  workspace_id: string;
  principal_type: "human" | "agent";
  name: string;
  avatar_url: string | null;
  scopes: string[];
  disabled: boolean;
  channel_visibility?: "visible" | "internal";
  created_at: string;
  updated_at: string;
};

export type ConversationMember = {
  principal_id: string;
  joined_at: string;
};

export type Conversation = {
  id: string;
  workspace_id: string;
  conversation_type: "direct" | "group";
  name: string | null;
  description: string | null;
  avatar_url: string | null;
  creator_id: string;
  created_at: string;
  updated_at: string;
  members: Record<string, ConversationMember>;
};

export type ChatMessage = {
  id: string;
  workspace_id: string;
  conversation_id: string;
  sender_id: string;
  content: string;
  content_type: string;
  metadata: Record<string, unknown>;
  edited_at: string | null;
  edited_by: string | null;
  server_seq: number;
  idempotency_key: string;
  created_at: string;
};

export type MessagePage = {
  messages: ChatMessage[];
  direction: "latest" | "before" | "after";
  has_more: boolean;
  next_cursor: number | null;
};

export type PinnedConversation = {
  conversation_id: string;
  pinned_at: string;
};

export type ArchivedConversation = {
  conversation_id: string;
  archived_at: string;
};

export type HiddenConversation = {
  conversation_id: string;
  hidden_at: string;
};

export type HostPluginManifest = {
  id: string;
  version: string;
  host_capabilities: string[];
  client_capabilities: string[];
};

/** Root message + its threaded replies, from
 * `GET /v1/conversations/{id}/threads/{root}`. */
export type ThreadDetail = {
  root: ChatMessage;
  replies: ChatMessage[];
};

export type ConsoleSnapshot = {
  principal: Principal;
  principals?: Principal[];
  conversations: Conversation[];
  messages_by_conversation: Record<string, ChatMessage[]>;
  agents: Principal[];
  audit_logs: unknown[];
  plugins?: HostPluginManifest[];
  pinned_conversations?: PinnedConversation[];
  archived_conversations?: ArchivedConversation[];
  hidden_conversations?: HiddenConversation[];
  /**
   * Per-conversation unread + mention counts (Mattermost pattern). The
   * Legacy operational snapshot unread rows. The product dashboard now gets
   * the same counters from its bounded bootstrap.
   */
  unreads?: Array<{
    conversation_id: string;
    unread_count: number;
    mention_count: number;
    /** COUNT(DISTINCT thread) with replies the caller hasn't read —
     * counts THREADS, not individual replies. */
    thread_unread_count?: number;
  }>;
};

export type DashboardSyncChange = {
  cursor: number;
  event_type: string;
  entity_type: string;
  entity_id: string;
  conversation_id: string | null;
  payload: Record<string, unknown>;
  created_at: string;
};

export type DashboardBootstrapItem = {
  conversation: Conversation;
  last_message: ChatMessage | null;
  last_activity_at: string;
  unread_count: number;
  mention_count: number;
  thread_unread_count: number;
  pinned_at: string | null;
  archived_at: string | null;
  hidden_at: string | null;
};

export type DashboardBootstrap = {
  principal: Principal;
  principals: Principal[];
  companies: Company[];
  agents: Principal[];
  conversations: {
    items: DashboardBootstrapItem[];
    next_cursor: string | null;
    has_more: boolean;
  };
  plugins: HostPluginManifest[];
  hidden_conversations: HiddenConversation[];
  /** Every runtime binding the person may see, taken under `sync_cursor`. */
  runtime_bindings: RuntimeBindingInfo[];
  sync_cursor: number;
  snapshot_at: string;
};

export type SearchResultItem = {
  message_id: string;
  conversation_id: string;
  conversation_name: string | null;
  sender_id: string;
  content: string;
  created_at: string;
};

export type RuntimeBindingInfo = {
  id: string;
  workspace_id?: string;
  conversation_id: string;
  conversation_type?: "direct" | "group";
  agent_principal_id: string;
  driver_type: string;
  interaction_mode?: "message" | "terminal" | null;
  runtime_host_id?: string | null;
  harness_account_id?: string | null;
  harness_account_name?: string | null;
  workspace_path: string;
  state: string;
};

export type AgentTask = {
  id: string;
  subject: string;
  status: string;
  description?: string;
  owner?: string;
};

export type AgentTasksResponse = {
  agent_id: string;
  driver_type: string;
  tasks: AgentTask[];
};

export type RuntimeStatusCommand = {
  command_id: string;
  message_id: string;
  turn_id: string;
  status:
    | "pending"
    | "leased"
    | "started"
    | "heartbeating"
    | "succeeded"
    | "committed"
    | "retry_scheduled"
    | "dead_letter";
  created_at: string;
  updated_at: string;
  lease_age_seconds: number | null;
  attempt_count: number;
  last_error: string | null;
};

export type ConversationRuntimeStatus = {
  conversation_id: string;
  agent_principal_id: string;
  agent_name: string;
  status: "idle" | "queued" | "busy";
  queued_count: number;
  active_command: RuntimeStatusCommand | null;
  last_error: string | null;
};

export type ChannelTaskStatus = "todo" | "in_progress" | "blocked" | "in_review" | "done";

export type ChannelTaskSourceKind = "agent" | "message";

export type ChannelTask = {
  task_id: string;
  conversation_id: string;
  task_key: string;
  title: string;
  status: ChannelTaskStatus;
  assignee_principal_id?: string;
  assignee_type?: "human" | "agent";
  assignee_name?: string;
  blocked_reason?: string;
  context_label?: string;
  source_kind: ChannelTaskSourceKind;
  source_message_id?: string;
  created_by?: string;
  created_by_type?: "human" | "agent";
  updated_by?: string;
  updated_by_type?: "human" | "agent";
  version: number;
  created_at: string;
  updated_at: string;
};

export type ChannelTaskVisibleValues = {
  status?: ChannelTaskStatus;
  assignee_principal_id?: string | null;
  blocked_reason?: string | null;
  context_label?: string | null;
  source_kind?: ChannelTaskSourceKind;
  source_message_id?: string | null;
};

export type ChannelTaskEvent = {
  event_id: string;
  task_id: string;
  kind: string;
  actor_principal_id?: string;
  actor_type?: "human" | "agent";
  created_at: string;
  resulting_version?: number;
  previous?: ChannelTaskVisibleValues;
  new?: ChannelTaskVisibleValues;
  workflow_kind?: string;
  status_effect?: ChannelTaskStatus;
  reason_code?: string;
};

export type ChannelTaskDetail = {
  task: ChannelTask;
  events: ChannelTaskEvent[];
};

export type CreateChannelTaskFromMessageRequest = {
  message_id: string;
  title: string;
  assignee_principal_id: string;
  context_label?: string | null;
  idempotency_key?: string;
};

export type PatchChannelTaskRequest = {
  status?: ChannelTaskStatus;
  assignee_principal_id?: string;
  blocked_reason?: string | null;
  context_label?: string | null;
};

export type Company = {
  id: string;
  name: string;
  slug: string;
  description: string | null;
  avatar_url: string | null;
  owner_id: string;
  agents_active?: boolean;
  folder_path?: string | null;
  /** Whether Create Agent chooses among harness accounts; off means the device's own login. */
  multi_harness_accounts?: boolean;
  archived_at: string | null;
  deleted_at: string | null;
  created_at: string;
  updated_at: string;
};

export type CronJob = {
  id: string;
  agent_id: string;
  conversation_id: string;
  name: string;
  description?: string;
  schedule_type: string;
  schedule_value: string;
  schedule_timezone?: string;
  message: string;
  session_target: string;
  delivery_mode: string;
  enabled: boolean;
  last_run_at: string | null;
  next_run_at: string | null;
  last_status: string | null;
  last_error: string | null;
  consecutive_errors: number;
  timeout_seconds: number;
  delete_after_run: boolean;
  created_at: string;
  updated_at: string;
};

/** One row of a `/api/filesystem?action=list` response. */
export type DirEntry = {
  name: string;
  type: string;
  path: string;
};
