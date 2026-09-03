import { expect, test } from "@playwright/test";
import { API_BASE, login, gotoDashboard } from "../fixtures/auth";
import {
  getConsoleSnapshot,
  provisionAgent,
  createGroup,
  addGroupMember,
  uniqueName,
} from "../fixtures/api";

test.describe("Agent management", () => {
  /* ---------------------------------------------------------------------- */
  /*  Create agent modal                                                     */
  /* ---------------------------------------------------------------------- */

  test("should open the create agent modal from sidebar menu", async ({
    page,
  }) => {
    await login(page);
    await gotoDashboard(page);
    const actionsBtn = page.locator('[aria-label="Actions menu"]');
    await actionsBtn.click();
    await page.getByText("Create Agent").click();
    // Modal should appear
    await expect(
      page.locator(".modal-overlay, .modal-backdrop").first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("should show agent name input in create modal", async ({ page }) => {
    await login(page);
    await gotoDashboard(page);
    const actionsBtn = page.locator('[aria-label="Actions menu"]');
    await actionsBtn.click();
    await page.getByText("Create Agent").click();
    await page.waitForTimeout(500);
    // Look for agent name input field
    const nameInput = page.locator('input[placeholder*="agent"], input[name="agent-name"], input[type="text"]').first();
    await expect(nameInput).toBeVisible({ timeout: 5000 });
  });

  test("should show driver type selector in create modal", async ({
    page,
  }) => {
    await login(page);
    await gotoDashboard(page);
    const actionsBtn = page.locator('[aria-label="Actions menu"]');
    await actionsBtn.click();
    await page.getByText("Create Agent").click();
    await page.waitForTimeout(500);
    // Look for driver select (Claude Code, Codex, etc.)
    const selector = page.locator("select, .driver-select, .driver-radio").first();
    await expect(selector).toBeVisible({ timeout: 5000 });
  });

  /* ---------------------------------------------------------------------- */
  /*  Provision agent via API                                                */
  /* ---------------------------------------------------------------------- */

  test("should provision an agent through the real creation API", async ({ page }) => {
    await login(page);
    const agentName = uniqueName("e2e-agent");
    const result = await provisionAgent(page, "", agentName);
    expect(result.agentId).toBeTruthy();
    expect(result.agentName).toBe(agentName);
    expect(result.secret).toBeTruthy();
  });

  test("should list the provisioned agent in the console snapshot", async ({
    page,
  }) => {
    const { token } = await login(page);
    // Provision our own agent instead of assuming one exists: another
    // worker may be running against an empty workspace at the same time.
    const agentName = uniqueName("e2e-snapshot-agent");
    const { agentId } = await provisionAgent(page, token, agentName);
    const snap = await getConsoleSnapshot(page, token);
    const listed = snap.agents.find((agent) => agent.id === agentId);
    expect(listed?.name).toBe(agentName);
  });

  test("should preserve every agent during parallel provisioning", async ({ page }) => {
    const { token } = await login(page);
    const names = Array.from({ length: 12 }, (_, index) =>
      uniqueName(`parallel-agent-${index}`),
    );

    const provisioned = await Promise.all(
      names.map((name) => provisionAgent(page, token, name)),
    );
    expect(new Set(provisioned.map((agent) => agent.agentId)).size).toBe(names.length);
    expect(new Set(provisioned.map((agent) => agent.secret)).size).toBe(names.length);

    const snapshot = await getConsoleSnapshot(page, token);
    expect(snapshot.agents.map((agent) => agent.name)).toEqual(expect.arrayContaining(names));
  });

  test("should invalidate the old agent secret after rotation", async ({ page }) => {
    const { token, principal } = await login(page);
    const agent = await provisionAgent(page, token, uniqueName("rotated-secret"));
    const group = await createGroup(page, token, principal.id, uniqueName("rotation-group"), [
      agent.agentId,
    ]);
    const postAsAgent = (secret: string, content: string) =>
      page.request.post(`${API_BASE}/v1/messages`, {
        headers: { Authorization: `Bearer ${secret}` },
        data: {
          actor_id: agent.agentId,
          conversation_id: group.id,
          content,
          content_type: "text/plain",
          idempotency_key: uniqueName("rotation-message"),
          metadata: {},
        },
      });

    expect((await postAsAgent(agent.secret, "before rotation")).status()).toBe(201);
    const rotate = await page.request.post(
      `${API_BASE}/v1/agents/${agent.agentId}/rotate-secret`,
      {
        headers: { Authorization: `Bearer ${token}` },
        data: { actor_id: principal.id },
      },
    );
    expect(rotate.ok()).toBeTruthy();
    const rotated = (await rotate.json()) as { secret: string };
    expect(rotated.secret).not.toBe(agent.secret);

    expect((await postAsAgent(agent.secret, "old secret must fail")).status()).toBe(401);
    expect((await postAsAgent(rotated.secret, "new secret works")).status()).toBe(201);
  });

  /* ---------------------------------------------------------------------- */
  /*  Agent in conversations                                                 */
  /* ---------------------------------------------------------------------- */

  test("should show agents in conversation member lists", async ({ page }) => {
    const { token, principal } = await login(page);
    const agent = await provisionAgent(page, token, uniqueName("agent-member"));
    const group = await createGroup(page, token, principal.id, uniqueName("agent-members"), [
      agent.agentId,
    ]);
    const memberIds = Object.keys(group.members);
    expect(memberIds).toContain(principal.id);
    expect(memberIds).toContain(agent.agentId);
  });

  test("should add an agent to a group via API", async ({ page }) => {
    const { token, principal } = await login(page);
    const agent = await provisionAgent(page, token, uniqueName("agent-add"));
    const group = await createGroup(page, token, principal.id, uniqueName("agent-add-group"));

    await addGroupMember(page, token, group.id, principal.id, [agent.agentId]);

    const snap = await getConsoleSnapshot(page, token);
    const updatedGroup = snap.conversations.find((c) => c.id === group.id);
    expect(updatedGroup).toBeTruthy();
    expect(Object.keys(updatedGroup?.members ?? {})).toContain(agent.agentId);
  });

  /* ---------------------------------------------------------------------- */
  /*  Disabled agents                                                        */
  /* ---------------------------------------------------------------------- */

  test("should remove disabled agent direct conversations from the sidebar", async ({
    page,
  }) => {
    const { token, principal } = await login(page);
    const agent = await provisionAgent(page, token, uniqueName("agent-disabled"));
    expect(agent.conversationId).toBeTruthy();
    const res = await page.request.post(`${API_BASE}/v1/agents/batch-disable`, {
      headers: { Authorization: `Bearer ${token}` },
      data: {
        actor_id: principal.id,
        agent_ids: [agent.agentId],
        conversation_ids: [agent.conversationId],
      },
    });
    expect(res.ok()).toBeTruthy();

    await gotoDashboard(page);
    await expect(page.locator(".conv-item").filter({ hasText: agent.agentName })).toHaveCount(0);
  });

  /* ---------------------------------------------------------------------- */
  /*  Batch delete/disable                                                   */
  /* ---------------------------------------------------------------------- */

  test("should open manage chats mode from sidebar menu", async ({ page }) => {
    const { token, principal } = await login(page);
    await createGroup(page, token, principal.id, uniqueName("agent-manage"));
    await gotoDashboard(page);
    const actionsBtn = page.locator('[aria-label="Actions menu"]');
    await actionsBtn.click();
    const manageBtn = page.getByText("Manage Chats");
    await expect(manageBtn).toBeVisible({ timeout: 3000 });
    await manageBtn.click();
    await expect(page.getByText("Cancel")).toBeVisible();
  });

  test("should show select all/none in manage mode", async ({ page }) => {
    const { token, principal } = await login(page);
    await createGroup(page, token, principal.id, uniqueName("agent-select"));
    await gotoDashboard(page);
    const actionsBtn = page.locator('[aria-label="Actions menu"]');
    await actionsBtn.click();
    const manageBtn = page.getByText("Manage Chats");
    await expect(manageBtn).toBeVisible({ timeout: 3000 });
    await manageBtn.click();
    await expect(page.getByRole("button", { name: "All", exact: true })).toBeVisible({
      timeout: 3000,
    });
  });

});
