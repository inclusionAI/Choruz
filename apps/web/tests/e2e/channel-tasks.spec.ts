import { expect, test, type Page } from "@playwright/test";
import { createServer, type Server } from "node:http";
import type {
  GroupLaunchPlanContract,
  GroupProvisioningJobContract,
  GroupProvisioningStepResult,
} from "../../lib/groups/group-provisioning-contract";
import { getGroupTemplate } from "../../lib/groups/team-templates";
import { API_BASE, WEB_BASE, login } from "../fixtures/auth";
import {
  addGroupMember,
  createAgent,
  createGroup,
  createDirectConversation,
  disablePrincipal,
  fetchChannelTasks,
  getMessages,
  provisionAgent,
  removeGroupMember,
  sendMessage,
  readOutboxCommandResults,
  setConversationCoordinatorForTest,
  setPrincipalChannelVisibilityForTest,
  setPrincipalDisabledForTest,
  uniqueName,
  waitForAgentPromptForTest,
  waitForChannelTask,
  waitForOutboxCommandResult,
  writeAgentOutboxCommand,
} from "../fixtures/api";

/*
 * Channel kanban board E2E coverage for verification slices 9.9, 9.10,
 * 9.13, 9.14, and 9.20.
 *
 * These specs assume the local dev stack is started with
 * `CHORUZ_PLUGINS` including `kanban` so the gateway, pipeline, and web app
 * all load the plugin. `infra/host/web_e2e.sh` forwards inherited env vars
 * to its child processes.
 *
 * If the plugin is disabled, the first spec detects its absence from the
 * console snapshot and skips the rest of the file with a clear message.
 */

async function openConversation(page: Page, conversationId: string) {
  await page.goto(`${WEB_BASE}/dashboard?conversationId=${conversationId}`);
  await page.waitForSelector(".chat-sidebar, .chat-app", { timeout: 15_000 });
  await page.waitForSelector(".chat-header, .message-list, .terminal-container", {
    timeout: 15_000,
  });
}

async function ensureKanbanPluginEnabled(page: Page, token: string) {
  const res = await page.request.get(`${API_BASE}/v1/console`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  expect(res.ok()).toBeTruthy();
  const snapshot = (await res.json()) as { plugins?: Array<{ id?: string }> };
  const enabled = snapshot.plugins?.some((plugin) => plugin.id === "kanban") === true;
  test.skip(
    !enabled,
    "CHORUZ_PLUGINS must include kanban on the gateway to run channel task E2E",
  );
}

async function gotoGroupTasksTab(page: Page, conversationId: string) {
  await openConversation(page, conversationId);
  const tasksTab = page.getByRole("tab", { name: "Tasks" });
  await expect(tasksTab).toBeVisible({ timeout: 15_000 });
  await tasksTab.click();
  await expect(page.locator(".channel-task-board")).toBeVisible({ timeout: 15_000 });
}

async function expectNoAgentChatMessages(
  page: Page,
  token: string,
  principalId: string,
  conversationId: string,
  baselineMaxServerSeq: number,
  agentIds: string[],
  quietWindowMs = 2_000,
) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < quietWindowMs) {
    const messages = await getMessages(page, token, principalId, conversationId, 200);
    const newMessages = messages.filter((message) => message.server_seq > baselineMaxServerSeq);
    const agentMessages = newMessages.filter((message) => agentIds.includes(message.sender_id));
    expect(agentMessages.map((message) => message.content)).toEqual([]);
    await page.waitForTimeout(500);
  }
}

async function startVisibleAgentTaskWebhook(
  config: {
    workspacePath?: string;
    conversationId?: string;
    ownerAgentId?: string;
  },
  reviewerAgentId: string,
  taskKey: string,
  taskTitle: string,
  runId: number,
) {
  let appMentionCount = 0;
  let emittedCount = 0;
  const seenMentionContents = new Set<string>();
  const serverErrors: string[] = [];
  const server = createServer((req, res) => {
    if (req.method !== "POST") {
      res.writeHead(405).end();
      return;
    }

    let body = "";
    req.setEncoding("utf8");
    req.on("data", (chunk) => {
      body += chunk;
    });
    req.on("end", () => {
      void (async () => {
        const payload = body ? JSON.parse(body) as {
          event_type?: string;
          payload?: { content?: string };
        } : {};
        if (payload.event_type === "app_mention" && emittedCount < 3) {
          const content = payload.payload?.content ?? "";
          if (seenMentionContents.has(content)) {
            res.writeHead(200, { "content-type": "application/json" });
            res.end(JSON.stringify({ ok: true }));
            return;
          }
          seenMentionContents.add(content);
          const emissionIndex = emittedCount;
          appMentionCount += 1;
          emittedCount += 1;
          const expectedContent =
            emissionIndex === 0
              ? "multi-step board request"
              : emissionIndex === 1
                ? "mark billing cleanup implementation in progress"
                : "hand billing cleanup review";
          expect(content).toContain(expectedContent);
          const { workspacePath, conversationId, ownerAgentId } = config;
          if (!workspacePath || !conversationId || !ownerAgentId) {
            throw new Error("webhook command config is incomplete");
          }
          if (emissionIndex === 0) {
            await writeAgentOutboxCommand(workspacePath, {
              type: "task_create",
              conversation_id: conversationId,
              task_key: taskKey,
              title: taskTitle,
              assignee_principal_id: ownerAgentId,
              idempotency_key: `agent-multistep-create-${runId}`,
            });
          } else if (emissionIndex === 1) {
            await writeAgentOutboxCommand(workspacePath, {
              type: "task_update",
              conversation_id: conversationId,
              task_key: taskKey,
              status: "in_progress",
              idempotency_key: `agent-multistep-update-${runId}`,
            });
          } else {
            await writeAgentOutboxCommand(workspacePath, {
              type: "task_transfer",
              conversation_id: conversationId,
              task_key: taskKey,
              assignee_principal_id: reviewerAgentId,
              idempotency_key: `agent-multistep-transfer-${runId}`,
            });
          }
        }
        res.writeHead(200, { "content-type": "application/json" });
        res.end(JSON.stringify({ ok: true }));
      })().catch((error) => {
        serverErrors.push(error instanceof Error ? error.message : "webhook failed");
        res.writeHead(500, { "content-type": "application/json" });
        res.end(JSON.stringify({ error: error instanceof Error ? error.message : "webhook failed" }));
      });
    });
  });

  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      server.off("error", reject);
      resolve();
    });
  });
  const address = server.address();
  if (!address || typeof address === "string") {
    throw new Error("webhook test server did not bind to a TCP port");
  }

  return {
    url: `http://127.0.0.1:${address.port}/hook`,
    server,
    appMentionCount: () => appMentionCount,
    emittedCount: () => emittedCount,
    serverErrors: () => [...serverErrors],
  };
}

type ScreenshotSmokeTask = {
  taskKey: string;
  title: string;
  assigneeName: string;
  assigneePrincipalId: string;
  idempotencyKey: string;
};

type ScreenshotSmokeCommandScript = {
  taskCreates: ScreenshotSmokeTask[];
  receiptContent: string;
};

async function startSoftwareTeamScreenshotSmokeWebhook(
  config: {
    workspacePath?: string;
    conversationId?: string;
    groupName?: string;
  },
  script: ScreenshotSmokeCommandScript,
) {
  let appMentionCount = 0;
  let emittedCount = 0;
  const serverErrors: string[] = [];
  const server = createServer((req, res) => {
    if (req.method !== "POST") {
      res.writeHead(405).end();
      return;
    }

    let body = "";
    req.setEncoding("utf8");
    req.on("data", (chunk) => {
      body += chunk;
    });
    req.on("end", () => {
      void (async () => {
        const payload = body ? JSON.parse(body) as {
          event_type?: string;
          payload?: { content?: string };
        } : {};
        if (payload.event_type === "app_mention" && emittedCount === 0) {
          const content = payload.payload?.content ?? "";
          if (!content.includes("software team screenshot smoke")) {
            res.writeHead(200, { "content-type": "application/json" });
            res.end(JSON.stringify({ ok: true }));
            return;
          }
          const { workspacePath, conversationId, groupName } = config;
          if (!workspacePath || !conversationId || !groupName) {
            throw new Error("software team screenshot smoke webhook config is incomplete");
          }

          for (const task of script.taskCreates) {
            await writeAgentOutboxCommand(workspacePath, {
              type: "task_create",
              conversation_id: conversationId,
              task_key: task.taskKey,
              title: task.title,
              assignee_principal_id: task.assigneePrincipalId,
              idempotency_key: task.idempotencyKey,
            });
          }

          await writeAgentOutboxCommand(workspacePath, {
            type: "send",
            group: groupName,
            content: script.receiptContent,
            metadata: { source: "software_team_screenshot_smoke" },
          });
          appMentionCount += 1;
          emittedCount += script.taskCreates.length + 1;
        }
        res.writeHead(200, { "content-type": "application/json" });
        res.end(JSON.stringify({ ok: true }));
      })().catch((error) => {
        serverErrors.push(error instanceof Error ? error.message : "webhook failed");
        res.writeHead(500, { "content-type": "application/json" });
        res.end(JSON.stringify({ error: error instanceof Error ? error.message : "webhook failed" }));
      });
    });
  });

  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      server.off("error", reject);
      resolve();
    });
  });
  const address = server.address();
  if (!address || typeof address === "string") {
    throw new Error("webhook test server did not bind to a TCP port");
  }

  return {
    url: `http://127.0.0.1:${address.port}/hook`,
    server,
    appMentionCount: () => appMentionCount,
    emittedCount: () => emittedCount,
    serverErrors: () => [...serverErrors],
  };
}

