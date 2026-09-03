import { currentTraceId } from "./choruz-trace";
import type { DashboardBootstrap, ThreadDetail } from "./choruz-types";
import { transportFetch } from "./transport";

const SESSION_COOKIE_NAME = "choruz_session";

export class ApiRequestError extends Error {
  constructor(
    public readonly status: number,
    detail: string,
  ) {
    super(detail);
    this.name = "ApiRequestError";
  }
}

export type SessionClaims = {
  principal_id: string;
  workspace_id: string;
  display_name: string;
  expires_at_epoch_s: number;
};

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

export type AuditLog = {
  id: string;
  workspace_id: string;
  actor_id: string;
  action: string;
  target_type: string;
  target_id: string;
  metadata: Record<string, unknown>;
  created_at: string;
};

export type AttachmentRecord = {
  id: string;
  workspace_id: string;
  owner_id: string;
  filename: string;
  content_type: string;
  size_bytes: number;
  download_path: string;
  created_at: string;
};

export type ConsoleSnapshot = {
  principal: Principal;
  principals?: Principal[];
  conversations: Conversation[];
  messages_by_conversation: Record<string, ChatMessage[]>;
  agents: Principal[];
  audit_logs: AuditLog[];
  plugins?: Array<{
    id: string;
    version: string;
    host_capabilities: string[];
    client_capabilities: string[];
  }>;
};

export type RuntimeBinding = {
  id: string;
  workspace_id: string;
  conversation_id: string;
  conversation_name: string;
  conversation_type: "direct" | "group";
  agent_principal_id: string;
  agent_name: string;
  driver_type:
    | "claude_print"
    | "claude_terminal"
    | "codex_exec"
    | "codex_app_server"
    | "codex_terminal"
    | "pi_terminal"
    | "grok_terminal"
    | "opencode_terminal"
    | "acp"
    | "webhook_agent";
  interaction_mode?: "message" | "terminal" | null;
  runtime_host_id?: string | null;
  harness_account_id?: string | null;
  harness_account_name?: string | null;
  workspace_path: string;
  git_worktree_path: string | null;
  external_session_id: string | null;
  external_thread_id: string | null;
  last_event_cursor: number;
  last_acked_event_cursor: number;
  last_seen_server_seq: number;
  state: "idle" | "running" | "paused" | "disabled" | "error";
  last_error: string | null;
  updated_at: string;
};

export type HarnessKind = "claude" | "codex" | "pi" | "grok" | "open_code";

export type NativeSessionSummary = {
  harness: HarnessKind;
  native_session_id: string;
  title: string;
  workspace_path: string;
  updated_at: string;
  model: string | null;
  branch: string | null;
  archived: boolean;
};

export type WorkspaceSessionScanResult = {
  workspace_path: string;
  sessions: NativeSessionSummary[];
  warnings: string[];
};

export type ImportedWorkspaceSession = {
  harness: HarnessKind;
  native_session_id: string;
  agent_principal_id: string;
  conversation_id: string;
  binding_id: string;
  agent_name: string;
  already_imported: boolean;
};

export type RuntimePolicy = {
  conversation_id: string;
  auto_mode: "disabled" | "mentioned_only" | "metadata_only";
  max_auto_turns: number;
  max_workflow_turns: number;
  require_human_after_n_turns: number;
  allow_agent_to_agent: boolean;
  allow_file_write: boolean;
  default_reviewer_agent_id: string | null;
  default_coordinator_agent_id: string | null;
  untagged_human_mode: "mentioned_only" | "coordinator_only" | "all_agents";
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
  folder_path: string | null;
  archived_at: string | null;
  deleted_at: string | null;
  created_at: string;
  updated_at: string;
};

export type CompanyMember = {
  principal_id: string;
  joined_at: string;
};

export type LocalLoginResponse = {
  principal: Principal;
  session_token: string;
};

export function sessionCookieName(): string {
  return SESSION_COOKIE_NAME;
}

export function apiBaseUrl(): string {
  // Browser: go through Next.js rewrites so everything is same-origin and
  // we don't need CORS on the gateway. `next.config.ts` maps
  // `/api/v1/:path*` → `${gateway}/v1/:path*`, so every client-side request
  // goes through `/api`.
  //
  // Calling `http://127.0.0.1:3000` directly from a browser on :3100 used
  // to fail with "Failed to fetch" (visible in e.g. Remote Servers modal)
  // because the gateway has no CORS middleware and the browser blocks the
  // cross-origin response.
  if (typeof window !== "undefined") {
    return "/api";
  }
  // Server-side (RSC / route handlers / scripts): hit the gateway directly.
  return (
    process.env.CHORUZ_API_BASE_URL?.trim()
    || process.env.CHORUZ_API_URL?.trim()
    || `http://127.0.0.1:${process.env.CHORUZ_API_PORT ?? "3000"}`
  );
}

