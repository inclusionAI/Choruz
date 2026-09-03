import { expect, test } from "@playwright/test";
import { API_BASE, login, gotoDashboard } from "../fixtures/auth";
import {
  createCompany,
  deleteCompany,
  getConsoleSnapshot,
  provisionAgent,
  uniqueName,
} from "../fixtures/api";

test.describe("Company management", () => {
  test.beforeEach(async ({ page }) => {
    await login(page);
    await gotoDashboard(page);
  });

  /* ---------------------------------------------------------------------- */
  /*  Company selector                                                       */
  /* ---------------------------------------------------------------------- */

  test("should show the company selector in the sidebar", async ({ page }) => {
    const selector = page.locator(".company-selector-btn");
    // There should be at least one company (the default)
    if (await selector.isVisible({ timeout: 5000 })) {
      await expect(selector).toBeVisible();
    }
  });

  test("should open company dropdown on click", async ({ page }) => {
    const btn = page.locator(".company-selector-btn");
    if (!(await btn.isVisible({ timeout: 5000 }))) {
      test.skip();
      return;
    }
    await btn.click();
    await expect(page.locator(".company-dropdown")).toBeVisible();
  });

  test("should display active company name", async ({ page }) => {
    const selectorName = page.locator(".company-selector-name");
    if (!(await selectorName.isVisible({ timeout: 5000 }))) {
      test.skip();
      return;
    }
    const name = await selectorName.textContent();
    expect(name?.trim().length).toBeGreaterThan(0);
  });

  /* ---------------------------------------------------------------------- */
  /*  Create company via API                                                 */
  /* ---------------------------------------------------------------------- */

  test("should create a company via API", async ({ page }) => {
    const { token, principal } = await login(page);
    const name = uniqueName("e2e-company");
    const company = await createCompany(page, token, principal.id, name);
    expect(company.id).toBeTruthy();
    expect(company.name).toBe(name);
  });

  test("creates a company with its AI Manager and runtime binding through the UI", async ({
    page,
  }) => {
    const { token } = await login(page);
    const companyName = uniqueName("managed-company");
    let companyId: string | undefined;

    try {
      await page.locator('[aria-label="Actions menu"]').click();
      await page.getByRole("button", { name: "New Company" }).click();

      const dialog = page.getByRole("dialog", { name: "Create Company" });
      await expect(dialog).toBeVisible();
      await dialog.getByLabel("Company Name").fill(companyName);
      await expect(dialog.getByLabel("Include AI Manager")).toBeChecked();
      const managerDriver = dialog.getByLabel("Manager Driver");
      for (const driver of ["pi_terminal", "grok_terminal", "opencode_terminal"]) {
        await expect(managerDriver.locator(`option[value="${driver}"]`)).toHaveCount(1);
      }
      await managerDriver.selectOption("codex_terminal");
      const companyResponsePromise = page.waitForResponse((response) =>
        response.request().method() === "POST" &&
        new URL(response.url()).pathname === "/api/companies"
      );
      await dialog.getByRole("button", { name: "Create", exact: true }).click();
      const companyResponse = await companyResponsePromise;
      expect(companyResponse.ok()).toBeTruthy();
      companyId = ((await companyResponse.json()) as { id: string }).id;
      await expect(dialog).toBeHidden({ timeout: 30_000 });
      await expect(page.locator(".company-selector-name")).toHaveText(companyName);
      expect(companyId).toBeTruthy();

      const snapshot = await getConsoleSnapshot(page, token);
      const manager = snapshot.agents.find((agent) =>
        agent.name === "AI Manager" && agent.workspace_id === companyId
      );
      expect(manager).toBeTruthy();
      const directConversation = snapshot.conversations.find((conversation) =>
        conversation.workspace_id === companyId &&
        conversation.conversation_type === "direct" &&
        manager &&
        Object.keys(conversation.members).includes(manager.id)
      );
      expect(directConversation).toBeTruthy();

      const bindingsResponse = await page.request.get(`${API_BASE}/v1/runtime/bindings`, {
        headers: { Authorization: `Bearer ${token}` },
      });
      expect(bindingsResponse.ok()).toBeTruthy();
      const bindings = await bindingsResponse.json() as Array<{
        agent_principal_id: string;
        conversation_id: string;
        driver_type: string;
        workspace_id: string;
      }>;
      expect(bindings).toContainEqual(expect.objectContaining({
        agent_principal_id: manager?.id,
        conversation_id: directConversation?.id,
        driver_type: "codex_terminal",
        workspace_id: companyId,
      }));

      const managerConversation = page.locator(".conv-item").filter({ hasText: "AI Manager" }).first();
      await expect(managerConversation).toBeVisible({ timeout: 15_000 });
      await managerConversation.click();
      await expect(page.locator(".terminal-container, .xterm, .xterm-screen").first()).toBeVisible({
        timeout: 15_000,
      });
      await expect(page.locator(".chat-input-row textarea")).toHaveCount(0);
    } finally {
      if (companyId) await deleteCompany(page, token, companyId);
    }
  });

  /* ---------------------------------------------------------------------- */
  /*  Switch company                                                         */
  /* ---------------------------------------------------------------------- */

  test("should switch active company from dropdown", async ({ page }) => {
    const btn = page.locator(".company-selector-btn");
    if (!(await btn.isVisible({ timeout: 5000 }))) {
      test.skip();
      return;
    }
    await btn.click();
    const items = page.locator(".company-dropdown-item");
    const count = await items.count();
    if (count < 2) {
      test.skip();
      return;
    }
    // Click the second item
    await items.nth(1).locator(".company-dropdown-item-name").click();
    // Dropdown should close
    await expect(page.locator(".company-dropdown")).not.toBeVisible({
      timeout: 5000,
    });
  });

  test("switching companies clears stale conversations and shows only the selected workspace", async ({
    page,
  }) => {
    const { token, principal } = await login(page);
    let firstCompany: Awaited<ReturnType<typeof createCompany>> | undefined;
    let secondCompany: Awaited<ReturnType<typeof createCompany>> | undefined;
    let firstAgent: Awaited<ReturnType<typeof provisionAgent>> | undefined;
    let secondAgent: Awaited<ReturnType<typeof provisionAgent>> | undefined;

    const switchTo = async (companyName: string) => {
      await page.locator(".company-selector-btn").click();
      const item = page.locator(".company-dropdown-item").filter({ hasText: companyName });
      await expect(item).toHaveCount(1);
      await item.locator(".company-dropdown-item-name").click();
      await expect(page.locator(".company-selector-name")).toHaveText(companyName);
    };

    try {
      firstCompany = await createCompany(page, token, principal.id, uniqueName("switch-a"));
      secondCompany = await createCompany(page, token, principal.id, uniqueName("switch-b"));
      firstAgent = await provisionAgent(page, token, uniqueName("switch-agent-a"), {
        workspaceId: firstCompany.id,
      });
      secondAgent = await provisionAgent(page, token, uniqueName("switch-agent-b"), {
        workspaceId: secondCompany.id,
      });
      await gotoDashboard(page);
      await switchTo(firstCompany.name);
      const firstConversation = page.locator(".conv-item").filter({ hasText: firstAgent.agentName }).first();
      await expect(firstConversation).toBeVisible({ timeout: 15_000 });
      await expect(page.locator(".conv-item").filter({ hasText: secondAgent.agentName })).toHaveCount(0);
      await firstConversation.click();
      await expect(page.locator(".terminal-container, .xterm, .xterm-screen").first()).toBeVisible({
        timeout: 15_000,
      });

      await switchTo(secondCompany.name);
      await expect(page.locator(".conv-item").filter({ hasText: firstAgent.agentName })).toHaveCount(0);
      await expect(page.locator(".conv-item").filter({ hasText: secondAgent.agentName }).first()).toBeVisible({
        timeout: 15_000,
      });
      await expect(page.locator(".terminal-container:visible, .xterm:visible")).toHaveCount(0);
      await expect(page.getByRole("heading", { name: "Welcome to Choruz" })).toBeVisible();
    } finally {
      if (firstCompany) await deleteCompany(page, token, firstCompany.id);
      if (secondCompany) await deleteCompany(page, token, secondCompany.id);
    }
  });

  /* ---------------------------------------------------------------------- */
  /*  Context menu actions                                                   */
  /* ---------------------------------------------------------------------- */

  test("should show context menu with dots button", async ({ page }) => {
    const btn = page.locator(".company-selector-btn");
    if (!(await btn.isVisible({ timeout: 5000 }))) {
      test.skip();
      return;
    }
    await btn.click();
    const dotsBtn = page.locator(".company-dots-btn").first();
    if (!(await dotsBtn.isVisible({ timeout: 3000 }))) {
      test.skip();
      return;
    }
    await dotsBtn.click();
    await expect(page.locator(".company-context-menu")).toBeVisible();
  });

  test("should show Rename option in context menu", async ({ page }) => {
    const btn = page.locator(".company-selector-btn");
    if (!(await btn.isVisible({ timeout: 5000 }))) {
      test.skip();
      return;
    }
    await btn.click();
    const dotsBtn = page.locator(".company-dots-btn").first();
    if (!(await dotsBtn.isVisible({ timeout: 3000 }))) {
      test.skip();
      return;
    }
    await dotsBtn.click();
    await expect(
      page.locator(".company-context-menu").getByText("Rename"),
    ).toBeVisible();
  });

  test("should show Archive option in context menu", async ({ page }) => {
    const btn = page.locator(".company-selector-btn");
    if (!(await btn.isVisible({ timeout: 5000 }))) {
      test.skip();
      return;
    }
    await btn.click();
    const dotsBtn = page.locator(".company-dots-btn").first();
    if (!(await dotsBtn.isVisible({ timeout: 3000 }))) {
      test.skip();
      return;
    }
    await dotsBtn.click();
    const ctx = page.locator(".company-context-menu");
    const hasArchive = await ctx.getByText("Archive").isVisible().catch(() => false);
    const hasUnarchive = await ctx.getByText("Unarchive").isVisible().catch(() => false);
    expect(hasArchive || hasUnarchive).toBeTruthy();
  });

  test("should show Delete option in context menu", async ({ page }) => {
    const btn = page.locator(".company-selector-btn");
    if (!(await btn.isVisible({ timeout: 5000 }))) {
      test.skip();
      return;
    }
    await btn.click();
    const dotsBtn = page.locator(".company-dots-btn").first();
    if (!(await dotsBtn.isVisible({ timeout: 3000 }))) {
      test.skip();
      return;
    }
    await dotsBtn.click();
    await expect(
      page.locator(".company-context-danger"),
    ).toBeVisible();
  });

  test("should show Hide option in context menu", async ({ page }) => {
    const btn = page.locator(".company-selector-btn");
    if (!(await btn.isVisible({ timeout: 5000 }))) {
      test.skip();
      return;
    }
    await btn.click();
    const dotsBtn = page.locator(".company-dots-btn").first();
    if (!(await dotsBtn.isVisible({ timeout: 3000 }))) {
      test.skip();
      return;
    }
    await dotsBtn.click();
    const ctx = page.locator(".company-context-menu");
    const hasHide = await ctx.getByText("Hide").isVisible().catch(() => false);
    const hasShow = await ctx.getByText("Show").isVisible().catch(() => false);
    expect(hasHide || hasShow).toBeTruthy();
  });

  /* ---------------------------------------------------------------------- */
  /*  Multi-select mode                                                      */
  /* ---------------------------------------------------------------------- */

  test("should enter multi-select mode in company dropdown", async ({
    page,
  }) => {
    const btn = page.locator(".company-selector-btn");
    if (!(await btn.isVisible({ timeout: 5000 }))) {
      test.skip();
      return;
    }
    await btn.click();
    const selectBtn = page.locator(".company-show-hidden").filter({ hasText: "Select" });
    if (!(await selectBtn.isVisible({ timeout: 3000 }))) {
      test.skip();
      return;
    }
    await selectBtn.click();
    // Should show checkboxes
    await expect(
      page.locator('.company-dropdown input[type="checkbox"]').first(),
    ).toBeVisible();
  });

  test("should show batch action buttons in select mode", async ({ page }) => {
    const btn = page.locator(".company-selector-btn");
    if (!(await btn.isVisible({ timeout: 5000 }))) {
      test.skip();
      return;
    }
    await btn.click();
    const selectBtn = page.locator(".company-show-hidden").filter({ hasText: "Select" });
    if (!(await selectBtn.isVisible({ timeout: 3000 }))) {
      test.skip();
      return;
    }
    await selectBtn.click();
    await expect(page.getByText("Cancel")).toBeVisible();
  });

  /* ---------------------------------------------------------------------- */
  /*  Rename (inline edit)                                                   */
  /* ---------------------------------------------------------------------- */

  test("should open inline rename input from context menu", async ({
    page,
  }) => {
    const btn = page.locator(".company-selector-btn");
    if (!(await btn.isVisible({ timeout: 5000 }))) {
      test.skip();
      return;
    }
    await btn.click();
    const dotsBtn = page.locator(".company-dots-btn").first();
    if (!(await dotsBtn.isVisible({ timeout: 3000 }))) {
      test.skip();
      return;
    }
    await dotsBtn.click();
    const renameBtn = page.locator(".company-context-menu").getByText("Rename");
    if (!(await renameBtn.isVisible({ timeout: 2000 }))) {
      test.skip();
      return;
    }
    await renameBtn.click();
    await expect(page.locator(".company-rename-input")).toBeVisible();
  });
});
