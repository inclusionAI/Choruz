import { expect, type Page } from "@playwright/test";
import { execFile } from "node:child_process";
import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import { promisify } from "node:util";
import { postgresQueryClient } from "../../lib/groups/group-provisioning-db";
import { API_BASE, WEB_BASE } from "./auth";

const execFileAsync = promisify(execFile);

/* -------------------------------------------------------------------------- */
/*  API helper – typed wrapper around page.request                             */
/* -------------------------------------------------------------------------- */

async function apiFetch<T>(
  page: Page,
  method: string,
  path: string,
  token: string,
  data?: Record<string, unknown>,
): Promise<T> {
  const opts: Parameters<Page["request"]["fetch"]>[1] = {
    method,
    headers: { Authorization: `Bearer ${token}` },
  };
  if (data) opts.data = data;
  const res = await page.request.fetch(`${API_BASE}${path}`, opts);
  expect(res.ok()).toBeTruthy();
  const text = await res.text();
  return text ? JSON.parse(text) : ({} as T);
}

/* -------------------------------------------------------------------------- */
/*  Console snapshot                                                           */
/* -------------------------------------------------------------------------- */

export async function getConsoleSnapshot(page: Page, token: string) {
  return apiFetch<{
    principal: { id: string; name: string };
    conversations: Array<{
      id: string;
      name: string | null;
      conversation_type: string;
      workspace_id: string;
      members: Record<string, { principal_id: string; joined_at: string }>;
    }>;
    agents: Array<{
      id: string;
      name: string;
      workspace_id: string;
      disabled: boolean;
    }>;
    pinned_conversations: Array<{
      conversation_id: string;
      pinned_at: string;
    }>;
  }>(page, "GET", "/v1/console", token);
}

/* -------------------------------------------------------------------------- */
/*  Company CRUD                                                               */
/* -------------------------------------------------------------------------- */

export async function createCompany(
  page: Page,
  token: string,
  principalId: string,
  name: string,
) {
  return apiFetch<{ id: string; name: string; slug: string }>(
    page,
    "POST",
    "/v1/companies",
    token,
    { actor_id: principalId, name, description: null },
  );
}

export async function deleteCompany(
  page: Page,
  token: string,
  companyId: string,
) {
  const res = await page.request.fetch(`${API_BASE}/v1/companies/${companyId}`, {
    method: "DELETE",
    headers: { Authorization: `Bearer ${token}` },
  });
  expect(res.ok(), `delete company -> ${res.status()}`).toBeTruthy();
}

export async function addCompanyMember(
  page: Page,
  token: string,
  companyId: string,
  actorId: string,
  principalId: string,
) {
  return apiFetch<{ company_id: string; principal_id: string; joined_at: string }>(
    page,
    "POST",
    `/v1/companies/${companyId}/members`,
    token,
    { actor_id: actorId, principal_id: principalId },
  );
}

/* -------------------------------------------------------------------------- */
/*  Agent provisioning                                                         */
/* -------------------------------------------------------------------------- */

type ProvisionAgentOptions = {
  driver?: string;
  workspaceId?: string;
  webhookUrl?: string;
};

export async function provisionAgent(
  page: Page,
  token: string,
  name: string,
  options: ProvisionAgentOptions = {},
) {
  const driver = options.driver ?? "claude_terminal";
  const res = await page.request.post(`${WEB_BASE}/api/agents/provision`, {
    data: {
      name,
      driver_type: driver,
      instructions: `E2E test agent: ${name}`,
      ...(options.workspaceId ? { workspace_id: options.workspaceId } : {}),
      ...(options.webhookUrl ? { webhook_url: options.webhookUrl } : {}),
    },
  });
  expect(res.ok(), `agent provision -> ${res.status()}: ${await res.text().catch(() => "")}`).toBeTruthy();
  const data = await res.json();
  return {
    agentId: data.agent.id as string,
    agentName: data.agent.name as string,
    secret: data.secret as string,
    conversationId: data.conversation?.id as string | undefined,
    workspacePath: data.workspace_path as string,
  };
}