export function decodeSessionClaims(token: string): SessionClaims | null {
  const [encodedPayload] = token.split(".", 1);
  if (!encodedPayload) {
    return null;
  }

  try {
    const payload = encodedPayload.replace(/-/g, "+").replace(/_/g, "/");
    const padded = payload + "=".repeat((4 - (payload.length % 4 || 4)) % 4);
    const json = Buffer.from(padded, "base64").toString("utf8");
    return JSON.parse(json) as SessionClaims;
  } catch {
    return null;
  }
}

export async function localLogin(
  username: string,
  password: string,
): Promise<LocalLoginResponse> {
  return apiJson<LocalLoginResponse>("/v1/auth/local/login", {
    method: "POST",
    body: JSON.stringify({ username, password }),
  });
}

export async function localSignup(
  username: string,
  password: string,
): Promise<LocalLoginResponse> {
  return apiJson<LocalLoginResponse>("/v1/auth/local/signup", {
    method: "POST",
    body: JSON.stringify({ username, password }),
  });
}

export async function fetchConsoleSnapshot(sessionToken: string): Promise<ConsoleSnapshot> {
  return apiJson<ConsoleSnapshot>("/v1/console", {}, sessionToken);
}

export async function fetchDashboardBootstrap(
  sessionToken: string,
  options: { limit?: number; after?: string } = {},
): Promise<DashboardBootstrap> {
  const query = new URLSearchParams();
  if (options.limit) query.set("limit", String(options.limit));
  if (options.after) query.set("after", options.after);
  const suffix = query.size > 0 ? `?${query}` : "";
  return apiJson<DashboardBootstrap>(`/v1/bootstrap${suffix}`, {}, sessionToken);
}

// ── Remote control plugin API ──

export async function fetchRemoteControlSettings(sessionToken: string) {
  return apiJson<import("../remote/remote-control").RemoteControlSettings>(
    "/v1/remote-control/settings", {}, sessionToken,
  );
}

export async function createRemoteControlPairing(sessionToken: string) {
  return apiJson<import("../remote/remote-control").RemoteControlPairing>(
    "/v1/remote-control/pairings", { method: "POST" }, sessionToken,
  );
}

export async function listRemoteControlDevices(sessionToken: string) {
  return apiJson<import("../remote/remote-control").RemoteControlDevice[]>(
    "/v1/remote-control/devices", {}, sessionToken,
  );
}

export async function revokeRemoteControlDevice(sessionToken: string, deviceId: string) {
  await apiJson(
    `/v1/remote-control/devices/${encodeURIComponent(deviceId)}`,
    { method: "DELETE" },
    sessionToken,
  );
}

export async function markRemoteControlDeviceSeen(sessionToken: string, deviceId: string) {
  await apiJson(
    `/v1/remote-control/devices/${encodeURIComponent(deviceId)}/seen`,
    { method: "PUT" },
    sessionToken,
  );
}

export async function listRuntimeHosts(sessionToken: string, companyId: string) {
  return apiJson<import("../remote/remote-control").RuntimeHost[]>(
    `/v1/companies/${encodeURIComponent(companyId)}/runtime-hosts`, {}, sessionToken,
  );
}

export async function createRuntimeHostPairing(sessionToken: string, companyId: string) {
  return apiJson<import("../remote/remote-control").RuntimeHostPairing>(
    `/v1/companies/${encodeURIComponent(companyId)}/runtime-host-pairings`,
    { method: "POST" },
    sessionToken,
  );
}

export async function renameRuntimeHost(
  sessionToken: string,
  hostId: string,
  name: string,
) {
  return apiJson<import("../remote/remote-control").RuntimeHost>(
    `/v1/runtime-hosts/${encodeURIComponent(hostId)}`,
    { method: "PUT", body: JSON.stringify({ name }) },
    sessionToken,
  );
}

export async function revokeRuntimeHost(sessionToken: string, hostId: string) {
  await apiEmpty(
    `/v1/runtime-hosts/${encodeURIComponent(hostId)}`,
    { method: "DELETE" },
    sessionToken,
  );
}

// ── Company API ──

