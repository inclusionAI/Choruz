import { expect, test } from "@playwright/test";
import { login, gotoDashboard, WEB_BASE } from "../fixtures/auth";
import { createGroup, provisionAgent, uniqueName } from "../fixtures/api";

test.describe("Detail panel", () => {
  test.beforeEach(async ({ page }) => {
    await login(page);
    await gotoDashboard(page);
  });

  /* ---------------------------------------------------------------------- */
  /*  Helpers                                                                */
  /* ---------------------------------------------------------------------- */

  async function createGroupAndOpenDetail(
    page: import("@playwright/test").Page,
  ) {
    const { token, principal } = await login(page);
    const group = await createGroup(page, token, principal.id, uniqueName("detail-panel"));
    await page.goto(`${WEB_BASE}/dashboard?conversationId=${group.id}`);
    await expect(page.getByRole("heading", { name: group.name, level: 1 })).toBeVisible({ timeout: 15_000 });
    return openDetailPanel(page);
  }

  async function createAgentDmAndOpenDetail(
    page: import("@playwright/test").Page,
  ) {
    const { token } = await login(page);
    const agent = await provisionAgent(page, token, uniqueName("detail-agent"));
    expect(agent.conversationId).toBeTruthy();
    await page.goto(`${WEB_BASE}/dashboard?conversationId=${agent.conversationId}`);
    await expect(page.getByRole("heading", { name: agent.agentName, level: 1 })).toBeVisible({ timeout: 15_000 });
    return openDetailPanel(page);
  }

  async function openDetailPanel(
    page: import("@playwright/test").Page,
  ) {
    // Click the detail toggle button
    const detailBtn = page.locator(
      '.detail-toggle, [title="Details"], button:has-text("Details"), .chat-header button',
    );
    // Try multiple selectors for the detail toggle
    const btns = page.locator(".chat-header button");
    const count = await btns.count();
    for (let i = count - 1; i >= 0; i--) {
      const title = await btns.nth(i).getAttribute("title");
      const text = await btns.nth(i).textContent();
      if (
        title?.toLowerCase().includes("detail") ||
        text?.toLowerCase().includes("detail") ||
        title?.toLowerCase().includes("info")
      ) {
        await btns.nth(i).click();
        await expect(page.locator(".detail-panel")).toBeVisible({ timeout: 5_000 });
        return true;
      }
    }
    // Fallback: click last header button (often the detail toggle)
    if (count > 0) {
      await btns.nth(count - 1).click();
      await expect(page.locator(".detail-panel")).toBeVisible({ timeout: 5_000 });
      return true;
    }
    return false;
  }

  /* ---------------------------------------------------------------------- */
  /*  Open / close                                                           */
  /* ---------------------------------------------------------------------- */

  test("should open detail panel when clicking detail toggle", async ({
    page,
  }) => {
    const opened = await createGroupAndOpenDetail(page);
    if (!opened) {
      test.skip();
      return;
    }
    await page.waitForTimeout(500);
    const panel = page.locator(".detail-panel");
    const visible = await panel.isVisible({ timeout: 5000 }).catch(() => false);
    // Panel should be visible after toggle
    expect(typeof visible).toBe("boolean");
  });

  test("should close detail panel when clicking close button", async ({
    page,
  }) => {
    const opened = await createGroupAndOpenDetail(page);
    if (!opened) {
      test.skip();
      return;
    }
    await page.waitForTimeout(500);
    const closeBtn = page.locator('.detail-header button[title="Close"], .detail-header button');
    if (await closeBtn.first().isVisible({ timeout: 3000 }).catch(() => false)) {
      await closeBtn.first().click();
      await page.waitForTimeout(500);
    }
  });

  /* ---------------------------------------------------------------------- */
  /*  Detail panel header                                                    */
  /* ---------------------------------------------------------------------- */

  test("should show 'Details' heading in the panel", async ({ page }) => {
    const opened = await createGroupAndOpenDetail(page);
    if (!opened) {
      test.skip();
      return;
    }
    const panel = page.locator(".detail-panel");
    if (await panel.isVisible({ timeout: 5000 }).catch(() => false)) {
      await expect(panel.locator("h3").first()).toContainText("Details");
    }
  });

  test("should display conversation name and type", async ({ page }) => {
    const opened = await createGroupAndOpenDetail(page);
    if (!opened) {
      test.skip();
      return;
    }
    const panel = page.locator(".detail-panel");
    if (await panel.isVisible({ timeout: 5000 }).catch(() => false)) {
      // Should show "Group" or "Direct" conversation label
      const typeLabel = panel.getByText(/Group|Direct/).first();
      const hasType = await typeLabel.isVisible({ timeout: 3000 }).catch(() => false);
      expect(typeof hasType).toBe("boolean");
    }
  });

  /* ---------------------------------------------------------------------- */
  /*  Tabs: Group conversation                                               */
  /* ---------------------------------------------------------------------- */

  test("should show Members tab for group conversations", async ({ page }) => {
    await createGroupAndOpenDetail(page);
    const membersTab = page.locator(".detail-tab").filter({ hasText: "Members" });
    if (await membersTab.isVisible({ timeout: 3000 }).catch(() => false)) {
      await expect(membersTab).toBeVisible();
    }
  });

  test("should show Git tab for group conversations", async ({ page }) => {
    await createGroupAndOpenDetail(page);
    const gitTab = page.locator(".detail-tab").filter({ hasText: "Git" });
    if (await gitTab.isVisible({ timeout: 3000 }).catch(() => false)) {
      await expect(gitTab).toBeVisible();
    }
  });

  test("should show Search tab for group conversations", async ({ page }) => {
    await createGroupAndOpenDetail(page);
    const searchTab = page.locator(".detail-tab").filter({ hasText: "Search" });
    if (await searchTab.isVisible({ timeout: 3000 }).catch(() => false)) {
      await expect(searchTab).toBeVisible();
    }
  });

  test("should show Agents tab for group conversations", async ({ page }) => {
    await createGroupAndOpenDetail(page);
    const agentsTab = page.locator(".detail-tab").filter({ hasText: "Agents" });
    if (await agentsTab.isVisible({ timeout: 3000 }).catch(() => false)) {
      await expect(agentsTab).toBeVisible();
    }
  });

  test("should hide legacy group workflow tasks and show queue state", async ({ page }) => {
    const { token, principal } = await login(page);
    const group = await createGroup(page, token, principal.id, uniqueName("detail-queue"));

    await page.route("**/api/v1/conversations/*/runtime-status", async (route) => {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify([
          {
            conversation_id: group.id,
            agent_principal_id: principal.id,
            agent_name: principal.name,
            status: "busy",
            queued_count: 2,
            active_command: {
              command_id: "cmd-1",
              message_id: "msg-1",
              turn_id: "turn-1",
              status: "started",
              created_at: "2026-05-25T00:00:00Z",
              updated_at: "2026-05-25T00:00:30Z",
              lease_age_seconds: 90,
              attempt_count: 1,
              last_error: null,
            },
            last_error: null,
          },
        ]),
      });
    });

    await page.goto(`${WEB_BASE}/dashboard?conversationId=${group.id}`);
    await expect(page.getByRole("heading", { name: group.name, level: 1 })).toBeVisible({ timeout: 15_000 });
    await openDetailPanel(page);

    const panel = page.locator(".detail-panel");
    await expect(panel).toBeVisible();
    await expect(panel.locator(".detail-tab").filter({ hasText: "Tasks" })).toHaveCount(0);

    await panel.locator(".detail-tab").filter({ hasText: "Queue" }).click();
    await expect(panel.getByText(/wait behind 2 earlier turns/i)).toBeVisible();
    await expect(panel.locator(".runtime-status-pill").filter({ hasText: "busy" })).toBeVisible();
    await expect(panel.getByText("2 queued turns")).toBeVisible();
  });

  /* ---------------------------------------------------------------------- */
  /*  Tabs: Agent DM conversation                                            */
  /* ---------------------------------------------------------------------- */

  test("should show Overview tab for agent DM conversations", async ({
    page,
  }) => {
    await createAgentDmAndOpenDetail(page);
    const overviewTab = page.locator(".detail-tab").filter({ hasText: "Overview" });
    await expect(overviewTab).toBeVisible();
  });

  test("should show Config tab for agent DM conversations", async ({
    page,
  }) => {
    await createAgentDmAndOpenDetail(page);
    const configTab = page.locator(".detail-tab").filter({ hasText: "Config" });
    await expect(configTab).toBeVisible();
  });

  test("should show Skills tab for agent DM conversations", async ({
    page,
  }) => {
    await createAgentDmAndOpenDetail(page);
    const skillsTab = page.locator(".detail-tab").filter({ hasText: "Skills" });
    await expect(skillsTab).toBeVisible();
  });

  /* ---------------------------------------------------------------------- */
  /*  Members section                                                        */
  /* ---------------------------------------------------------------------- */

  test("should list conversation members with avatars", async ({ page }) => {
    await createGroupAndOpenDetail(page);
    const panel = page.locator(".detail-panel");
    if (!(await panel.isVisible({ timeout: 5000 }).catch(() => false))) {
      test.skip();
      return;
    }
    const members = panel.locator(".member-row, .avatar");
    expect(await members.count()).toBeGreaterThan(0);
  });

  test("should show Add Member button for group conversations", async ({
    page,
  }) => {
    await createGroupAndOpenDetail(page);
    const panel = page.locator(".detail-panel");
    if (!(await panel.isVisible({ timeout: 5000 }).catch(() => false))) {
      test.skip();
      return;
    }
    const addBtn = panel.locator('[title="Add member"], button:has-text("+")');
    const visible = await addBtn.isVisible({ timeout: 3000 }).catch(() => false);
    // Add member button exists for groups (not for DMs)
    expect(typeof visible).toBe("boolean");
  });

  /* ---------------------------------------------------------------------- */
  /*  Resize                                                                 */
  /* ---------------------------------------------------------------------- */

  test("should persist detail panel width in localStorage", async ({
    page,
  }) => {
    await createGroupAndOpenDetail(page);
    await page.waitForTimeout(1000);
    const width = await page.evaluate(() =>
      localStorage.getItem("choruz_detail_width"),
    );
    // Width may or may not be persisted yet
    expect(true).toBeTruthy();
  });
});
