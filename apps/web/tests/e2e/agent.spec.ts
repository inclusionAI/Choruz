import { expect, test } from "@playwright/test";
import { mkdir, writeFile, readFile, rm } from "node:fs/promises";
import { join } from "node:path";
import { API_BASE, login, signup, gotoDashboard } from "../fixtures/auth";
import {
  getConsoleSnapshot,
  provisionAgent,
  createGroup,
  addGroupMember,
  uniqueName,
  createCompany,
  deleteCompany,
} from "../fixtures/api";

for (const scenario of ["reports load and delete failures", "keeps skill content with its selected title", "keeps loading until the selected skill returns", "browses and imports a Markdown file"] as const) {
  test(`Skills ${scenario}`, async ({ page }, testInfo) => {
    const { token, principal } = await signup(page, uniqueName("skills-user"), "skills-test-password");
    const company = await createCompany(page, token, principal.id, uniqueName("skills-owner"));
    const ownedDirs: string[] = [];
    let release = () => {};
    let releaseSelected = () => {};
    try {
      const agent = await provisionAgent(page, token, uniqueName("skills-agent"), { workspaceId: company.id });
      for (const name of ["audit-a", "audit-b"]) {
        const dir = join(agent.workspacePath, ".claude", "skills", name);
        ownedDirs.push(dir);
        await mkdir(dir, { recursive: true });
        await writeFile(join(dir, "SKILL.md"), `# ${name}\nOwned ${name} content\n`);
      }
      await page.routeWebSocket((url) => url.pathname.startsWith("/v1/ws/terminals/"), (socket) => socket.close());
      await gotoDashboard(page);
      await page.locator(".company-selector-btn").click();
      await page.locator(".company-dropdown-item-name").filter({ hasText: company.name }).click();
      await page.locator(`[data-conversation-id="${agent.conversationId}"]`).click();
      await page.getByTitle("Toggle details", { exact: true }).click();
      if (scenario === "browses and imports a Markdown file") {
        const sourceDir = join(agent.workspacePath, "skill-source");
        const source = join(sourceDir, "audit-import.md");
        const installed = join(agent.workspacePath, ".claude", "commands", "audit-import.md");
        ownedDirs.push(sourceDir, installed);
        await mkdir(sourceDir, { recursive: true });
        await writeFile(source, "# Imported skill\nOwned original instructions\n");
        await writeFile(join(sourceDir, "not-a-skill.txt"), "Do not import");
        await page.locator(".detail-tab").filter({ hasText: "Skills" }).click();
        const panel = page.locator(".detail-panel");
        await panel.getByTitle("Add skill", { exact: true }).click();
        await panel.getByRole("button", { name: "Browse", exact: true }).click();
        const picker = page.locator(".folder-picker-modal");
        await picker.getByRole("option", { name: "skill-source", exact: true }).dblclick();
        await expect(picker.getByRole("option", { name: "not-a-skill.txt", exact: true })).toHaveCount(0);
        await expect(picker.getByRole("button", { name: "Select File", exact: true })).toBeDisabled();
        await expect(picker.getByRole("option", { name: "audit-import.md", exact: true })).toBeVisible();
        await picker.getByRole("option", { name: "audit-import.md", exact: true }).click();
        await picker.screenshot({ path: testInfo.outputPath("skill-file-picker.png") });
        await picker.getByRole("button", { name: "Select File", exact: true }).click();
        await expect(picker).toHaveCount(0);
        await expect(panel.getByPlaceholder("/path/to/skill.md", { exact: true })).toHaveValue(source);
        await expect(readFile(installed, "utf8")).rejects.toMatchObject({ code: "ENOENT" });
        await panel.getByRole("button", { name: "Import skill", exact: true }).click();
        await expect(panel.getByText("audit-import", { exact: true })).toBeVisible();
        expect(await readFile(installed, "utf8")).toBe(await readFile(source, "utf8"));
        expect(await readFile(source, "utf8")).toBe("# Imported skill\nOwned original instructions\n");
        return;
      }
      if (scenario === "reports load and delete failures") {
        await page.route((url) => url.pathname === "/api/agent-skills" && url.searchParams.get("workspace_path") === agent.workspacePath && !url.searchParams.has("read"),
          (route) => route.fulfill({ status: 503, body: "unavailable" }), { times: 1 });
      }
      await page.locator(".detail-tab").filter({ hasText: "Skills" }).click();
      const panel = page.locator(".detail-panel");
      if (scenario === "reports load and delete failures") {
        await expect(panel.getByRole("alert")).toContainText("Could not load skills");
        await expect(panel.getByRole("heading", { name: "Skills (0)", exact: true })).toHaveCount(0);
        await expect(panel.getByText("No skills installed.", { exact: true })).toHaveCount(0);
        await panel.getByRole("button", { name: "Retry", exact: true }).click();
        await expect(panel.getByText("audit-a", { exact: true })).toBeVisible();
        await page.route("**/api/agent-skills", async (route) => {
          if (route.request().method() === "DELETE") await route.fulfill({ status: 503, body: "unavailable" });
          else await route.continue();
        }, { times: 1 });
        page.on("dialog", (dialog) => dialog.accept());
        await panel.getByTitle("Delete audit-a", { exact: true }).click();
        await expect(panel.getByRole("alert")).toContainText('Could not delete "audit-a"');
        expect(await readFile(join(ownedDirs[0], "SKILL.md"), "utf8")).toContain("Owned audit-a");
        await expect(panel.getByText("audit-a", { exact: true })).toBeVisible();
        await panel.getByTitle("Delete audit-a", { exact: true }).click();
        await expect(panel.getByText("audit-a", { exact: true })).toHaveCount(0);
        await expect(readFile(join(ownedDirs[0], "SKILL.md"), "utf8")).rejects.toMatchObject({ code: "ENOENT" });
      } else {
        await expect(panel.getByText("audit-a", { exact: true })).toBeVisible();
        let arrived!: () => void;
        let completed!: () => void;
        const held = new Promise<void>((resolve) => { arrived = resolve; });
        const released = new Promise<void>((resolve) => { release = resolve; });
        const delivered = new Promise<void>((resolve) => { completed = resolve; });
        const selectedResponse = new Promise<void>((resolve) => { releaseSelected = resolve; });
        if (scenario === "keeps loading until the selected skill returns") {
          await page.route((url) => url.pathname === "/api/agent-skills" && url.searchParams.get("workspace_path") === agent.workspacePath && url.searchParams.get("read") === "audit-b", async (route) => {
            const response = await route.fetch();
            await selectedResponse;
            await route.fulfill({ response });
          });
        }
        await page.route((url) => url.pathname === "/api/agent-skills" && url.searchParams.get("workspace_path") === agent.workspacePath && url.searchParams.get("read") === "audit-a", async (route) => {
          const response = await route.fetch();
          arrived();
          await released;
          await route.fulfill({ response });
          completed();
        });
        await panel.getByText("audit-a", { exact: true }).click();
        await held;
        await panel.getByText("audit-b", { exact: true }).click();
        if (scenario === "keeps skill content with its selected title") {
          await expect(panel.locator(".skill-content-pre")).toContainText("Owned audit-b");
        }
        release();
        await delivered;
        await page.evaluate(() => new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
        if (scenario === "keeps loading until the selected skill returns") {
          await expect(panel.locator(".skill-content-panel")).toContainText("Loading");
          await expect(panel.locator(".skill-content-pre")).toHaveCount(0);
          releaseSelected();
        }
        await expect(panel.locator(".skill-content-pre")).toContainText("Owned audit-b");
        await expect(panel.locator(".skill-content-pre")).not.toContainText("Owned audit-a");
      }
    } finally {
      release();
      releaseSelected();
      await page.unrouteAll({ behavior: "wait" });
      try { await deleteCompany(page, token, company.id); }
      finally { for (const dir of ownedDirs) await rm(dir, { recursive: true, force: true }); }
    }
  });
}

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