export async function fetchCompanies(sessionToken: string): Promise<Company[]> {
  return apiJson<Company[]>("/v1/companies", {}, sessionToken);
}

export async function createCompany(
  sessionToken: string,
  payload: { actor_id: string; name: string; slug?: string; description?: string; folder_path?: string },
): Promise<Company> {
  return apiJson<Company>("/v1/companies", {
    method: "POST",
    body: JSON.stringify(payload),
  }, sessionToken);
}

export async function fetchCompany(sessionToken: string, companyId: string): Promise<Company> {
  return apiJson<Company>(`/v1/companies/${companyId}`, {}, sessionToken);
}

export async function updateCompany(
  sessionToken: string,
  companyId: string,
  payload: { actor_id: string; name?: string; description?: string },
): Promise<Company> {
  return apiJson<Company>(`/v1/companies/${companyId}`, {
    method: "PATCH",
    body: JSON.stringify(payload),
  }, sessionToken);
}

export async function deleteCompany(sessionToken: string, companyId: string): Promise<void> {
  await apiJson(`/v1/companies/${companyId}`, { method: "DELETE" }, sessionToken);
}

export async function fetchCompanyMembers(sessionToken: string, companyId: string): Promise<CompanyMember[]> {
  return apiJson<CompanyMember[]>(`/v1/companies/${companyId}/members`, {}, sessionToken);
}

export async function addCompanyMember(
  sessionToken: string,
  companyId: string,
  payload: { actor_id: string; principal_id: string },
): Promise<CompanyMember> {
  return apiJson<CompanyMember>(`/v1/companies/${companyId}/members`, {
    method: "POST",
    body: JSON.stringify(payload),
  }, sessionToken);
}

export async function fetchRuntimeBindings(sessionToken: string): Promise<RuntimeBinding[]> {
  return apiJson<RuntimeBinding[]>("/v1/runtime/bindings", {}, sessionToken);
}

export async function fetchRuntimeBinding(
  sessionToken: string,
  bindingId: string,
): Promise<RuntimeBinding> {
  return apiJson<RuntimeBinding>(`/v1/runtime/bindings/${bindingId}`, {}, sessionToken);
}

export async function rebindRuntimeBinding(
  sessionToken: string,
  bindingId: string,
  workspacePath: string,
): Promise<RuntimeBinding> {
  return apiJson<RuntimeBinding>(
    `/v1/runtime/bindings/${bindingId}/rebind`,
    {
      method: "POST",
      body: JSON.stringify({
        workspace_path: workspacePath,
      }),
    },
    sessionToken,
  );
}

export async function createDirectConversation(
  sessionToken: string,
  actorId: string,
  peerPrincipalId: string,
  workspaceId?: string,
): Promise<Conversation> {
  return apiJson<Conversation>(
    "/v1/conversations/direct",
    {
      method: "POST",
      body: JSON.stringify({
        actor_id: actorId,
        peer_principal_id: peerPrincipalId,
        ...(workspaceId ? { workspace_id: workspaceId } : {}),
      }),
    },
    sessionToken,
  );
}

export async function createGroup(
  sessionToken: string,
  actorId: string,
  name: string,
  memberIds: string[],
  workspaceId?: string,
): Promise<Conversation> {
  return apiJson<Conversation>(
    "/v1/groups",
    {
      method: "POST",
      body: JSON.stringify({
        actor_id: actorId,
        name,
        description: null,
        avatar_url: null,
        member_ids: memberIds,
        ...(workspaceId ? { workspace_id: workspaceId } : {}),
      }),
    },
    sessionToken,
  );
}

export async function addGroupMembers(
  sessionToken: string,
  actorId: string,
  conversationId: string,
  memberIds: string[],
): Promise<Conversation> {
  return apiJson<Conversation>(
    `/v1/groups/${conversationId}/members`,
    {
      method: "POST",
      body: JSON.stringify({
        actor_id: actorId,
        member_ids: memberIds,
      }),
    },
    sessionToken,
  );
}

export async function removeGroupMember(
  sessionToken: string,
  actorId: string,
  conversationId: string,
  principalId: string,
): Promise<Conversation> {
  return apiJson<Conversation>(
    `/v1/groups/${conversationId}/members/${principalId}?actor_id=${encodeURIComponent(actorId)}`,
    {
      method: "DELETE",
    },
    sessionToken,
  );
}