export async function createAgent(
  page: Page,
  token: string,
  principalId: string,
  name: string,
  workspaceId?: string,
  channelVisibility?: "visible" | "internal",
) {
  return apiFetch<{
    principal: {
      id: string;
      name: string;
      workspace_id: string;
      principal_type: string;
      disabled: boolean;
      channel_visibility?: "visible" | "internal";
    };
    secret: string;
  }>(page, "POST", "/v1/agents", token, {
    actor_id: principalId,
    name,
    scopes: ["messages:read", "messages:write"],
    ...(workspaceId ? { workspace_id: workspaceId } : {}),
    ...(channelVisibility ? { channel_visibility: channelVisibility } : {}),
  });
}

export async function disablePrincipal(
  page: Page,
  token: string,
  actorId: string,
  principalId: string,
) {
  return apiFetch<{
    id: string;
    name: string;
    workspace_id: string;
    principal_type: string;
    disabled: boolean;
  }>(
    page,
    "POST",
    `/v1/principals/${principalId}/disable?actor_id=${encodeURIComponent(actorId)}`,
    token,
  );
}

export async function createDirectConversation(
  page: Page,
  token: string,
  principalId: string,
  peerPrincipalId: string,
  workspaceId?: string,
) {
  return apiFetch<{
    id: string;
    name: string | null;
    conversation_type: string;
    members: Record<string, { principal_id: string; joined_at: string }>;
  }>(page, "POST", "/v1/conversations/direct", token, {
    actor_id: principalId,
    peer_principal_id: peerPrincipalId,
    ...(workspaceId ? { workspace_id: workspaceId } : {}),
  });
}

/* -------------------------------------------------------------------------- */
/*  Group management                                                           */
/* -------------------------------------------------------------------------- */

export async function createGroup(
  page: Page,
  token: string,
  principalId: string,
  name: string,
  memberIds: string[] = [],
  workspaceId?: string,
) {
  return apiFetch<{
    id: string;
    name: string;
    conversation_type: string;
    members: Record<string, { principal_id: string; joined_at: string }>;
  }>(page, "POST", "/v1/groups", token, {
    actor_id: principalId,
    name,
    description: null,
    avatar_url: null,
    member_ids: memberIds,
    ...(workspaceId ? { workspace_id: workspaceId } : {}),
  });
}

export async function createAndOpenGroup(
  page: Page,
  token: string,
  principalId: string,
  prefix: string = "e2e-group",
) {
  const group = await createGroup(page, token, principalId, uniqueName(prefix));
  await page.goto(`${WEB_BASE}/dashboard`);
  const groupHeader = page
    .getByRole("group", { name: "Group Conversations" })
    .getByRole("button", { name: /^Group Conversations/ });
  await expect(groupHeader).toBeVisible({ timeout: 15_000 });
  if ((await groupHeader.getAttribute("aria-expanded")) !== "true") {
    await groupHeader.click();
  }
  const item = page.locator(".conv-item").filter({ hasText: group.name }).first();
  await expect(item).toBeVisible({ timeout: 15_000 });
  await item.click();
  await expect(page.locator("textarea").first()).toBeVisible({ timeout: 10_000 });
  return group;
}

export async function addGroupMember(
  page: Page,
  token: string,
  groupId: string,
  actorId: string,
  memberIds: string[],
) {
  return apiFetch<void>(page, "POST", `/v1/groups/${groupId}/members`, token, {
    actor_id: actorId,
    member_ids: memberIds,
  });
}

// Test-only runtime policy helper: this spec intentionally uses non-runtime
// target agents, so the operator must stay authorized as the group coordinator
// after handing ownership to another visible assignee.
export async function setConversationCoordinatorForTest(
  conversationId: string,
  coordinatorAgentId: string,
) {
  await withPgClient(async (client) => {
    const result = await client.query(
      `INSERT INTO conversation_runtime_policies
         (conversation_id, default_coordinator_agent_id)
       VALUES ($1, $2)
       ON CONFLICT (conversation_id) DO UPDATE
       SET default_coordinator_agent_id = EXCLUDED.default_coordinator_agent_id,
           updated_at = NOW()`,
      [conversationId, coordinatorAgentId],
    );
    expect(result.rowCount, `runtime policy for ${conversationId} should be upserted`).toBe(1);
  });
}

