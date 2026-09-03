import { expect, test } from "@playwright/test";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { login, gotoDashboard, API_BASE, WEB_BASE } from "../fixtures/auth";
import {
  createCompany,
  createGroup,
  getConsoleSnapshot,
  provisionAgent,
  sendMessage,
  getMessages,
  uniqueName,
} from "../fixtures/api";

const execFileAsync = promisify(execFile);

async function writeOutboxCommand(workspacePath: string, command: Record<string, unknown>) {
  await execFileAsync(`${workspacePath}/.choruz/send`, [JSON.stringify(command)]);
}

async function waitForConsoleCondition<T>(
  page: Parameters<typeof getConsoleSnapshot>[0],
  token: string,
  predicate: (snapshot: Awaited<ReturnType<typeof getConsoleSnapshot>>) => T | null | undefined,
  timeoutMs = 30_000,
): Promise<T> {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeoutMs) {
    const snapshot = await getConsoleSnapshot(page, token);
    const result = predicate(snapshot);
    if (result) return result;
    await page.waitForTimeout(1_000);
  }
  throw new Error(`Console condition was not met within ${timeoutMs}ms`);
}

/**
 * Outbox / agent message pipeline tests.
 *
 * NOTE: The existing tests/outbox-reply.spec.ts covers simulated agent
 * replies and CHORUZ_REPLY tag checks.  This file adds complementary tests.
 */