export async function sendMessage(
  sessionToken: string,
  actorId: string,
  conversationId: string,
  content: string,
  metadata: Record<string, unknown> = {},
  contentType = "text",
  idempotencyKey?: string,
): Promise<ChatMessage> {
  return apiJson<ChatMessage>(
    "/v1/messages",
    {
      method: "POST",
      body: JSON.stringify({
        actor_id: actorId,
        conversation_id: conversationId,
        idempotency_key: idempotencyKey ?? `${Date.now()}-${Math.random().toString(16).slice(2)}`,
        content,
        content_type: contentType,
        metadata,
      }),
    },
    sessionToken,
  );
}

// Root message + threaded replies. Single declaration in choruz-types.ts —
// re-exported (not mirrored) so the API client and components cannot drift.
export type { ThreadDetail } from "./choruz-types";

/** Fetch ONE message (quote-reply preview for targets outside the loaded
 * history window). 404 ⇒ deleted or never existed in this conversation. */
export async function fetchConversationMessage(
  sessionToken: string,
  conversationId: string,
  messageId: string,
): Promise<ChatMessage> {
  return apiJson<ChatMessage>(
    `/v1/conversations/${encodeURIComponent(conversationId)}/messages/${encodeURIComponent(messageId)}`,
    {},
    sessionToken,
  );
}

export async function fetchThread(
  sessionToken: string,
  conversationId: string,
  threadRootId: string,
): Promise<ThreadDetail> {
  return apiJson<ThreadDetail>(
    `/v1/conversations/${encodeURIComponent(conversationId)}/threads/${encodeURIComponent(threadRootId)}`,
    {},
    sessionToken,
  );
}

/** Upsert the caller's thread read receipt (clears the thread unread dot). */
export async function markThreadViewed(
  sessionToken: string,
  conversationId: string,
  threadRootId: string,
): Promise<void> {
  const headers = new Headers();
  headers.set("authorization", `Bearer ${sessionToken}`);
  const response = await fetch(
    `${apiBaseUrl()}/v1/conversations/${encodeURIComponent(conversationId)}/threads/${encodeURIComponent(threadRootId)}/view`,
    { method: "POST", headers, cache: "no-store" },
  );
  if (!response.ok) {
    let detail = `${response.status} ${response.statusText}`;
    try {
      const payload = (await response.json()) as { error?: { detail?: string } | string };
      if (typeof payload.error === "string") detail = payload.error;
      else if (payload.error && typeof payload.error.detail === "string") detail = payload.error.detail;
    } catch {}
    throw new Error(detail);
  }
}

export async function createAgent(
  sessionToken: string,
  actorId: string,
  name: string,
  scopes: string[],
  workspaceId?: string,
  channelVisibility?: "visible" | "internal",
): Promise<{ principal: Principal; secret: string }> {
  return apiJson<{ principal: Principal; secret: string }>(
    "/v1/agents",
    {
      method: "POST",
      body: JSON.stringify({
        actor_id: actorId,
        name,
        scopes,
        ...(workspaceId ? { workspace_id: workspaceId } : {}),
        ...(channelVisibility ? { channel_visibility: channelVisibility } : {}),
      }),
    },
    sessionToken,
  );
}

export async function rotateAgentSecret(
  sessionToken: string,
  actorId: string,
  agentId: string,
): Promise<{ principal: Principal; secret: string }> {
  return apiJson<{ principal: Principal; secret: string }>(
    `/v1/agents/${agentId}/rotate-secret`,
    {
      method: "POST",
      body: JSON.stringify({
        actor_id: actorId,
      }),
    },
    sessionToken,
  );
}

export type BatchDisableAgentsResponse = {
  disabled: number;
  failed: number;
  conversations_deleted: number;
  conversations_failed: number;
};

export async function batchDisableAgents(
  sessionToken: string,
  actorId: string,
  agentIds: string[],
): Promise<BatchDisableAgentsResponse> {
  return apiJson<BatchDisableAgentsResponse>(
    "/v1/agents/batch-disable",
    {
      method: "POST",
      body: JSON.stringify({
        actor_id: actorId,
        agent_ids: agentIds,
        conversation_ids: [],
      }),
    },
    sessionToken,
  );
}

export async function uploadAttachment(
  sessionToken: string,
  actorId: string,
  file: File,
): Promise<AttachmentRecord> {
  const arrayBuffer = await file.arrayBuffer();
  return apiJson<AttachmentRecord>(
    "/v1/attachments",
    {
      method: "POST",
      body: JSON.stringify({
        actor_id: actorId,
        filename: file.name,
        content_type: file.type || "application/octet-stream",
        data_base64: Buffer.from(arrayBuffer).toString("base64"),
      }),
    },
    sessionToken,
  );
}