async function startTrivialRequestDecisionWebhook(
  config: {
    workspacePath?: string;
    trivialGroupName?: string;
    trackedConversationId?: string;
    ownerAgentId?: string;
    trackedTaskKey?: string;
    trackedTaskTitle?: string;
    trackIdempotencyKey?: string;
  },
  runId: number,
  trivialReplyContent: string,
) {
  let appMentionCount = 0;
  let emittedCount = 0;
  const emittedCommandTypes: string[] = [];
  const handledBranches = new Set<string>();
  const serverErrors: string[] = [];
  const server = createServer((req, res) => {
    if (req.method !== "POST") {
      res.writeHead(405).end();
      return;
    }

    let body = "";
    req.setEncoding("utf8");
    req.on("data", (chunk) => {
      body += chunk;
    });
    req.on("end", () => {
      void (async () => {
        const payload = body ? JSON.parse(body) as {
          event_type?: string;
          payload?: { content?: string };
        } : {};
        if (payload.event_type !== "app_mention") {
          res.writeHead(200, { "content-type": "application/json" });
          res.end(JSON.stringify({ ok: true }));
          return;
        }

        appMentionCount += 1;
        const content = payload.payload?.content ?? "";
        const branch = content.includes(`trivial one-turn request ${runId}`)
          ? "trivial"
          : content.includes(`explicit tracking request ${runId}`)
            ? "explicit-tracking"
            : null;
        if (!branch || handledBranches.has(branch)) {
          res.writeHead(200, { "content-type": "application/json" });
          res.end(JSON.stringify({ ok: true }));
          return;
        }

        const { workspacePath } = config;
        if (!workspacePath) {
          throw new Error("decision webhook command config is missing workspacePath");
        }

        if (branch === "trivial") {
          const { trivialGroupName } = config;
          if (!trivialGroupName) {
            throw new Error("decision webhook command config is missing trivialGroupName");
          }
          const command = {
            type: "send",
            group: trivialGroupName,
            content: trivialReplyContent,
          };
          handledBranches.add(branch);
          emittedCommandTypes.push(command.type);
          await writeAgentOutboxCommand(workspacePath, command);
          emittedCount += 1;
        } else {
          const {
            trackedConversationId,
            ownerAgentId,
            trackedTaskKey,
            trackedTaskTitle,
            trackIdempotencyKey,
          } = config;
          if (!trackedConversationId || !ownerAgentId || !trackedTaskKey || !trackedTaskTitle || !trackIdempotencyKey) {
            throw new Error("decision webhook command config is missing tracking fields");
          }
          const command = {
            type: "task_create",
            conversation_id: trackedConversationId,
            task_key: trackedTaskKey,
            title: trackedTaskTitle,
            assignee_principal_id: ownerAgentId,
            idempotency_key: trackIdempotencyKey,
          };
          handledBranches.add(branch);
          emittedCommandTypes.push(command.type);
          await writeAgentOutboxCommand(workspacePath, command);
          emittedCount += 1;
        }

        res.writeHead(200, { "content-type": "application/json" });
        res.end(JSON.stringify({ ok: true }));
      })().catch((error) => {
        serverErrors.push(error instanceof Error ? error.message : "webhook failed");
        res.writeHead(500, { "content-type": "application/json" });
        res.end(JSON.stringify({ error: error instanceof Error ? error.message : "webhook failed" }));
      });
    });
  });

  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      server.off("error", reject);
      resolve();
    });
  });
  const address = server.address();
  if (!address || typeof address === "string") {
    throw new Error("webhook test server did not bind to a TCP port");
  }

  return {
    url: `http://127.0.0.1:${address.port}/hook`,
    server,
    appMentionCount: () => appMentionCount,
    emittedCount: () => emittedCount,
    emittedCommandTypes: () => [...emittedCommandTypes],
    serverErrors: () => [...serverErrors],
  };
}

async function closeServer(server: Server) {
  server.closeAllConnections?.();
  await new Promise<void>((resolve, reject) => {
    server.close((error) => {
      server.closeAllConnections?.();
      if (error) reject(error);
      else resolve();
    });
  });
}

async function startNoopAgentWebhook() {
  const server = createServer((req, res) => {
    if (req.method !== "POST") {
      res.writeHead(405).end();
      return;
    }
    req.resume();
    req.on("end", () => {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify({ ok: true }));
    });
  });

  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      server.off("error", reject);
      resolve();
    });
  });
  const address = server.address();
  if (!address || typeof address === "string") {
    throw new Error("noop webhook test server did not bind to a TCP port");
  }
  return { url: `http://127.0.0.1:${address.port}/hook`, server };
}

type RuntimeRosterEntry = { id: string; name: string; type: string };

function parseRuntimeRoster(prompt: string): RuntimeRosterEntry[] {
  const rosterStart = prompt.indexOf(" roster:");
  expect(rosterStart, `expected prompt to include roster field: ${prompt}`).toBeGreaterThanOrEqual(0);
  const jsonStart = rosterStart + " roster:".length;
  expect(prompt[jsonStart], `expected prompt roster to start with a JSON array: ${prompt}`).toBe("[");

  let depth = 0;
  let inString = false;
  let escaping = false;
  for (let index = jsonStart; index < prompt.length; index += 1) {
    const char = prompt[index];
    if (inString) {
      if (escaping) {
        escaping = false;
      } else if (char === "\\") {
        escaping = true;
      } else if (char === '"') {
        inString = false;
      }
      continue;
    }
    if (char === '"') {
      inString = true;
    } else if (char === "[") {
      depth += 1;
    } else if (char === "]") {
      depth -= 1;
      if (depth === 0) {
        return JSON.parse(prompt.slice(jsonStart, index + 1)) as RuntimeRosterEntry[];
      }
    }
  }
  throw new Error(`Could not parse runtime roster JSON array from prompt: ${prompt}`);
}

function expectRuntimeRoster(roster: RuntimeRosterEntry[], label: string, expected: { present?: string[]; absent?: string[] }) {
  const ids = roster.map((entry) => entry.id);
  for (const id of expected.present ?? []) expect(ids, `${label}: expected runtime roster to include ${id}`).toContain(id);
  for (const id of expected.absent ?? []) expect(ids, `${label}: expected runtime roster to omit ${id}`).not.toContain(id);
}

async function waitForWebhookCommandEmission(
  page: Page,
  emittedCount: () => number,
  expectedCount = 1,
  timeoutMs = 15_000,
  serverErrors: () => string[] = () => [],
) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeoutMs) {
    const errors = serverErrors();
    if (errors.length > 0) {
      throw new Error(`Webhook test server failed: ${errors.join("; ")}`);
    }
    if (emittedCount() >= expectedCount) return;
    await page.waitForTimeout(250);
  }
  throw new Error("Timed out waiting for visible agent webhook command emission");
}

async function expectNoWorkspaceTaskCommandResultsOrConversationTasks(
  page: Page,
  token: string,
  conversationId: string,
  workspacePath: string,
  quietWindowMs = 15_000,
) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < quietWindowMs) {
    const [tasks, results] = await Promise.all([
      fetchChannelTasks(page, token, conversationId),
      readOutboxCommandResults(workspacePath),
    ]);
    expect(results.filter((result) => result.command_type?.startsWith("task_"))).toEqual([]);
    expect(tasks).toEqual([]);
    await page.waitForTimeout(500);
  }
}

async function waitForConversationMessage(
  page: Page,
  token: string,
  principalId: string,
  conversationId: string,
  predicate: (message: { sender_id: string; content: string; server_seq: number }) => boolean,
  timeoutMs = 30_000,
) {
  const startedAt = Date.now();
  let lastMessages: Array<{ sender_id: string; content: string; server_seq: number }> = [];
  while (Date.now() - startedAt < timeoutMs) {
    lastMessages = await getMessages(page, token, principalId, conversationId, 50);
    const message = lastMessages.find(predicate);
    if (message) return message;
    await page.waitForTimeout(1_000);
  }
  throw new Error(`Timed out waiting for conversation message. Last messages: ${JSON.stringify(lastMessages)}`);
}

function assignmentReceiptLines(content: string): string[] {
  const lines = content.split(/\r?\n/);
  const start = lines.findIndex((line) => /^Assignments:\s*$/i.test(line.trim()));
  if (start === -1) return [];
  const receiptLines: string[] = [];
  for (const line of lines.slice(start + 1)) {
    if (!line.trim()) break;
    if (/^[A-Z][A-Za-z ]+:\s*$/.test(line.trim())) break;
    if (/^\s*-\s+/.test(line)) receiptLines.push(line.trim());
  }
  return receiptLines;
}

async function createGroupProvisioningJobViaApp(
  page: Page,
  token: string,
  request: { idempotencyKey: string; companyId: string; plan: GroupLaunchPlanContract },
): Promise<GroupProvisioningJobContract> {
  const response = await page.request.post(`${WEB_BASE}/api/group-provisioning-jobs`, {
    headers: { Cookie: `choruz_session=${token}` },
    data: request,
  });
  expect(
    response.ok(),
    `create group provisioning job -> ${response.status()}: ${await response.text().catch(() => "")}`,
  ).toBeTruthy();
  const payload = await response.json() as { job: GroupProvisioningJobContract };
  return payload.job;
}