type GroupMemberRemovalResponse = {
  id: string;
  conversation_type?: string;
  members?: Record<string, { principal_id: string; joined_at: string }>;
};

export async function removeGroupMember(
  page: Page,
  token: string,
  groupId: string,
  actorId: string,
  principalId: string,
) {
  return apiFetch<GroupMemberRemovalResponse>(
    page,
    "DELETE",
    `/v1/groups/${groupId}/members/${principalId}?actor_id=${encodeURIComponent(actorId)}`,
    token,
  );
}

type PgQueryResult<Row = Record<string, unknown>> = {
  rowCount: number | null;
  rows: Row[];
};

type PgClient = {
  query<Row = Record<string, unknown>>(sql: string, params?: unknown[]): Promise<PgQueryResult<Row>>;
};

async function withPgClient<T>(fn: (client: PgClient) => Promise<T>): Promise<T> {
  const client = await postgresQueryClient() as PgClient;
  return fn(client);
}

// Test-only restore helper: the product exposes disable but not re-enable.
// Runtime roster assertions below intentionally verify the DB-backed router path.
export async function setPrincipalDisabledForTest(
  principalId: string,
  disabled: boolean,
) {
  await withPgClient(async (client) => {
    const result = await client.query(
      "UPDATE principal SET disabled = $1, updated_at = NOW() WHERE id = $2",
      [disabled, principalId],
    );
    expect(result.rowCount, `principal ${principalId} should exist`).toBe(1);
  });
}

// Test-only visibility toggle: there is no member-facing update route for this field.
export async function setPrincipalChannelVisibilityForTest(
  principalId: string,
  channelVisibility: "visible" | "internal",
) {
  await withPgClient(async (client) => {
    const result = await client.query(
      "UPDATE principal SET channel_visibility = $1, updated_at = NOW() WHERE id = $2",
      [channelVisibility, principalId],
    );
    expect(result.rowCount, `principal ${principalId} should exist`).toBe(1);
  });
}

export async function waitForAgentPromptForTest(
  page: Page,
  agentId: string,
  conversationId: string,
  messageId: string,
  timeoutMs = 30_000,
): Promise<string> {
  const startedAt = Date.now();
  let lastPrompts: string[] = [];
  while (Date.now() - startedAt < timeoutMs) {
    const prompts = await withPgClient(async (client) => {
      const result = await client.query<{ prompt: string }>(
        `SELECT prompt
           FROM agent_commands
          WHERE agent_id = $1
            AND conversation_id = $2
            AND message_id = $3
          ORDER BY created_at DESC
          LIMIT 5`,
        [agentId, conversationId, messageId],
      );
      return result.rows.map((row) => row.prompt);
    });
    if (prompts[0]) return prompts[0];
    lastPrompts = prompts;
    await page.waitForTimeout(500);
  }
  throw new Error(
    `Timed out waiting for agent prompt for message ${messageId}. Last prompts: ${JSON.stringify(lastPrompts)}`,
  );
}

/* -------------------------------------------------------------------------- */
/*  Messages                                                                   */
/* -------------------------------------------------------------------------- */

export async function sendMessage(
  page: Page,
  token: string,
  principalId: string,
  conversationId: string,
  content: string,
) {
  return apiFetch<{ id: string; content: string; server_seq: number }>(
    page,
    "POST",
    "/v1/messages",
    token,
    {
      actor_id: principalId,
      conversation_id: conversationId,
      content,
      content_type: "text/plain",
      idempotency_key: `e2e-${Date.now()}-${Math.random().toString(16).slice(2)}`,
      metadata: {},
    },
  );
}

export async function getMessages(
  page: Page,
  token: string,
  principalId: string,
  conversationId: string,
  limit: number = 50,
) {
  return apiFetch<
    Array<{
      id: string;
      sender_id: string;
      content: string;
      server_seq: number;
      created_at: string;
    }>
  >(
    page,
    "GET",
    `/v1/conversations/${conversationId}/messages?principal_id=${principalId}&limit=${limit}`,
    token,
  );
}