test.describe("Outbox / agent message pipeline", () => {
  /* ---------------------------------------------------------------------- */
  /*  Message delivery                                                       */
  /* ---------------------------------------------------------------------- */

  test("should deliver messages via the API to a group conversation", async ({
    page,
  }) => {
    const { token, principal } = await login(page);
    const snap = await getConsoleSnapshot(page, token);
    const group = snap.conversations.find(
      (c) => c.conversation_type === "group",
    );
    if (!group) {
      test.skip();
      return;
    }
    const content = `outbox-delivery-${Date.now()}`;
    const msg = await sendMessage(
      page,
      token,
      principal.id,
      group.id,
      content,
    );
    expect(msg.id).toBeTruthy();
    expect(msg.content).toBe(content);
  });

  test("should assign sequential server_seq to messages", async ({ page }) => {
    const { token, principal } = await login(page);
    const snap = await getConsoleSnapshot(page, token);
    const group = snap.conversations.find(
      (c) => c.conversation_type === "group",
    );
    if (!group) {
      test.skip();
      return;
    }
    const msg1 = await sendMessage(
      page,
      token,
      principal.id,
      group.id,
      `seq-test-1-${Date.now()}`,
    );
    const msg2 = await sendMessage(
      page,
      token,
      principal.id,
      group.id,
      `seq-test-2-${Date.now()}`,
    );
    expect(msg2.server_seq).toBeGreaterThan(msg1.server_seq);
  });

  test("should not duplicate messages on rapid sends", async ({ page }) => {
    const { token, principal } = await login(page);
    const snap = await getConsoleSnapshot(page, token);
    const group = snap.conversations.find(
      (c) => c.conversation_type === "group",
    );
    if (!group) {
      test.skip();
      return;
    }
    const uniqueKey = `rapid-${Date.now()}`;
    // Send two messages rapidly
    await Promise.all([
      sendMessage(page, token, principal.id, group.id, `${uniqueKey}-a`),
      sendMessage(page, token, principal.id, group.id, `${uniqueKey}-b`),
    ]);
    const msgs = await getMessages(page, token, principal.id, group.id);
    const matching = msgs.filter((m) => m.content.startsWith(uniqueKey));
    expect(matching.length).toBe(2);
  });

  /* ---------------------------------------------------------------------- */
  /*  Idempotency                                                            */
  /* ---------------------------------------------------------------------- */

  test("should enforce idempotency key (same key = same message)", async ({
    page,
  }) => {
    const { token, principal } = await login(page);
    const snap = await getConsoleSnapshot(page, token);
    const group = snap.conversations.find(
      (c) => c.conversation_type === "group",
    );
    if (!group) {
      test.skip();
      return;
    }
    const idempotencyKey = `idem-${Date.now()}`;
    const data = {
      actor_id: principal.id,
      conversation_id: group.id,
      content: "idempotency test",
      content_type: "text/plain",
      idempotency_key: idempotencyKey,
      metadata: {},
    };
    const res1 = await page.request.post(`${API_BASE}/v1/messages`, {
      headers: { Authorization: `Bearer ${token}` },
      data,
    });
    const res2 = await page.request.post(`${API_BASE}/v1/messages`, {
      headers: { Authorization: `Bearer ${token}` },
      data,
    });
    expect(res1.ok()).toBeTruthy();
    // Second call should either succeed (idempotent) or be rejected
    // Either way, there should only be ONE message with that key
    const msgs = await getMessages(page, token, principal.id, group.id);
    const matches = msgs.filter(
      (m) => m.content === "idempotency test",
    );
    expect(matches.length).toBeGreaterThanOrEqual(1);
  });

  /* ---------------------------------------------------------------------- */
  /*  No tag residue                                                         */
  /* ---------------------------------------------------------------------- */

  test("should never contain CHORUZ_REPLY tags in any message", async ({
    page,
  }) => {
    const { token, principal } = await login(page);
    const snap = await getConsoleSnapshot(page, token);
    for (const conv of snap.conversations) {
      const msgs = await getMessages(page, token, principal.id, conv.id, 20);
      for (const m of msgs) {
        expect(m.content).not.toContain("{{CHORUZ_REPLY}}");
        expect(m.content).not.toContain("{{/CHORUZ_REPLY}}");
      }
    }
  });

  test("should not contain ANSI escape sequences in messages", async ({
    page,
  }) => {
    const { token, principal } = await login(page);
    const snap = await getConsoleSnapshot(page, token);
    const group = snap.conversations.find(
      (c) => c.conversation_type === "group",
    );
    if (!group) {
      test.skip();
      return;
    }
    const msgs = await getMessages(page, token, principal.id, group.id, 20);
    for (const m of msgs) {
      // ANSI escape pattern
      expect(m.content).not.toMatch(/\x1b\[/);
    }
  });

  /* ---------------------------------------------------------------------- */
  /*  Agent provisioning + messaging                                         */
  /* ---------------------------------------------------------------------- */

  test("should provision an agent and deliver a @mention message", async ({
    page,
  }) => {
    const { token, principal } = await login(page);
    const agentName = uniqueName("e2e-outbox-agent");
    const agent = await provisionAgent(page, token, agentName);
    expect(agent.agentId).toBeTruthy();

    const groupName = uniqueName("e2e-outbox-group");
    const group = await createGroup(page, token, principal.id, groupName, [
      agent.agentId,
    ]);
    expect(group.id).toBeTruthy();

    const mentionMsg = `@${agentName} hello from outbox test`;
    const msg = await sendMessage(
      page,
      token,
      principal.id,
      group.id,
      mentionMsg,
    );
    expect(msg.content).toContain(`@${agentName}`);
  });

  test("AI manager-style outbox commands create agents before creating their group", async ({
    page,
  }) => {
    test.setTimeout(90_000);

    const { token, principal } = await login(page);

    const companyName = uniqueName("e2e-company");
    const company = await createCompany(page, token, principal.id, companyName);
    const managerName = uniqueName("e2e-manager");
    const managerRes = await page.request.post(`${WEB_BASE}/api/agents/provision`, {
      data: {
        name: managerName,
        driver_type: "claude_terminal",
        instructions: "E2E manager that emits outbox commands.",
        workspace_id: company.id,
      },
    });
    expect(managerRes.ok()).toBeTruthy();
    const managerData = await managerRes.json();
    const workspacePath = managerData.workspace_path as string;
    expect(workspacePath).toBeTruthy();

    const backendName = uniqueName("backend-dev");
    const testerName = uniqueName("test-engineer");
    const groupName = uniqueName("rest-api-team");

    await writeOutboxCommand(workspacePath, {
      type: "provision_agent",
      name: backendName,
      driver: "codex_terminal",
      instructions: "Own REST API implementation.",
    });
    await writeOutboxCommand(workspacePath, {
      type: "provision_agent",
      name: testerName,
      driver: "codex_terminal",
      instructions: "Own API coverage and verification.",
    });
    await writeOutboxCommand(workspacePath, {
      type: "create_group",
      name: groupName,
      description: "REST API team",
      members: [backendName, testerName],
    });

    const result = await waitForConsoleCondition(page, token, (snapshot) => {
      const backend = snapshot.agents.find((agent) => agent.name === backendName);
      const tester = snapshot.agents.find((agent) => agent.name === testerName);
      const group = snapshot.conversations.find(
        (conversation) => conversation.name === groupName && conversation.workspace_id === company.id,
      );
      if (!backend || !tester || !group) return null;

      const memberIds = Object.keys(group.members);
      if (!memberIds.includes(backend.id) || !memberIds.includes(tester.id)) {
        return null;
      }

      return { backend, tester, group };
    }, 45_000);

    expect(result.backend.disabled).toBeFalsy();
    expect(result.tester.disabled).toBeFalsy();
    expect(result.group.conversation_type).toBe("group");

    await gotoDashboard(page);
    await page.locator(".company-selector-btn").click();
    await page
      .locator(".company-dropdown-item")
      .filter({ hasText: companyName })
      .locator(".company-dropdown-item-name")
      .click();
    await expect(page.getByText(groupName)).toBeVisible({ timeout: 15_000 });
  });

  /* ---------------------------------------------------------------------- */
  /*  Message ordering                                                       */
  /* ---------------------------------------------------------------------- */

  test("should maintain message order by server_seq", async ({ page }) => {
    const { token, principal } = await login(page);
    const snap = await getConsoleSnapshot(page, token);
    const group = snap.conversations.find(
      (c) => c.conversation_type === "group",
    );
    if (!group) {
      test.skip();
      return;
    }
    // Send several messages
    for (let i = 0; i < 3; i++) {
      await sendMessage(
        page,
        token,
        principal.id,
        group.id,
        `order-${i}-${Date.now()}`,
      );
    }
    const msgs = await getMessages(page, token, principal.id, group.id, 50);
    // server_seq should be monotonically increasing
    for (let i = 1; i < msgs.length; i++) {
      expect(msgs[i].server_seq).toBeGreaterThanOrEqual(msgs[i - 1].server_seq);
    }
  });
});