async function runGroupProvisioningJobViaApp(
  page: Page,
  token: string,
  jobId: string,
  maxSteps = 10,
): Promise<GroupProvisioningJobContract> {
  const response = await page.request.post(`${WEB_BASE}/api/group-provisioning-jobs/${jobId}/run`, {
    headers: { Cookie: `choruz_session=${token}` },
    data: { maxSteps },
  });
  expect(
    response.ok(),
    `run group provisioning job -> ${response.status()}: ${await response.text().catch(() => "")}`,
  ).toBeTruthy();
  const payload = await response.json() as { job: GroupProvisioningJobContract };
  return payload.job;
}

async function runGroupProvisioningJobToCompletion(
  page: Page,
  token: string,
  job: GroupProvisioningJobContract,
): Promise<GroupProvisioningJobContract> {
  let current = job;
  const terminal = new Set(["completed", "completed_with_warning", "failed", "failed_validation", "partial_failure", "rolled_back", "canceled"]);
  for (let attempt = 0; attempt < 20 && !terminal.has(current.status); attempt += 1) {
    current = await runGroupProvisioningJobViaApp(page, token, current.id, 10);
  }
  expect(
    current.status,
    `provisioning job ${current.id} should complete; status=${current.status}; steps=${current.stepResults.length}`,
  ).toBe("completed");
  return current;
}

function stepResult<K extends GroupProvisioningStepResult["kind"]>(
  job: GroupProvisioningJobContract,
  kind: K,
  roleSlotId?: string,
): Extract<GroupProvisioningStepResult, { kind: K }> {
  const result = job.stepResults.find((candidate) =>
    candidate.kind === kind && (!roleSlotId || ("roleSlotId" in candidate && candidate.roleSlotId === roleSlotId)),
  );
  expect(result, `expected ${kind} step result${roleSlotId ? ` for ${roleSlotId}` : ""}`).toBeTruthy();
  return result as Extract<GroupProvisioningStepResult, { kind: K }>;
}

function requireSoftwareTeamTemplate() {
  const template = getGroupTemplate("software-development-team");
  expect(template).toBeTruthy();
  return template!;
}

function roleTemplateIdForSoftwareTeamSlot(slotId: string): string {
  const template = requireSoftwareTeamTemplate();
  const slot = template.roleSlots.find((candidate) => candidate.id === slotId);
  expect(slot, `software-development-team role slot should exist: ${slotId}`).toBeTruthy();
  return slot!.roleTemplateId;
}

function softwareTeamSmokePlan(options: {
  groupName: string;
  mission: string;
  operatorAgentId: string;
  operatorDisplayName: string;
}): GroupLaunchPlanContract {
  const version = "1.0.0" as const;
  return {
    groupTemplateId: "software-development-team",
    groupTemplateVersion: version,
    groupName: options.groupName,
    mission: options.mission,
    kickoffText: [
      `Mission: ${options.mission}`,
      "",
      "Workflow: Plan -> implement -> review -> verify -> summarize.",
      "",
      "Please wait for the user to provide the first concrete work item before starting execution.",
      "",
      "Roles: Project Operator, Backend Engineer, Code Reviewer.",
      "",
      "Next user action: send the first concrete work item or question when ready. Until then, work waits for the user kickoff.",
    ].join("\n"),
    startWorkMode: "manual",
    workflow: {
      coordinatorRoleSlotId: "project-operator",
      participantRoleDefaults: {
        "project-operator": ["coordinator"],
        "backend-engineer": ["owner"],
        "frontend-engineer": ["owner"],
        "code-reviewer": ["reviewer", "quality_check"],
        "qa-tester": ["quality_check"],
        "devops-engineer": ["operations"],
      },
    },
    rolePlans: [
      {
        slotId: "project-operator",
        action: "reuse",
        existingAgentId: options.operatorAgentId,
        displayName: options.operatorDisplayName,
        roleTemplateId: roleTemplateIdForSoftwareTeamSlot("project-operator"),
        roleTemplateVersion: version,
      },
      {
        slotId: "backend-engineer",
        action: "create",
        agentName: "backend-engineer",
        roleTemplateId: roleTemplateIdForSoftwareTeamSlot("backend-engineer"),
        roleTemplateVersion: version,
        driver: "codex_terminal",
        instructionStatus: "group_context_added",
        setupInputs: {},
        selectedSkills: [],
        workspaceMode: "generated",
      },
      {
        slotId: "code-reviewer",
        action: "create",
        agentName: "code-reviewer",
        roleTemplateId: roleTemplateIdForSoftwareTeamSlot("code-reviewer"),
        roleTemplateVersion: version,
        driver: "codex_terminal",
        instructionStatus: "group_context_added",
        setupInputs: {},
        selectedSkills: [],
        workspaceMode: "generated",
      },
      {
        slotId: "frontend-engineer",
        action: "skip",
        roleTemplateId: roleTemplateIdForSoftwareTeamSlot("frontend-engineer"),
        roleTemplateVersion: version,
        reason: "user_choice",
      },
      {
        slotId: "qa-tester",
        action: "skip",
        roleTemplateId: roleTemplateIdForSoftwareTeamSlot("qa-tester"),
        roleTemplateVersion: version,
        reason: "user_choice",
      },
      {
        slotId: "devops-engineer",
        action: "skip",
        roleTemplateId: roleTemplateIdForSoftwareTeamSlot("devops-engineer"),
        roleTemplateVersion: version,
        reason: "user_choice",
      },
    ],
  };
}

