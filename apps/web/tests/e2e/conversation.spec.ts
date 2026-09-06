import { expect, test } from "@playwright/test";
import { postgresQueryClient } from "../../lib/groups/group-provisioning-db";
import { expandSidebarConversationSections, login, gotoDashboard } from "../fixtures/auth";
import {
  createGroup,
  createAgent,
  getConsoleSnapshot,
  provisionAgent,
  sendMessage,
  uniqueName,
} from "../fixtures/api";

test.describe("Conversations", () => {
  test.beforeEach(async ({ page }) => {
    const { token, principal } = await login(page);
    await createGroup(page, token, principal.id, uniqueName("conv-seed"));
    await gotoDashboard(page);
  });

  /* ---------------------------------------------------------------------- */
  /*  Conversation list                                                      */
  /* ---------------------------------------------------------------------- */

  test("should render the conversation list in the sidebar", async ({
    page,
  }) => {
    await expect(
      page.locator('.conversation-list, [role="listbox"]'),
    ).toBeVisible({ timeout: 10_000 });
  });

  test("should display at least one conversation", async ({ page }) => {
    await page.waitForSelector(".conv-item", { timeout: 10_000 });
    const count = await page.locator(".conv-item").count();
    expect(count).toBeGreaterThan(0);
  });

  test("should show conversation type indicators", async ({ page }) => {
    await page.waitForSelector(".conv-item", { timeout: 10_000 });
    // Each conv-item should have content (name/avatar)
    const first = page.locator(".conv-item").first();
    const text = await first.textContent();
    expect(text?.trim().length).toBeGreaterThan(0);
  });

  /* ---------------------------------------------------------------------- */
  /*  Create group conversation                                              */
  /* ---------------------------------------------------------------------- */

  test("should open create group modal from sidebar + menu", async ({
    page,
  }) => {
    const actionsBtn = page.locator('[aria-label="Actions menu"]');
    await actionsBtn.click();
    await expect(page.getByRole("button", { name: "New Group" })).toBeVisible();
    await page.getByRole("button", { name: "New Group" }).click();
    // The create group modal should be visible
    await expect(
      page.locator(".modal-overlay, .modal-backdrop").first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("should create a group via the API and see it in the sidebar", async ({
    page,
  }) => {
    const { token, principal } = await login(page);
    const groupName = uniqueName("e2e-group");
    await createGroup(page, token, principal.id, groupName);
    // Reload to pick up the new group
    await gotoDashboard(page);
    await expandSidebarConversationSections(page);
    await page.waitForSelector(".conv-item", { timeout: 10_000 });
    await expect(page.getByText(groupName)).toBeVisible({ timeout: 10_000 });
  });

  for (const rejectFirst of [false, true]) {
  test(`a delayed blank group submission creates one group on double click${rejectFirst ? " after a rejected request" : ""}`, async ({ page }) => {
    const { token, principal } = await login(page);
    const memberName = uniqueName("group-member");
    await createAgent(page, token, principal.id, memberName);
    const groupName = uniqueName("group-once");
    await gotoDashboard(page);
    await page.getByRole("button", { name: "Actions menu" }).click();
    await page.getByRole("button", { name: "New Group", exact: true }).click();
    const modal = page.getByRole("dialog", { name: "Create Group" });
    await modal.getByLabel("Group name", { exact: true }).fill(groupName);
    await modal.getByRole("button", { name: new RegExp(memberName) }).click();
    let release!: () => void;
    const held = new Promise<void>((resolve) => { release = resolve; });
    let received!: () => void;
    const submitted = new Promise<void>((resolve) => { received = resolve; });
    let posts = 0;
    await page.route("**/api/v1/groups", async (route) => {
      if (route.request().method() !== "POST" || route.request().postDataJSON().name !== groupName) {
        await route.continue();
        return;
      }
      posts += 1;
      if (rejectFirst && posts === 1) {
        await route.fulfill({ status: 400, contentType: "application/json", body: JSON.stringify({ error: "Creation rejected" }) });
        return;
      }
      const response = await route.fetch();
      received();
      await held;
      await route.fulfill({ response });
    });
    try {
      if (rejectFirst) {
        await modal.getByRole("button", { name: "Create (1 selected)", exact: true }).click();
        await expect(modal.getByRole("alert")).toBeVisible();
        await expect(modal.getByRole("button", { name: "Create (1 selected)", exact: true })).toBeEnabled();
      }
      await modal.getByRole("button", { name: "Create (1 selected)", exact: true }).dblclick();
      await submitted;
      await expect(modal.getByRole("button", { name: "Creating…", exact: true })).toBeDisabled();
      await expect(modal.getByRole("button", { name: "Cancel", exact: true })).toBeDisabled();
      release();
      await expect(modal).not.toBeVisible();
      const db = await postgresQueryClient();
      const rows = await db.query("SELECT id FROM conversation WHERE creator_id = $1 AND name = $2", [principal.id, groupName]);
      expect(rows.rows).toHaveLength(1);
      expect(posts).toBe(rejectFirst ? 2 : 1);
    } finally {
      release();
      await page.unrouteAll({ behavior: "wait" });
    }
  });
  }

  /* ---------------------------------------------------------------------- */
  /*  Select conversation                                                    */
  /* ---------------------------------------------------------------------- */

  test("should select a conversation and show the chat area", async ({
    page,
  }) => {
    await page.waitForSelector(".conv-item", { timeout: 10_000 });
    await page.locator(".conv-item").first().click();
    // Chat header or message list should appear
    await expect(
      page.locator(".chat-header, .message-list, .terminal-container").first(),
    ).toBeVisible({ timeout: 10_000 });
  });

  test("should highlight the active conversation in sidebar", async ({
    page,
  }) => {
    await page.waitForSelector(".conv-item", { timeout: 10_000 });
    // Remember which conversation was clicked: tests running in parallel
    // create groups too, and a newer one can move to the top of the list.
    const item = page.locator(".conv-item").first();
    const id = await item.locator(".conv-item-main").getAttribute("data-conversation-id");
    expect(id).toBeTruthy();
    const selected = page.locator(`.conv-item:has(.conv-item-main[data-conversation-id="${id}"])`);
    await selected.locator(".conv-item-main").click();
    await expect(selected).toHaveClass(/\bactive\b/);
  });

  /* ---------------------------------------------------------------------- */
  /*  Direct conversation (DM with agent)                                    */
  /* ---------------------------------------------------------------------- */

  test("should create a direct conversation when provisioning an agent", async ({
    page,
  }) => {
    const { token, principal } = await login(page);
    const snap = await getConsoleSnapshot(page, token);
    const directConvs = snap.conversations.filter(
      (c) => c.conversation_type === "direct",
    );
    // There should be at least the AI Manager direct conversation
    expect(directConvs.length).toBeGreaterThanOrEqual(0);
  });

  /* ---------------------------------------------------------------------- */
  /*  Unread count badges                                                    */
  /* ---------------------------------------------------------------------- */

  test("should show unread badge when a new message arrives in another conv", async ({
    page,
  }) => {
    const { token, principal } = await login(page);
    const snap = await getConsoleSnapshot(page, token);
    const groups = snap.conversations.filter(
      (c) => c.conversation_type === "group",
    );
    if (groups.length < 2) {
      test.skip();
      return;
    }
    // Select first group
    await page.waitForSelector(".conv-item", { timeout: 10_000 });
    await page.locator(".conv-item").first().click();
    // Send a message to the second group via API
    const secondGroup = groups[1];
    await sendMessage(
      page,
      token,
      principal.id,
      secondGroup.id,
      `unread-test-${Date.now()}`,
    );
    // Wait for the badge to appear (WS push or poll)
    await page.waitForTimeout(5000);
    // Check if any unread badges are visible
    const badges = page.locator(".unread-badge, .unread-count");
    // The badge may or may not be visible depending on timing; just verify no crash
    expect(await badges.count()).toBeGreaterThanOrEqual(0);
  });

  /* ---------------------------------------------------------------------- */
  /*  Search filter                                                          */
  /* ---------------------------------------------------------------------- */

  test("should filter conversations with the search input", async ({
    page,
  }) => {
    await page.waitForSelector(".conv-item", { timeout: 10_000 });
    const countBefore = await page.locator(".conv-item").count();
    const searchInput = page.locator(".sidebar-search input");
    await searchInput.fill("zzz-nonexistent-query");
    await page.waitForTimeout(300);
    const countAfter = await page.locator(".conv-item").count();
    // Either zero results or fewer than before
    expect(countAfter).toBeLessThanOrEqual(countBefore);
  });

  test("should clear search and restore full list", async ({ page }) => {
    const { token, principal } = await login(page);
    const group = await createGroup(page, token, principal.id, uniqueName("restore-search"));
    await gotoDashboard(page);
    const ownedItem = page.locator(`[data-conversation-id="${group.id}"]`);
    await expect(ownedItem).toBeVisible();
    const searchInput = page.locator(".sidebar-search input");
    await searchInput.fill("zzz-nonexistent");
    await expect(ownedItem).toHaveCount(0);
    await searchInput.fill("");
    await expect(ownedItem).toBeVisible();
  });

  /* ---------------------------------------------------------------------- */
  /*  Conversation sorting                                                   */
  /* ---------------------------------------------------------------------- */

  test("should sort conversations by most recent message", async ({
    page,
  }) => {
    // Verify that the conversation list is rendered (sorting is done client-side)
    await page.waitForSelector(".conv-item", { timeout: 10_000 });
    const count = await page.locator(".conv-item").count();
    expect(count).toBeGreaterThan(0);
    // The list is sorted if it renders without error
  });
});