export async function createRuntimeBinding(
  sessionToken: string,
  actorId: string,
  conversationId: string,
  agentPrincipalId: string,
  driverType: string,
  workspacePath: string,
  options?: {
    configJson?: Record<string, unknown>;
    gitWorktreePath?: string | null;
  },
): Promise<RuntimeBinding> {
  return apiJson<RuntimeBinding>(
    "/v1/runtime/bindings",
    {
      method: "POST",
      body: JSON.stringify({
        actor_id: actorId,
        conversation_id: conversationId,
        agent_principal_id: agentPrincipalId,
        driver_type: driverType,
        workspace_path: workspacePath,
        git_worktree_path: options?.gitWorktreePath ?? null,
        config_json: options?.configJson ?? {},
      }),
    },
    sessionToken,
  );
}

export async function scanWorkspaceSessions(
  sessionToken: string,
  workspacePath: string,
  harnesses: HarnessKind[],
  signal?: AbortSignal,
): Promise<WorkspaceSessionScanResult> {
  return apiJson<WorkspaceSessionScanResult>(
    "/v1/workspace-sessions/scan",
    {
      method: "POST",
      body: JSON.stringify({ workspace_path: workspacePath, harnesses }),
      signal,
    },
    sessionToken,
  );
}

export async function importWorkspaceSessions(
  sessionToken: string,
  companyId: string,
  workspacePath: string,
  sessions: Array<Pick<NativeSessionSummary, "harness" | "native_session_id" | "workspace_path">>,
): Promise<{ imported: ImportedWorkspaceSession[] }> {
  return apiJson<{ imported: ImportedWorkspaceSession[] }>(
    "/v1/workspace-sessions/import",
    {
      method: "POST",
      body: JSON.stringify({
        company_id: companyId,
        workspace_path: workspacePath,
        sessions,
      }),
    },
    sessionToken,
  );
}

export async function upsertRuntimePolicy(
  sessionToken: string,
  conversationId: string,
  payload: {
    allow_agent_to_agent?: boolean;
    auto_mode?: "disabled" | "mentioned_only" | "metadata_only";
    default_coordinator_agent_id?: string;
    untagged_human_mode?: "mentioned_only" | "coordinator_only" | "all_agents";
  },
): Promise<RuntimePolicy> {
  return apiJson<RuntimePolicy>(
    `/v1/runtime/policies/${conversationId}`,
    {
      method: "PUT",
      body: JSON.stringify(payload),
    },
    sessionToken,
  );
}

export async function fetchConversationRuntimeStatus(
  sessionToken: string,
  conversationId: string,
): Promise<ConversationRuntimeStatus[]> {
  return apiJson<ConversationRuntimeStatus[]>(
    `/v1/conversations/${encodeURIComponent(conversationId)}/runtime-status`,
    {},
    sessionToken,
  );
}

export async function fetchChannelTasks(
  sessionToken: string,
  conversationId: string,
): Promise<ChannelTask[]> {
  return apiJson<ChannelTask[]>(
    `/v1/conversations/${encodeURIComponent(conversationId)}/tasks`,
    {},
    sessionToken,
  );
}

export async function fetchChannelTask(
  sessionToken: string,
  taskId: string,
): Promise<ChannelTaskDetail> {
  return apiJson<ChannelTaskDetail>(
    `/v1/tasks/${encodeURIComponent(taskId)}`,
    {},
    sessionToken,
  );
}

export async function createChannelTaskFromMessage(
  sessionToken: string,
  conversationId: string,
  payload: CreateChannelTaskFromMessageRequest,
): Promise<ChannelTask> {
  return apiJson<ChannelTask>(
    `/v1/conversations/${encodeURIComponent(conversationId)}/tasks/from-message`,
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
    sessionToken,
  );
}

export async function patchChannelTask(
  sessionToken: string,
  taskId: string,
  payload: PatchChannelTaskRequest,
): Promise<ChannelTask> {
  return apiJson<ChannelTask>(
    `/v1/tasks/${encodeURIComponent(taskId)}`,
    {
      method: "PATCH",
      body: JSON.stringify(payload),
    },
    sessionToken,
  );
}

export async function exportConversation(
  sessionToken: string,
  actorId: string,
  conversationId: string,
): Promise<unknown> {
  return apiJson(
    `/v1/export/conversations/${conversationId}?actor_id=${encodeURIComponent(actorId)}`,
    {},
    sessionToken,
  );
}