test.describe("Channel tasks board", () => {
  test("runtime assignee roster refreshes across added, removed, disabled, hidden, and restored agents", async ({ page }) => {
    test.setTimeout(240_000);

    const { token, principal } = await login(page);
    await ensureKanbanPluginEnabled(page, token);

    const runId = Date.now();
    const noopWebhook = await startNoopAgentWebhook();
    try {
      const operator = await provisionAgent(
        page,
        token,
        uniqueName("ct-roster-operator"),
        { driver: "webhook_agent", webhookUrl: noopWebhook.url },
      );
      // These target agents intentionally have no runtime binding: this test
      // only mentions the operator, so targets are roster candidates, not turn recipients.
      const addedAgent = await createAgent(
        page,
        token,
        principal.id,
        uniqueName("ct-roster-added"),
        principal.workspace_id,
      );
      const restoredAgent = await createAgent(
        page,
        token,
        principal.id,
        uniqueName("ct-roster-restored"),
        principal.workspace_id,
      );
      const disabledAgent = await createAgent(
        page,
        token,
        principal.id,
        uniqueName("ct-roster-disabled"),
        principal.workspace_id,
      );
      const internalAgent = await createAgent(
        page,
        token,
        principal.id,
        uniqueName("ct-roster-internal"),
        principal.workspace_id,
        "internal",
      );
      const group = await createGroup(
        page,
        token,
        principal.id,
        uniqueName("ct-roster-group"),
        [
          operator.agentId,
          restoredAgent.principal.id,
          disabledAgent.principal.id,
          internalAgent.principal.id,
        ],
      );
      await setConversationCoordinatorForTest(group.id, operator.agentId);
      const taskKey = `ROSTER-${runId}`;
      const taskTitle = `Runtime roster refresh matrix ${runId}`;
      const createIdempotencyKey = `roster-create-${runId}`;

      const baselineMessages = await getMessages(page, token, principal.id, group.id, 20);
      const baselineMaxServerSeq = Math.max(
        0,
        ...baselineMessages.map((message) => message.server_seq),
      );

      const captureRuntimeRoster = async (
        label: string,
        expected: { present?: string[]; absent?: string[] },
      ) => {
        const marker = `roster freshness ${label} ${runId}`;
        const message = await sendMessage(
          page,
          token,
          principal.id,
          group.id,
          `@${operator.agentName} ${marker}`,
        );
        const prompt = await waitForAgentPromptForTest(
          page,
          operator.agentId,
          group.id,
          message.id,
        );
        const roster = parseRuntimeRoster(prompt);
        expectRuntimeRoster(roster, label, expected);
        return roster;
      };

      await captureRuntimeRoster("initial", {
        present: [operator.agentId, restoredAgent.principal.id, disabledAgent.principal.id],
        absent: [addedAgent.principal.id, internalAgent.principal.id],
      });

      await writeAgentOutboxCommand(operator.workspacePath, {
        type: "task_create",
        conversation_id: group.id,
        task_key: taskKey,
        title: taskTitle,
        assignee_principal_id: operator.agentId,
        idempotency_key: createIdempotencyKey,
      });
      const createResult = await waitForOutboxCommandResult(
        page,
        operator.workspacePath,
        (result) =>
          result.command_type === "task_create" &&
          result.idempotency_key === createIdempotencyKey,
      );
      expect(createResult.ok).toBe(true);
      expect(createResult.task_key).toBe(taskKey);
      expect(createResult.task_id).toBeTruthy();

      const createdTask = await waitForChannelTask(
        page,
        token,
        group.id,
        (task) =>
          task.task_key === taskKey &&
          task.title === taskTitle &&
          task.assignee_principal_id === operator.agentId &&
          task.source_kind === "agent",
      );
      let currentAssigneeId = operator.agentId;
      let currentVersion = createdTask.version;

      const expectCurrentTaskUnchanged = async (label: string) => {
        const tasks = await fetchChannelTasks(page, token, group.id);
        const task = tasks.find((candidate) => candidate.task_key === taskKey);
        expect(task, `${label}: task should still exist`).toBeTruthy();
        expect(task?.assignee_principal_id, `${label}: failed transfer must not mutate assignee`).toBe(currentAssigneeId);
        expect(task?.version, `${label}: failed transfer must not mutate task version`).toBe(currentVersion);
        expect(tasks.filter((candidate) => candidate.task_key === taskKey)).toHaveLength(1);
      };

      const transferTask = async (
        label: string,
        assigneePayload: { assignee?: string; assignee_principal_id?: string },
        expected: { ok: true; assigneePrincipalId: string } | { ok: false },
      ) => {
        const idempotencyKey = `roster-${label}-${runId}`;
        await writeAgentOutboxCommand(operator.workspacePath, {
          type: "task_transfer",
          conversation_id: group.id,
          task_key: taskKey,
          idempotency_key: idempotencyKey,
          ...assigneePayload,
        });
        const result = await waitForOutboxCommandResult(
          page,
          operator.workspacePath,
          (candidate) =>
            candidate.command_type === "task_transfer" &&
            candidate.idempotency_key === idempotencyKey,
        );
        expect(result.command_type).toBe("task_transfer");
        expect(result.ok, label).toBe(expected.ok);
        expect(result.task_key, label).toBe(taskKey);
        if (!expected.ok) {
          expect(result.error_code, label).toBe("invalid_assignee");
          expect(result.message, label).toContain("Could not resolve task assignee");
          await expectCurrentTaskUnchanged(label);
          return;
        }

        const task = await waitForChannelTask(
          page,
          token,
          group.id,
          (candidate) =>
            candidate.task_key === taskKey &&
            candidate.assignee_principal_id === expected.assigneePrincipalId &&
            candidate.version > currentVersion,
        );
        expect(result.ok, label).toBe(true);
        expect(result.task_id, label).toBe(task.task_id);
        currentAssigneeId = expected.assigneePrincipalId;
        currentVersion = task.version;
      };

      await captureRuntimeRoster("added-before-membership", {
        present: [operator.agentId],
        absent: [addedAgent.principal.id, internalAgent.principal.id],
      });
      await transferTask(
        "added-before-membership",
        { assignee: addedAgent.principal.name },
        { ok: false },
      );
      await addGroupMember(page, token, group.id, principal.id, [addedAgent.principal.id]);
      await captureRuntimeRoster("added-after-membership", {
        present: [operator.agentId, addedAgent.principal.id],
        absent: [internalAgent.principal.id],
      });
      await transferTask(
        "added-after-membership",
        { assignee: addedAgent.principal.name },
        { ok: true, assigneePrincipalId: addedAgent.principal.id },
      );
      await transferTask(
        "back-from-added",
        { assignee_principal_id: operator.agentId },
        { ok: true, assigneePrincipalId: operator.agentId },
      );

      await captureRuntimeRoster("removed-before-removal", {
        present: [operator.agentId, restoredAgent.principal.id],
        absent: [internalAgent.principal.id],
      });
      await transferTask(
        "restored-before-removal",
        { assignee: restoredAgent.principal.name },
        { ok: true, assigneePrincipalId: restoredAgent.principal.id },
      );
      await transferTask(
        "back-before-removal",
        { assignee_principal_id: operator.agentId },
        { ok: true, assigneePrincipalId: operator.agentId },
      );
      await removeGroupMember(page, token, group.id, principal.id, restoredAgent.principal.id);
      await captureRuntimeRoster("removed-after-removal", {
        present: [operator.agentId],
        absent: [restoredAgent.principal.id, internalAgent.principal.id],
      });
      await transferTask(
        "removed-stale-id",
        { assignee_principal_id: restoredAgent.principal.id },
        { ok: false },
      );
      await addGroupMember(page, token, group.id, principal.id, [restoredAgent.principal.id]);
      await captureRuntimeRoster("removed-restored", {
        present: [operator.agentId, restoredAgent.principal.id],
        absent: [internalAgent.principal.id],
      });
      await transferTask(
        "removed-restored-by-name",
        { assignee: restoredAgent.principal.name },
        { ok: true, assigneePrincipalId: restoredAgent.principal.id },
      );
      await transferTask(
        "back-from-removed-restored",
        { assignee_principal_id: operator.agentId },
        { ok: true, assigneePrincipalId: operator.agentId },
      );

      await captureRuntimeRoster("disabled-before-disable", {
        present: [operator.agentId, disabledAgent.principal.id],
        absent: [internalAgent.principal.id],
      });
      await transferTask(
        "disabled-before-disable",
        { assignee: disabledAgent.principal.name },
        { ok: true, assigneePrincipalId: disabledAgent.principal.id },
      );
      await transferTask(
        "back-before-disable",
        { assignee_principal_id: operator.agentId },
        { ok: true, assigneePrincipalId: operator.agentId },
      );
      const disabledPrincipal = await disablePrincipal(
        page,
        token,
        principal.id,
        disabledAgent.principal.id,
      );
      expect(disabledPrincipal.disabled).toBe(true);
      await captureRuntimeRoster("disabled-after-disable", {
        present: [operator.agentId],
        absent: [disabledAgent.principal.id, internalAgent.principal.id],
      });
      await transferTask(
        "disabled-stale-id",
        { assignee_principal_id: disabledAgent.principal.id },
        { ok: false },
      );
      await setPrincipalDisabledForTest(disabledAgent.principal.id, false);
      await captureRuntimeRoster("disabled-restored", {
        present: [operator.agentId, disabledAgent.principal.id],
        absent: [internalAgent.principal.id],
      });
      await transferTask(
        "disabled-restored-by-name",
        { assignee: disabledAgent.principal.name },
        { ok: true, assigneePrincipalId: disabledAgent.principal.id },
      );
      await transferTask(
        "back-from-disabled-restored",
        { assignee_principal_id: operator.agentId },
        { ok: true, assigneePrincipalId: operator.agentId },
      );

      await captureRuntimeRoster("internal-hidden", {
        present: [operator.agentId],
        absent: [internalAgent.principal.id],
      });
      await transferTask(
        "internal-hidden-name",
        { assignee: internalAgent.principal.name },
        { ok: false },
      );
      await transferTask(
        "internal-hidden-id",
        { assignee_principal_id: internalAgent.principal.id },
        { ok: false },
      );
      await setPrincipalChannelVisibilityForTest(internalAgent.principal.id, "visible");
      await captureRuntimeRoster("internal-restored", {
        present: [operator.agentId, internalAgent.principal.id],
      });
      await transferTask(
        "internal-restored-by-name",
        { assignee: internalAgent.principal.name },
        { ok: true, assigneePrincipalId: internalAgent.principal.id },
      );
      await transferTask(
        "back-from-internal-restored",
        { assignee_principal_id: operator.agentId },
        { ok: true, assigneePrincipalId: operator.agentId },
      );
      await setPrincipalChannelVisibilityForTest(internalAgent.principal.id, "internal");
      await captureRuntimeRoster("internal-hidden-again", {
        present: [operator.agentId],
        absent: [internalAgent.principal.id],
      });
      await transferTask(
        "internal-hidden-again-id",
        { assignee_principal_id: internalAgent.principal.id },
        { ok: false },
      );

      const finalTasks = await fetchChannelTasks(page, token, group.id);
      expect(finalTasks.filter((task) => task.task_key === taskKey)).toHaveLength(1);
      await expectNoAgentChatMessages(
        page,
        token,
        principal.id,
        group.id,
        baselineMaxServerSeq,
        [
          operator.agentId,
          addedAgent.principal.id,
          restoredAgent.principal.id,
          disabledAgent.principal.id,
          internalAgent.principal.id,
        ],
        10_000,
      );
    } finally {
      await closeServer(noopWebhook.server);
    }
  });

  test("visible group webhook agent command path handles multi-step task commands without chat noise", async ({ page }) => {
    test.setTimeout(150_000);

    const { token, principal } = await login(page);
    await ensureKanbanPluginEnabled(page, token);

    const reviewerAgent = await provisionAgent(page, token, uniqueName("ct-agent-reviewer"));
    const runId = Date.now();
    const taskKey = `E2E-${runId}`;
    const taskTitle = `Billing cleanup implementation ${runId}`;
    const webhookConfig: {
      workspacePath?: string;
      conversationId?: string;
      ownerAgentId?: string;
    } = {};

    const ownerName = uniqueName("ct-agent-owner");
    const webhook = await startVisibleAgentTaskWebhook(
      webhookConfig,
      reviewerAgent.agentId,
      taskKey,
      taskTitle,
      runId,
    );
    const ownerAgent = await provisionAgent(
      page,
      token,
      ownerName,
      { driver: "webhook_agent", webhookUrl: webhook.url },
    );
    const group = await createGroup(
      page,
      token,
      principal.id,
      uniqueName("ct-agent-work-group"),
      [ownerAgent.agentId, reviewerAgent.agentId],
    );
    webhookConfig.workspacePath = ownerAgent.workspacePath;
    webhookConfig.conversationId = group.id;
    webhookConfig.ownerAgentId = ownerAgent.agentId;

    const createKey = `agent-multistep-create-${runId}`;
    const updateKey = `agent-multistep-update-${runId}`;
    const transferKey = `agent-multistep-transfer-${runId}`;

    try {
      const baselineMessages = await getMessages(page, token, principal.id, group.id, 20);
      const baselineMaxServerSeq = Math.max(
        0,
        ...baselineMessages.map((message) => message.server_seq),
      );

      await sendMessage(
        page,
        token,
        principal.id,
        group.id,
        `@${ownerAgent.agentName} multi-step board request ${runId}: please investigate, implement, verify, and hand off review for billing cleanup.`,
      );
      await waitForWebhookCommandEmission(page, webhook.emittedCount, 1, 15_000, webhook.serverErrors);
      expect(webhook.appMentionCount()).toBe(1);

      const createResult = await waitForOutboxCommandResult(
        page,
        ownerAgent.workspacePath,
        (result) =>
          result.command_type === "task_create" &&
          result.idempotency_key === createKey &&
          result.ok === true,
      );
      expect(createResult.task_key).toBe(taskKey);
      expect(createResult.task_id).toBeTruthy();

      const createdTask = await waitForChannelTask(
        page,
        token,
        group.id,
        (task) =>
          task.task_id === createResult.task_id &&
          task.task_key === taskKey &&
          task.title === taskTitle &&
          task.assignee_principal_id === ownerAgent.agentId &&
          task.source_kind === "agent",
      );
      const tasksAfterCreate = await fetchChannelTasks(page, token, group.id);
      expect(tasksAfterCreate.filter((task) => task.task_id === createdTask.task_id)).toHaveLength(1);
      expect(tasksAfterCreate.filter((task) => task.title === taskTitle)).toHaveLength(1);
      await expectNoAgentChatMessages(
        page,
        token,
        principal.id,
        group.id,
        baselineMaxServerSeq,
        [ownerAgent.agentId, reviewerAgent.agentId],
      );

      await gotoGroupTasksTab(page, group.id);
      const board = page.locator(".channel-task-board");
      const todoColumn = board.locator(".channel-task-column", {
        has: page.locator(".channel-task-column-header", { hasText: "Todo" }),
      });
      const inProgressColumn = board.locator(".channel-task-column", {
        has: page.locator(".channel-task-column-header", { hasText: "In Progress" }),
      });
      const columnFor = (label: string) =>
        board.locator(".channel-task-column", {
          has: page.locator(".channel-task-column-header", { hasText: label }),
        });
      const card = board.locator(".channel-task-card", { hasText: taskTitle }).first();
      await expect(todoColumn.locator(".channel-task-card", { hasText: taskTitle })).toHaveCount(1, {
        timeout: 45_000,
      });
      for (const label of ["In Progress", "Blocked", "In Review", "Done"]) {
        expect(await columnFor(label).locator(".channel-task-card", { hasText: taskTitle }).count()).toBe(0);
      }
      await expect(card.locator(".channel-task-meta")).toContainText(ownerAgent.agentName);

      await sendMessage(
        page,
        token,
        principal.id,
        group.id,
        `@${ownerAgent.agentName} mark billing cleanup implementation in progress ${runId}.`,
      );
      await waitForWebhookCommandEmission(page, webhook.emittedCount, 2, 15_000, webhook.serverErrors);
      expect(webhook.appMentionCount()).toBe(2);

      const updateResult = await waitForOutboxCommandResult(
        page,
        ownerAgent.workspacePath,
        (result) =>
          result.command_type === "task_update" &&
          result.idempotency_key === updateKey &&
          result.ok === true &&
          result.task_key === taskKey,
      );
      expect(updateResult.task_id).toBe(createdTask.task_id);

      await waitForChannelTask(
        page,
        token,
        group.id,
        (task) =>
          task.task_id === createdTask.task_id &&
          task.status === "in_progress" &&
          task.version > createdTask.version,
      );
      await expect(inProgressColumn.locator(".channel-task-card", { hasText: taskTitle })).toHaveCount(1, {
        timeout: 45_000,
      });
      await expect(todoColumn.locator(".channel-task-card", { hasText: taskTitle })).toHaveCount(0);
      const tasksAfterUpdate = await fetchChannelTasks(page, token, group.id);
      expect(tasksAfterUpdate.filter((task) => task.task_id === createdTask.task_id)).toHaveLength(1);
      await expectNoAgentChatMessages(
        page,
        token,
        principal.id,
        group.id,
        baselineMaxServerSeq,
        [ownerAgent.agentId, reviewerAgent.agentId],
      );

      await sendMessage(
        page,
        token,
        principal.id,
        group.id,
        `@${ownerAgent.agentName} hand billing cleanup review to ${reviewerAgent.agentName} ${runId}.`,
      );
      await waitForWebhookCommandEmission(page, webhook.emittedCount, 3, 15_000, webhook.serverErrors);
      expect(webhook.appMentionCount()).toBe(3);

      const transferResult = await waitForOutboxCommandResult(
        page,
        ownerAgent.workspacePath,
        (result) =>
          result.command_type === "task_transfer" &&
          result.idempotency_key === transferKey &&
          result.ok === true &&
          result.task_key === taskKey,
      );
      expect(transferResult.task_id).toBe(createdTask.task_id);

      await waitForChannelTask(
        page,
        token,
        group.id,
        (task) =>
          task.task_id === createdTask.task_id &&
          task.status === "in_progress" &&
          task.assignee_principal_id === reviewerAgent.agentId,
      );
      await expect(card.locator(".channel-task-meta")).toContainText(reviewerAgent.agentName, {
        timeout: 45_000,
      });
      await expect(card.locator(".channel-task-meta")).not.toContainText(ownerAgent.agentName, {
        timeout: 15_000,
      });
      const tasksAfterTransfer = await fetchChannelTasks(page, token, group.id);
      expect(tasksAfterTransfer.filter((task) => task.task_id === createdTask.task_id)).toHaveLength(1);
      expect(webhook.serverErrors()).toEqual([]);
      await expectNoAgentChatMessages(
        page,
        token,
        principal.id,
        group.id,
        baselineMaxServerSeq,
        [ownerAgent.agentId, reviewerAgent.agentId],
      );
    } finally {
      await closeServer(webhook.server);
    }
  });

  test("visible group webhook agent distinguishes trivial replies from explicit task tracking", async ({ page }) => {
    test.setTimeout(150_000);

    const { token, principal } = await login(page);
    await ensureKanbanPluginEnabled(page, token);

    const runId = Date.now();
    const trivialReplyContent = `Trivial answer ${runId}: 4.`;
    const trackedTaskKey = `E2E-TRACK-${runId}`;
    const trackedTaskTitle = `Explicitly tracked work ${runId}`;
    const trackIdempotencyKey = `explicit-track-${runId}`;
    const webhookConfig: {
      workspacePath?: string;
      trivialGroupName?: string;
      trackedConversationId?: string;
      ownerAgentId?: string;
      trackedTaskKey?: string;
      trackedTaskTitle?: string;
      trackIdempotencyKey?: string;
    } = {
      trackedTaskKey,
      trackedTaskTitle,
      trackIdempotencyKey,
    };
    const webhook = await startTrivialRequestDecisionWebhook(
      webhookConfig,
      runId,
      trivialReplyContent,
    );
    const agent = await provisionAgent(
      page,
      token,
      uniqueName("ct-trivial-agent"),
      { driver: "webhook_agent", webhookUrl: webhook.url },
    );
    const trivialGroup = await createGroup(
      page,
      token,
      principal.id,
      uniqueName("ct-trivial-group"),
      [agent.agentId],
    );
    const trackedGroup = await createGroup(
      page,
      token,
      principal.id,
      uniqueName("ct-tracked-group"),
      [agent.agentId],
    );
    webhookConfig.workspacePath = agent.workspacePath;
    webhookConfig.trivialGroupName = trivialGroup.name;
    webhookConfig.trackedConversationId = trackedGroup.id;
    webhookConfig.ownerAgentId = agent.agentId;

    try {
      expect(await fetchChannelTasks(page, token, trivialGroup.id)).toEqual([]);
      expect(await fetchChannelTasks(page, token, trackedGroup.id)).toEqual([]);
      expect(
        (await readOutboxCommandResults(agent.workspacePath))
          .filter((result) => result.command_type === "task_create"),
      ).toEqual([]);

      await sendMessage(
        page,
        token,
        principal.id,
        trivialGroup.id,
        `@${agent.agentName} trivial one-turn request ${runId}: what is 2 + 2? Please answer briefly.`,
      );
      await waitForWebhookCommandEmission(page, webhook.emittedCount, 1, 15_000, webhook.serverErrors);
      expect(webhook.appMentionCount()).toBeGreaterThanOrEqual(1);
      expect(webhook.emittedCommandTypes()).toEqual(["send"]);

      await expect.poll(async () => {
        const messages = await getMessages(page, token, principal.id, trivialGroup.id, 50);
        return messages.some(
          (message) => message.sender_id === agent.agentId && message.content === trivialReplyContent,
        );
      }, { timeout: 45_000 }).toBe(true);

      await expectNoWorkspaceTaskCommandResultsOrConversationTasks(
        page,
        token,
        trivialGroup.id,
        agent.workspacePath,
      );
      await gotoGroupTasksTab(page, trivialGroup.id);
      const trivialBoard = page.locator(".channel-task-board");
      await expect(trivialBoard.locator(".channel-task-card")).toHaveCount(0);
      await expect(trivialBoard.locator(".channel-task-board-empty")).toContainText(/no tasks/i);

      await sendMessage(
        page,
        token,
        principal.id,
        trackedGroup.id,
        `@${agent.agentName} explicit tracking request ${runId}: please track this work on the board.`,
      );
      await waitForWebhookCommandEmission(page, webhook.emittedCount, 2, 15_000, webhook.serverErrors);
      expect(webhook.emittedCommandTypes()).toEqual(["send", "task_create"]);

      const createResult = await waitForOutboxCommandResult(
        page,
        agent.workspacePath,
        (result) =>
          result.command_type === "task_create" &&
          result.idempotency_key === trackIdempotencyKey &&
          result.ok === true,
      );
      expect(createResult.task_key).toBe(trackedTaskKey);
      expect(createResult.task_id).toBeTruthy();

      await waitForChannelTask(
        page,
        token,
        trackedGroup.id,
        (task) =>
          task.task_id === createResult.task_id &&
          task.task_key === trackedTaskKey &&
          task.title === trackedTaskTitle &&
          task.assignee_principal_id === agent.agentId &&
          task.source_kind === "agent",
      );
      expect(await fetchChannelTasks(page, token, trivialGroup.id)).toEqual([]);
      await gotoGroupTasksTab(page, trivialGroup.id);
      await expect(trivialBoard.locator(".channel-task-card")).toHaveCount(0);
      await expect(trivialBoard.locator(".channel-task-board-empty")).toContainText(/no tasks/i);

      await gotoGroupTasksTab(page, trackedGroup.id);
      const trackedBoard = page.locator(".channel-task-board");
      await expect(trackedBoard.locator(".channel-task-card", { hasText: trackedTaskTitle })).toHaveCount(1, {
        timeout: 45_000,
      });
      expect(webhook.serverErrors()).toEqual([]);
    } finally {
      await closeServer(webhook.server);
    }
  });

  test("software-development-team screenshot smoke backs assignment prose with board tasks and omits skipped optional roles", async ({ page }) => {
    test.setTimeout(180_000);

    const { token, principal } = await login(page);
    await ensureKanbanPluginEnabled(page, token);

    const runId = Date.now();
    const skippedRolePattern = /\b(frontend-engineer|qa-tester|devops-engineer|Frontend Engineer|QA Tester|DevOps Engineer)\b/i;
    const webhookConfig: {
      workspacePath?: string;
      conversationId?: string;
      groupName?: string;
    } = {};
    const operatorScript: ScreenshotSmokeCommandScript = {
      taskCreates: [],
      receiptContent: "",
    };
    const webhook = await startSoftwareTeamScreenshotSmokeWebhook(webhookConfig, operatorScript);
    try {
      const operator = await provisionAgent(
        page,
        token,
        uniqueName("project-operator"),
        { driver: "webhook_agent", webhookUrl: webhook.url },
      );
      const mission = `Ship the screenshot smoke regression ${runId}`;
      const createdJob = await createGroupProvisioningJobViaApp(page, token, {
        idempotencyKey: `software-team-smoke-${runId}`,
        companyId: principal.workspace_id,
        plan: softwareTeamSmokePlan({
          groupName: uniqueName("software-development-team"),
          mission,
          operatorAgentId: operator.agentId,
          operatorDisplayName: operator.agentName,
        }),
      });
      const completedJob = await runGroupProvisioningJobToCompletion(page, token, createdJob);
      const softwareTeamTemplate = requireSoftwareTeamTemplate();
      const templateSlotIds = softwareTeamTemplate.roleSlots.map((slot) => slot.id).sort();
      const optionalSlotIds = softwareTeamTemplate.roleSlots
        .filter((slot) => !slot.required)
        .map((slot) => slot.id)
        .sort();
      expect(completedJob.plan.groupTemplateId).toBe("software-development-team");
      expect(completedJob.plan.rolePlans.map((role) => role.slotId).sort()).toEqual(templateSlotIds);
      expect(completedJob.plan.rolePlans.filter((role) => role.action === "skip").map((role) => role.slotId).sort()).toEqual(optionalSlotIds);
      for (const slotId of optionalSlotIds) {
        expect(stepResult(completedJob, "skipped_optional_role", slotId)).toMatchObject({
          kind: "skipped_optional_role",
          roleSlotId: slotId,
          roleTemplateId: roleTemplateIdForSoftwareTeamSlot(slotId),
        });
      }

      const group = stepResult(completedJob, "created_group");
      const backend = stepResult(completedJob, "created_agent", "backend-engineer");
      const reviewer = stepResult(completedJob, "created_agent", "code-reviewer");
      const kickoff = stepResult(completedJob, "kickoff_post");
      expect(kickoff.kickoffText).toContain("Current members:");
      expect(kickoff.kickoffText).toContain(operator.agentName);
      expect(kickoff.kickoffText).toContain(backend.agentName);
      expect(kickoff.kickoffText).toContain(reviewer.agentName);
      expect(kickoff.kickoffText).not.toMatch(skippedRolePattern);

      const smokeTasks: ScreenshotSmokeTask[] = [
        {
          taskKey: `SMOKE-${runId}-BE`,
          title: `Implement screenshot smoke backend fixture ${runId}`,
          assigneeName: backend.agentName,
          assigneePrincipalId: backend.agentId,
          idempotencyKey: `software-team-smoke-backend-${runId}`,
        },
        {
          taskKey: `SMOKE-${runId}-REVIEW`,
          title: `Review screenshot smoke regression ${runId}`,
          assigneeName: reviewer.agentName,
          assigneePrincipalId: reviewer.agentId,
          idempotencyKey: `software-team-smoke-review-${runId}`,
        },
      ];
      const receiptContent = [
        "Board tasks created:",
        `- ${smokeTasks[0].taskKey} | ${smokeTasks[0].title} | ${smokeTasks[0].assigneeName}`,
        `- ${smokeTasks[1].taskKey} | ${smokeTasks[1].title} | ${smokeTasks[1].assigneeName}`,
        "",
        "Assignments:",
        `- ${smokeTasks[0].taskKey} | ${smokeTasks[0].title} | ${smokeTasks[0].assigneeName}`,
        `- ${smokeTasks[1].taskKey} | ${smokeTasks[1].title} | ${smokeTasks[1].assigneeName}`,
      ].join("\n");
      operatorScript.taskCreates = smokeTasks;
      operatorScript.receiptContent = receiptContent;
      webhookConfig.workspacePath = operator.workspacePath;
      webhookConfig.conversationId = group.groupConversationId;
      webhookConfig.groupName = group.groupName;

      await sendMessage(
        page,
        token,
        principal.id,
        group.groupConversationId,
        `@${operator.agentName} software team screenshot smoke ${runId}: break down this Kanban-worthy regression and assign owners using the current roster only.`,
      );
      await waitForWebhookCommandEmission(page, webhook.emittedCount, smokeTasks.length + 1, 15_000, webhook.serverErrors);
      expect(webhook.appMentionCount()).toBe(1);

      const commandResults = [];
      for (const task of smokeTasks) {
        const result = await waitForOutboxCommandResult(
          page,
          operator.workspacePath,
          (candidate) =>
            candidate.command_type === "task_create" &&
            candidate.idempotency_key === task.idempotencyKey &&
            candidate.ok === true,
        );
        expect(result.task_key).toBe(task.taskKey);
        expect(result.task_id).toBeTruthy();
        commandResults.push(result);
      }

      const apiTasks = [];
      for (const task of smokeTasks) {
        const createdTask = await waitForChannelTask(
          page,
          token,
          group.groupConversationId,
          (candidate) =>
            candidate.task_key === task.taskKey &&
            candidate.title === task.title &&
            candidate.assignee_principal_id === task.assigneePrincipalId &&
            candidate.source_kind === "agent",
        );
        expect(createdTask.assignee_name).toBe(task.assigneeName);
        apiTasks.push(createdTask);
      }
      expect(apiTasks).toHaveLength(smokeTasks.length);
      expect(JSON.stringify(apiTasks)).not.toMatch(skippedRolePattern);

      const operatorReceipt = await waitForConversationMessage(
        page,
        token,
        principal.id,
        group.groupConversationId,
        (message) =>
          message.sender_id === operator.agentId &&
          message.content.includes("Board tasks created:") &&
          message.content.includes("Assignments:"),
      );
      expect(operatorReceipt.content).toBe(receiptContent);
      expect(operatorReceipt.content).not.toMatch(skippedRolePattern);
      const assignmentRows = assignmentReceiptLines(operatorReceipt.content).map((line) => {
        const [taskKey, title, assigneeName] = line.replace(/^-\s+/, "").split("|").map((part) => part.trim());
        return { taskKey, title, assigneeName };
      });
      expect(assignmentRows).toEqual(
        apiTasks.map((task) => ({
          taskKey: task.task_key,
          title: task.title,
          assigneeName: task.assignee_name,
        })),
      );
      for (const row of assignmentRows) {
        const apiTask = apiTasks.find((task) => task.task_key === row.taskKey);
        const commandResult = commandResults.find((result) =>
          result.command_type === "task_create" &&
          result.ok === true &&
          result.task_key === row.taskKey,
        );
        expect(
          commandResult,
          `assignment line should be backed by a successful same-turn task_create: ${JSON.stringify(row)}`,
        ).toBeTruthy();
        expect(commandResult?.task_id).toBe(apiTask?.task_id);
      }

      await gotoGroupTasksTab(page, group.groupConversationId);
      const board = page.locator(".channel-task-board");
      for (const task of smokeTasks) {
        const card = board.locator(".channel-task-card", { hasText: task.title }).first();
        await expect(card).toBeVisible({ timeout: 45_000 });
        await expect(card.locator(".channel-task-meta")).toContainText(task.assigneeName);
        await expect(card).not.toContainText(skippedRolePattern);
        const assigneeOptions = await card.locator("select").nth(1).locator("option").allTextContents();
        expect(assigneeOptions.join("\n")).not.toMatch(skippedRolePattern);
        expect(assigneeOptions).toEqual(expect.arrayContaining(smokeTasks.map((candidate) => candidate.assigneeName)));
      }
      const allCards = board.locator(".channel-task-card");
      await expect(allCards).toHaveCount(smokeTasks.length);
      await expect(board).not.toContainText(skippedRolePattern);
    } finally {
      await closeServer(webhook.server);
    }
  });

  test("group conversation exposes Chat and Tasks tabs with the kanban columns", async ({ page }) => {
    const { token, principal } = await login(page);
    await ensureKanbanPluginEnabled(page, token);

    const agent = await provisionAgent(page, token, uniqueName("ct-agent"));
    const group = await createGroup(
      page,
      token,
      principal.id,
      uniqueName("ct-group"),
      [agent.agentId],
    );

    await openConversation(page, group.id);

    const chatTab = page.getByRole("tab", { name: "Chat" });
    const tasksTab = page.getByRole("tab", { name: "Tasks" });
    await expect(chatTab).toBeVisible({ timeout: 15_000 });
    await expect(tasksTab).toBeVisible();
    await expect(chatTab).toHaveAttribute("aria-selected", "true");

    await tasksTab.click();
    await expect(tasksTab).toHaveAttribute("aria-selected", "true");

    const board = page.locator(".channel-task-board");
    await expect(board).toBeVisible({ timeout: 15_000 });

    for (const label of ["Todo", "In Progress", "Blocked", "In Review", "Done"]) {
      await expect(
        board.locator(".channel-task-column-header", { hasText: label }),
      ).toBeVisible();
    }

    await expect(board.locator(".channel-task-board-empty")).toContainText(/no tasks/i);

    await chatTab.click();
    await expect(chatTab).toHaveAttribute("aria-selected", "true");
    await expect(page.locator(".channel-task-board")).toBeHidden();
  });

  test("create task from message requires an assignee and lands on the board", async ({ page }) => {
    const { token, principal } = await login(page);
    await ensureKanbanPluginEnabled(page, token);

    const agent = await provisionAgent(page, token, uniqueName("ct-cfm-agent"));
    const group = await createGroup(
      page,
      token,
      principal.id,
      uniqueName("ct-cfm-group"),
      [agent.agentId],
    );

    const messageContent = `please track this work ${Date.now()}`;
    await sendMessage(page, token, principal.id, group.id, messageContent);

    await openConversation(page, group.id);

    const messageRow = page.locator(".msg-group").filter({ hasText: messageContent }).first();
    await expect(messageRow).toBeVisible({ timeout: 15_000 });
    await messageRow.hover();
    await messageRow.locator('[aria-label="Message actions"]').click();
    await page.getByRole("menuitem", { name: "Create task" }).click();

    const modal = page.locator(".channel-task-create-modal");
    await expect(modal).toBeVisible({ timeout: 5000 });

    const submit = modal.getByRole("button", { name: /^Create$/ });
    await expect(submit).toBeDisabled();

    const titleInput = modal.locator("label", { hasText: "Title" }).locator("input");
    await titleInput.fill(`Track ${messageContent}`);
    // Still disabled until assignee is picked.
    await expect(submit).toBeDisabled();

    const assigneeSelect = modal.locator("label", { hasText: "Assignee" }).locator("select");
    await assigneeSelect.selectOption({ label: agent.agentName });
    await expect(submit).toBeEnabled();

    await submit.click();
    await expect(modal).toBeHidden({ timeout: 10_000 });

    await page.getByRole("tab", { name: "Tasks" }).click();

    const card = page.locator(".channel-task-card", { hasText: `Track ${messageContent}` }).first();
    await expect(card).toBeVisible({ timeout: 15_000 });
    await expect(card.locator(".channel-task-meta")).toContainText(agent.agentName);
    // The task card title is a read-only heading; there is no editable
    // post-creation title control anywhere on the card.
    await expect(card.locator("h3")).toHaveText(`Track ${messageContent}`);
    await expect(card.locator("h3").locator("input, textarea, [contenteditable='true']")).toHaveCount(0);
    await expect(card.getByRole("button", { name: /edit title/i })).toHaveCount(0);
  });

  test("repeated create-from-message dedupes and does not mutate the existing task", async ({ page }) => {
    const { token, principal } = await login(page);
    await ensureKanbanPluginEnabled(page, token);

    const agent = await provisionAgent(page, token, uniqueName("ct-dedupe-agent"));
    const group = await createGroup(
      page,
      token,
      principal.id,
      uniqueName("ct-dedupe-group"),
      [agent.agentId],
    );

    const messageContent = `dedupe-source ${Date.now()}`;
    await sendMessage(page, token, principal.id, group.id, messageContent);

    await openConversation(page, group.id);

    async function openCreateModal() {
      const row = page.locator(".msg-group").filter({ hasText: messageContent }).first();
      await expect(row).toBeVisible({ timeout: 15_000 });
      await row.hover();
      await row.locator('[aria-label="Message actions"]').click();
      await page.getByRole("menuitem", { name: "Create task" }).click();
      return page.locator(".channel-task-create-modal");
    }

    // First create.
    let modal = await openCreateModal();
    const originalTitle = `Original ${messageContent}`;
    await modal.locator("label", { hasText: "Title" }).locator("input").fill(originalTitle);
    await modal.locator("label", { hasText: "Assignee" }).locator("select").selectOption({ label: agent.agentName });
    await modal.getByRole("button", { name: /^Create$/ }).click();
    await expect(modal).toBeHidden({ timeout: 10_000 });

    await page.getByRole("tab", { name: "Tasks" }).click();
    const board = page.locator(".channel-task-board");
    await expect(board.locator(".channel-task-card", { hasText: originalTitle })).toBeVisible({ timeout: 15_000 });
    const initialCount = await board.locator(".channel-task-card").count();

    // Second attempt with a changed title should dedupe to the same task
    // and must not mutate the existing card title.
    await page.getByRole("tab", { name: "Chat" }).click();
    modal = await openCreateModal();
    await modal.locator("label", { hasText: "Title" }).locator("input").fill(`Different ${messageContent}`);
    await modal.locator("label", { hasText: "Assignee" }).locator("select").selectOption({ label: agent.agentName });
    await modal.getByRole("button", { name: /^Create$/ }).click();
    await expect(modal).toBeHidden({ timeout: 10_000 });

    await page.getByRole("tab", { name: "Tasks" }).click();
    await expect(board.locator(".channel-task-card", { hasText: originalTitle })).toBeVisible();
    await expect(board.locator(".channel-task-card", { hasText: `Different ${messageContent}` })).toHaveCount(0);
    await expect(board.locator(".channel-task-card")).toHaveCount(initialCount);
  });

  test("status menu moves a card to the new column", async ({ page }) => {
    const { token, principal } = await login(page);
    await ensureKanbanPluginEnabled(page, token);

    const agent = await provisionAgent(page, token, uniqueName("ct-move-agent"));
    const group = await createGroup(
      page,
      token,
      principal.id,
      uniqueName("ct-move-group"),
      [agent.agentId],
    );

    const messageContent = `move-me ${Date.now()}`;
    await sendMessage(page, token, principal.id, group.id, messageContent);

    await openConversation(page, group.id);

    const row = page.locator(".msg-group").filter({ hasText: messageContent }).first();
    await expect(row).toBeVisible({ timeout: 15_000 });
    await row.hover();
    await row.locator('[aria-label="Message actions"]').click();
    await page.getByRole("menuitem", { name: "Create task" }).click();
    const modal = page.locator(".channel-task-create-modal");
    await modal.locator("label", { hasText: "Title" }).locator("input").fill(`Move ${messageContent}`);
    await modal.locator("label", { hasText: "Assignee" }).locator("select").selectOption({ label: agent.agentName });
    await modal.getByRole("button", { name: /^Create$/ }).click();
    await expect(modal).toBeHidden({ timeout: 10_000 });

    await page.getByRole("tab", { name: "Tasks" }).click();
    const board = page.locator(".channel-task-board");
    const todoColumn = board.locator(".channel-task-column", { has: page.locator(".channel-task-column-header", { hasText: "Todo" }) });
    const inProgressColumn = board.locator(".channel-task-column", { has: page.locator(".channel-task-column-header", { hasText: "In Progress" }) });
    const card = board.locator(".channel-task-card", { hasText: `Move ${messageContent}` }).first();
    await expect(card).toBeVisible({ timeout: 15_000 });
    await expect(todoColumn.locator(".channel-task-card", { hasText: `Move ${messageContent}` })).toHaveCount(1);

    const statusSelect = card.locator("select").first();
    await statusSelect.selectOption("in_progress");

    await expect(inProgressColumn.locator(".channel-task-card", { hasText: `Move ${messageContent}` })).toHaveCount(1, { timeout: 10_000 });
    await expect(todoColumn.locator(".channel-task-card", { hasText: `Move ${messageContent}` })).toHaveCount(0);

    // Also exercise the prev/next status buttons (accessible alternative to
    // the <select>) added in commit 8d6040d. Move forward to Blocked via the
    // next-status button on the now-relocated card.
    const blockedColumn = board.locator(".channel-task-column", { has: page.locator(".channel-task-column-header", { hasText: "Blocked" }) });
    const movedCard = inProgressColumn.locator(".channel-task-card", { hasText: `Move ${messageContent}` }).first();
    await movedCard.getByRole("button", { name: /Move .* to Blocked/ }).click();
    await expect(blockedColumn.locator(".channel-task-card", { hasText: `Move ${messageContent}` })).toHaveCount(1, { timeout: 10_000 });
    await expect(inProgressColumn.locator(".channel-task-card", { hasText: `Move ${messageContent}` })).toHaveCount(0);
  });

  test("assignee picker reassigns the card to a new visible assignee", async ({ page }) => {
    const { token, principal } = await login(page);
    await ensureKanbanPluginEnabled(page, token);

    const agentA = await provisionAgent(page, token, uniqueName("ct-reassign-a"));
    const agentB = await provisionAgent(page, token, uniqueName("ct-reassign-b"));
    const group = await createGroup(
      page,
      token,
      principal.id,
      uniqueName("ct-reassign-group"),
      [agentA.agentId, agentB.agentId],
    );

    const messageContent = `assign-me ${Date.now()}`;
    await sendMessage(page, token, principal.id, group.id, messageContent);

    await openConversation(page, group.id);

    const row = page.locator(".msg-group").filter({ hasText: messageContent }).first();
    await expect(row).toBeVisible({ timeout: 15_000 });
    await row.hover();
    await row.locator('[aria-label="Message actions"]').click();
    await page.getByRole("menuitem", { name: "Create task" }).click();
    const modal = page.locator(".channel-task-create-modal");
    await modal.locator("label", { hasText: "Title" }).locator("input").fill(`Reassign ${messageContent}`);
    await modal.locator("label", { hasText: "Assignee" }).locator("select").selectOption({ label: agentA.agentName });
    await modal.getByRole("button", { name: /^Create$/ }).click();
    await expect(modal).toBeHidden({ timeout: 10_000 });

    await page.getByRole("tab", { name: "Tasks" }).click();
    const card = page.locator(".channel-task-card", { hasText: `Reassign ${messageContent}` }).first();
    await expect(card).toBeVisible({ timeout: 15_000 });
    await expect(card.locator(".channel-task-meta")).toContainText(agentA.agentName);

    const assigneeSelect = card.locator("select").nth(1);
    await assigneeSelect.selectOption({ label: agentB.agentName });
    await expect(card.locator(".channel-task-meta")).toContainText(agentB.agentName, { timeout: 10_000 });
    await expect(card.locator(".channel-task-meta")).not.toContainText(agentA.agentName);
  });

  test("server rejection rolls the card back instead of corrupting local state", async ({ page }) => {
    const { token, principal } = await login(page);
    await ensureKanbanPluginEnabled(page, token);

    const agent = await provisionAgent(page, token, uniqueName("ct-reject-agent"));
    const group = await createGroup(
      page,
      token,
      principal.id,
      uniqueName("ct-reject-group"),
      [agent.agentId],
    );

    const messageContent = `reject-me ${Date.now()}`;
    await sendMessage(page, token, principal.id, group.id, messageContent);

    await openConversation(page, group.id);

    const row = page.locator(".msg-group").filter({ hasText: messageContent }).first();
    await expect(row).toBeVisible({ timeout: 15_000 });
    await row.hover();
    await row.locator('[aria-label="Message actions"]').click();
    await page.getByRole("menuitem", { name: "Create task" }).click();
    const modal = page.locator(".channel-task-create-modal");
    await modal.locator("label", { hasText: "Title" }).locator("input").fill(`Reject ${messageContent}`);
    await modal.locator("label", { hasText: "Assignee" }).locator("select").selectOption({ label: agent.agentName });
    await modal.getByRole("button", { name: /^Create$/ }).click();
    await expect(modal).toBeHidden({ timeout: 10_000 });

    await page.getByRole("tab", { name: "Tasks" }).click();
    const board = page.locator(".channel-task-board");
    const card = board.locator(".channel-task-card", { hasText: `Reject ${messageContent}` }).first();
    await expect(card).toBeVisible({ timeout: 15_000 });
    const todoColumn = board.locator(".channel-task-column", { has: page.locator(".channel-task-column-header", { hasText: "Todo" }) });
    await expect(todoColumn.locator(".channel-task-card", { hasText: `Reject ${messageContent}` })).toHaveCount(1);

    await page.route("**/api/v1/tasks/*", async (route) => {
      if (route.request().method() === "PATCH") {
        await route.fulfill({
          status: 403,
          contentType: "application/json",
          body: JSON.stringify({ error: { code: "Forbidden", detail: "test denial" } }),
        });
        return;
      }
      await route.continue();
    });

    const statusSelect = card.locator("select").first();
    await statusSelect.selectOption("in_progress");

    await expect(board.locator(".channel-task-board-toast")).toBeVisible({ timeout: 10_000 });
    await expect(board.locator(".channel-task-board-toast")).toContainText(/test denial|forbidden/i);
    await expect(todoColumn.locator(".channel-task-card", { hasText: `Reject ${messageContent}` })).toHaveCount(1);

    await page.unroute("**/api/v1/tasks/*");
  });

  test("direct-agent conversation exposes the Tasks tab and loads the board", async ({ page }) => {
    const { token, principal } = await login(page);
    await ensureKanbanPluginEnabled(page, token);

    const agent = await provisionAgent(page, token, uniqueName("ct-direct-agent"));
    const direct = await createDirectConversation(
      page,
      token,
      principal.id,
      agent.agentId,
    );

    await gotoGroupTasksTab(page, direct.id);
    const board = page.locator(".channel-task-board");
    await expect(board).toBeVisible();
    for (const label of ["Todo", "In Progress", "Blocked", "In Review", "Done"]) {
      await expect(
        board.locator(".channel-task-column-header", { hasText: label }),
      ).toBeVisible();
    }
  });

  // Human-to-human direct hiding (Tasks tab + Create task action) is covered by
  // the vitest suite — see `apps/web/lib/channel-tasks/channel-tasks.test.ts`,
  // `apps/web/components/chat/chat-app-channel-tasks.test.ts` (`does not render the
  // conversation Tasks tab for human-to-human direct conversations`), and
  // `apps/web/lib/channel-tasks/channel-task-create.test.ts` (`keeps human-to-human direct
  // conversations hidden through the caller visibility flag`). The E2E
  // equivalent would require seeding two humans inside the same workspace,
  // which the local-auth signup flow does not support (each signup provisions
  // its own workspace by design — see `workspace-isolation.spec.ts`).
});