/* -------------------------------------------------------------------------- */
/*  Channel task agent command helpers                                         */
/* -------------------------------------------------------------------------- */

export type ChannelTaskE2E = {
  task_id: string;
  conversation_id: string;
  task_key: string;
  title: string;
  status: "todo" | "in_progress" | "blocked" | "in_review" | "done";
  assignee_principal_id?: string;
  assignee_name?: string;
  source_kind: "agent" | "message";
  version: number;
};

export type OutboxCommandResult = {
  command_type?: string;
  ok?: boolean;
  error_code?: string;
  message?: string;
  task_key?: string;
  task_id?: string;
  idempotency_key?: string;
  emitted_at?: string;
};

export async function writeAgentOutboxCommand(
  workspacePath: string,
  command: Record<string, unknown>,
) {
  await execFileAsync(`${workspacePath}/.choruz/send`, [JSON.stringify(command)]);
}

export async function fetchChannelTasks(
  page: Page,
  token: string,
  conversationId: string,
): Promise<ChannelTaskE2E[]> {
  return apiFetch<ChannelTaskE2E[]>(
    page,
    "GET",
    `/v1/conversations/${conversationId}/tasks`,
    token,
  );
}

export async function waitForChannelTask(
  page: Page,
  token: string,
  conversationId: string,
  predicate: (task: ChannelTaskE2E) => boolean,
  timeoutMs = 30_000,
): Promise<ChannelTaskE2E> {
  const startedAt = Date.now();
  let lastTasks: ChannelTaskE2E[] = [];
  while (Date.now() - startedAt < timeoutMs) {
    lastTasks = await fetchChannelTasks(page, token, conversationId);
    const task = lastTasks.find(predicate);
    if (task) return task;
    await page.waitForTimeout(1_000);
  }
  throw new Error(
    `Timed out waiting for channel task. Last tasks: ${JSON.stringify(lastTasks)}`,
  );
}

export async function waitForOutboxCommandResult(
  page: Page,
  workspacePath: string,
  predicate: (result: OutboxCommandResult) => boolean,
  timeoutMs = 45_000,
): Promise<OutboxCommandResult> {
  const startedAt = Date.now();
  let lastResults: OutboxCommandResult[] = [];
  while (Date.now() - startedAt < timeoutMs) {
    lastResults = await readOutboxCommandResults(workspacePath);
    const result = lastResults.find(predicate);
    if (result) return result;
    await page.waitForTimeout(1_000);
  }
  throw new Error(
    `Timed out waiting for outbox command result. Last results: ${JSON.stringify(lastResults)}`,
  );
}

export async function readOutboxCommandResults(
  workspacePath: string,
): Promise<OutboxCommandResult[]> {
  const resultsDir = join(workspacePath, ".choruz-outbox", "results");
  let files: string[];
  try {
    files = await readdir(resultsDir);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return [];
    throw error;
  }

  const results: OutboxCommandResult[] = [];
  for (const file of files.filter((name) => name.endsWith(".json"))) {
    const raw = await readFile(join(resultsDir, file), "utf8");
    try {
      results.push(JSON.parse(raw) as OutboxCommandResult);
    } catch {
      // Ignore malformed leftovers; command result writes are atomic, so this
      // should only happen for unrelated manual files in a reused workspace.
    }
  }
  return results;
}

export async function pinConversation(
  page: Page,
  token: string,
  conversationId: string,
) {
  await apiFetch<void>(
    page,
    "PUT",
    `/v1/conversations/${conversationId}/pin`,
    token,
  );
}

export async function unpinConversation(
  page: Page,
  token: string,
  conversationId: string,
) {
  await apiFetch<void>(
    page,
    "DELETE",
    `/v1/conversations/${conversationId}/pin`,
    token,
  );
}

/* -------------------------------------------------------------------------- */
/*  Helpers                                                                    */
/* -------------------------------------------------------------------------- */

/** Generate a unique name with a test-run prefix for easy cleanup. */
export function uniqueName(prefix: string): string {
  return `${prefix}-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
}