// ── SSH tunnels ──

export type SshHost = {
  name: string;
  hostname: string | null;
  user: string | null;
  port: number | null;
  identity_file: string | null;
};

export type SshTunnel = {
  id: string;
  host: string;
  local_port: number;
  remote_port: number;
  pid: number | null;
  started_at: string;
  generation: number | null;
  status: "ready" | "disconnected";
  disconnected_at?: string;
  last_error?: string;
};

export async function listSshHosts(sessionToken: string): Promise<SshHost[]> {
  return apiJson<SshHost[]>("/v1/ssh/hosts", {}, sessionToken);
}

export async function createSshTunnel(
  sessionToken: string,
  payload: { host: string; local_port?: number; remote_port?: number },
): Promise<SshTunnel> {
  return apiJson<SshTunnel>(
    "/v1/ssh/tunnel",
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
    sessionToken,
  );
}

/**
 * VS-Code-Remote-SSH analog: one-click connect to a remote Choruz instance.
 * Backend:
 *   1. `ssh <host> 'choruz-server'` — runs the headless binary remote-side,
 *      which prints `CHORUZ_LISTENING=<port>` on stdout.
 *   2. Reads that port, picks a free local high port.
 *   3. Opens the tunnel as a second ssh process.
 * The caller never supplies or sees port numbers.
 */
export async function connectChoruzSshTunnel(
  sessionToken: string,
  payload: { host: string; remote_binary?: string },
): Promise<SshTunnel> {
  return apiJson<SshTunnel>(
    "/v1/ssh/connect-choruz",
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
    sessionToken,
  );
}

export async function listSshTunnels(sessionToken: string): Promise<SshTunnel[]> {
  return apiJson<SshTunnel[]>("/v1/ssh/tunnels", {}, sessionToken);
}

export async function deleteSshTunnel(
  sessionToken: string,
  tunnelId: string,
): Promise<void> {
  await apiEmpty(
    `/v1/ssh/tunnel/${encodeURIComponent(tunnelId)}`,
    { method: "DELETE" },
    sessionToken,
  );
}

/** Bearer auth, JSON content type for bodies, and the active trace id. */
function requestHeaders(init: RequestInit, sessionToken?: string): Headers {
  const headers = new Headers(init.headers);
  if (init.body && !headers.has("content-type")) {
    headers.set("content-type", "application/json");
  }
  if (sessionToken) {
    headers.set("authorization", `Bearer ${sessionToken}`);
  }
  const traceId = currentTraceId();
  if (traceId) headers.set("x-trace-id", traceId);
  return headers;
}

async function apiEmpty(
  path: string,
  init: RequestInit = {},
  sessionToken?: string,
): Promise<void> {
  const response = await transportFetch(`${apiBaseUrl()}${path}`, {
    ...init,
    headers: requestHeaders(init, sessionToken),
    cache: "no-store",
  });
  if (!response.ok) {
    throw await apiRequestError(response);
  }
}

/**
 * Authenticated request used by the chat UI for endpoints without a typed
 * wrapper above. Like `apiJson`, but an empty response body resolves to
 * `undefined` instead of failing to parse.
 */
export async function apiFetch<T>(
  path: string,
  sessionToken: string,
  init: RequestInit = {},
): Promise<T> {
  const response = await transportFetch(`${apiBaseUrl()}${path}`, {
    ...init,
    headers: requestHeaders(init, sessionToken),
    cache: "no-store",
  });
  if (!response.ok) throw await apiRequestError(response);
  const text = await response.text();
  if (!text) return undefined as T;
  return JSON.parse(text) as T;
}

async function apiRequestError(response: Response): Promise<ApiRequestError> {
  let detail = `${response.status} ${response.statusText}`;
  try {
    const payload = (await response.json()) as { error?: { detail?: string } | string };
    if (typeof payload.error === "string") detail = payload.error;
    else if (payload.error && typeof payload.error.detail === "string") detail = payload.error.detail;
  } catch {}
  return new ApiRequestError(response.status, detail);
}

async function apiJson<T>(
  path: string,
  init: RequestInit = {},
  sessionToken?: string,
): Promise<T> {
  const response = await transportFetch(`${apiBaseUrl()}${path}`, {
    ...init,
    headers: requestHeaders(init, sessionToken),
    cache: "no-store",
  });

  if (!response.ok) throw await apiRequestError(response);

  return (await response.json()) as T;
}