test.describe("Channel tasks visual layout", () => {
  for (const viewport of [
    { name: "desktop", width: 1440, height: 900 },
    { name: "mobile", width: 390, height: 844 },
  ]) {
    test(`board layout holds at ${viewport.name} width (${viewport.width}x${viewport.height})`, async ({ page }) => {
      const { token, principal } = await login(page);
    await ensureKanbanPluginEnabled(page, token);

      await page.setViewportSize({ width: viewport.width, height: viewport.height });
      const agent = await provisionAgent(page, token, uniqueName(`ct-vis-${viewport.name}-agent`));
      const group = await createGroup(
        page,
        token,
        principal.id,
        uniqueName(`ct-vis-${viewport.name}-group`),
        [agent.agentId],
      );

      // Seed a long-content message + task via the API. The card-creation UI
      // flow (message actions menu) is already covered by the desktop board
      // tests above; this spec is specifically about how the rendered board
      // behaves at each viewport, so we skip the responsive UI flake by
      // creating the task directly.
      const longTitleMessage = "A".repeat(120);
      const message = await sendMessage(page, token, principal.id, group.id, longTitleMessage);
      const taskRes = await page.request.post(
        `${API_BASE}/v1/conversations/${group.id}/tasks/from-message`,
        {
          headers: { Authorization: `Bearer ${token}` },
          data: {
            message_id: message.id,
            title: `Long ${longTitleMessage}`.slice(0, 120),
            assignee_principal_id: agent.agentId,
          },
        },
      );
      expect(
        taskRes.ok(),
        `tasks/from-message -> ${taskRes.status()}: ${await taskRes.text().catch(() => "")}`,
      ).toBeTruthy();

      await openConversation(page, group.id);
      await page.getByRole("tab", { name: "Tasks" }).click();
      const board = page.locator(".channel-task-board");
      await expect(board).toBeVisible({ timeout: 15_000 });
      const card = board.locator(".channel-task-card").first();
      await expect(card).toBeVisible();

      // Board container is rendered with non-zero height and starts at or
      // below the viewport top. Full no-vertical-overflow
      // (y + height <= viewport.height) is intentionally NOT asserted here —
      // the responsive board can extend below the fold and rely on internal
      // column scroll; the screenshot below (written under
      // tests/screenshots/, gitignored) is the local artifact for visual
      // layout review.
      const boardBox = await board.boundingBox();
      expect(boardBox).not.toBeNull();
      expect(boardBox!.y).toBeGreaterThanOrEqual(0);
      expect(boardBox!.height).toBeGreaterThan(0);

      // No card should overflow its column horizontally.
      const cardBox = await card.boundingBox();
      const columnBox = await card.locator("xpath=ancestor::div[contains(@class,'channel-task-column')][1]").boundingBox();
      expect(cardBox).not.toBeNull();
      expect(columnBox).not.toBeNull();
      expect(cardBox!.x + cardBox!.width).toBeLessThanOrEqual(columnBox!.x + columnBox!.width + 1);
      expect(cardBox!.width).toBeLessThanOrEqual(columnBox!.width + 1);

      // Status select stays inside the card.
      const statusSelect = card.locator("select").first();
      const statusBox = await statusSelect.boundingBox();
      expect(statusBox).not.toBeNull();
      expect(statusBox!.width).toBeLessThanOrEqual(cardBox!.width + 1);

      // Tabs row stays visible at this viewport (or, on mobile, the tablist
      // is at least still rendered above the board even if compressed).
      await expect(page.getByRole("tab", { name: "Chat" })).toBeVisible();
      await expect(page.getByRole("tab", { name: "Tasks" })).toBeVisible();

      const screenshotPath = `tests/screenshots/channel-task-board-${viewport.name}.png`;
      await page.screenshot({ path: screenshotPath, fullPage: false });
    });
  }
});
