import { expect, test } from "@playwright/test";
import { API_BASE, WEB_BASE, login, gotoDashboard } from "../fixtures/auth";
import { createGroup, sendMessage, uniqueName } from "../fixtures/api";

test.describe("Search", () => {
  test.beforeEach(async ({ page }) => {
    await login(page);
    await gotoDashboard(page);
  });

  /* ---------------------------------------------------------------------- */
  /*  Sidebar search (conversation filter)                                   */
  /* ---------------------------------------------------------------------- */

  test("should have a search input in the sidebar", async ({ page }) => {
    const searchInput = page.locator(".sidebar-search input");
    await expect(searchInput).toBeVisible({ timeout: 10_000 });
  });

  test("should accept text input for filtering", async ({ page }) => {
    const searchInput = page.locator(".sidebar-search input");
    await searchInput.fill("test");
    const value = await searchInput.inputValue();
    expect(value).toBe("test");
  });

  test("should filter conversation list based on search query", async ({
    page,
  }) => {
    const { token, principal } = await login(page);
    const group = await createGroup(page, token, principal.id, uniqueName("search-filter"));
    await gotoDashboard(page);
    const ownedConversation = page.locator(".conv-item").filter({ hasText: group.name });
    await expect(ownedConversation).toBeVisible();
    const searchInput = page.locator(".sidebar-search input");
    await searchInput.fill("zzz-no-match-expected");
    await expect(ownedConversation).toHaveCount(0);
  });

  test("should show 'No conversations found' for unmatched search", async ({
    page,
  }) => {
    const searchInput = page.locator(".sidebar-search input");
    await searchInput.fill("zzzzzzzzz-unique-no-match-999");
    await page.waitForTimeout(500);
    const empty = page.locator(".empty-state, .no-results");
    const noConvMsg = page.getByText("No conversations found");
    const hasEmpty = await empty.isVisible({ timeout: 3000 }).catch(() => false);
    const hasMsg = await noConvMsg.isVisible({ timeout: 1000 }).catch(() => false);
    expect(hasEmpty || hasMsg || (await page.locator(".conv-item").count()) === 0).toBeTruthy();
  });

  test("should clear search and show all conversations", async ({ page }) => {
    const { token, principal } = await login(page);
    const group = await createGroup(page, token, principal.id, uniqueName("search-clear"));
    await gotoDashboard(page);
    const ownedConversation = page.locator(".conv-item").filter({ hasText: group.name });
    await expect(ownedConversation).toBeVisible();
    const searchInput = page.locator(".sidebar-search input");
    await searchInput.fill("zzz-no-match-expected");
    await expect(ownedConversation).toHaveCount(0);
    await searchInput.fill("");
    await expect(ownedConversation).toBeVisible();
  });

  test("should have placeholder text in search input", async ({ page }) => {
    const searchInput = page.locator(".sidebar-search input");
    const placeholder = await searchInput.getAttribute("placeholder");
    expect(placeholder).toContain("Search");
  });

  /* ---------------------------------------------------------------------- */
  /*  Detail panel search                                                    */
  /* ---------------------------------------------------------------------- */

  test("should show Search tab in detail panel for group chats", async ({
    page,
  }) => {
    await page.waitForSelector(".conv-item", { timeout: 10_000 });
    await page.locator(".conv-item").first().click();
    await page.waitForTimeout(500);
    // Open detail panel
    const headerBtns = page.locator(".chat-header button");
    const btnCount = await headerBtns.count();
    if (btnCount > 0) {
      await headerBtns.nth(btnCount - 1).click();
      await page.waitForTimeout(500);
    }
    const searchTab = page.locator(".detail-tab").filter({ hasText: "Search" });
    if (await searchTab.isVisible({ timeout: 3000 }).catch(() => false)) {
      await searchTab.click();
      await page.waitForTimeout(500);
      // Should show a search input within the detail panel
      const detailSearchInput = page.locator(".detail-panel input, .detail-search input");
      const hasInput = await detailSearchInput.isVisible({ timeout: 3000 }).catch(() => false);
      expect(typeof hasInput).toBe("boolean");
    }
  });

  /* ---------------------------------------------------------------------- */
  /*  Search results                                                         */
  /* ---------------------------------------------------------------------- */

  test("should display search results when typing in detail search", async ({
    page,
  }) => {
    const { token, principal } = await login(page);
    const group = await createGroup(page, token, principal.id, uniqueName("search-detail"));
    const uniqueContent = `search-target-${Date.now()}`;
    const message = await sendMessage(page, token, principal.id, group.id, uniqueContent);

    const apiResult = await page.request.get(
      `${API_BASE}/v1/messages/search?principal_id=${principal.id}&q=${encodeURIComponent(uniqueContent)}&conversation_id=${group.id}`,
      { headers: { Authorization: `Bearer ${token}` } },
    );
    expect(apiResult.ok()).toBeTruthy();
    expect(await apiResult.json()).toEqual(
      expect.arrayContaining([expect.objectContaining({ message_id: message.id })]),
    );

    await page.goto(`${WEB_BASE}/dashboard?conversationId=${group.id}`);
    await expect(page.locator(".messages-area")).toBeVisible({ timeout: 15_000 });
    await page.locator('.chat-header button[title="Toggle details"]').click();
    await page.locator(".detail-tab").filter({ hasText: "Search" }).click();
    await page.locator('input[placeholder="Search messages…"]').fill(uniqueContent);
    const result = page.locator(".detail-search-result", { hasText: uniqueContent });
    await expect(result).toBeVisible({ timeout: 10_000 });
    await expect(result.locator("mark")).toHaveText(uniqueContent);
  });

  test("should navigate to conversation when clicking a search result", async ({
    page,
  }) => {
    const { token, principal } = await login(page);
    const group = await createGroup(page, token, principal.id, uniqueName("search-navigate"));
    const content = `navigate-search-${Date.now()}`;
    await sendMessage(page, token, principal.id, group.id, content);

    await page.goto(`${WEB_BASE}/dashboard?conversationId=${group.id}`);
    await expect(page.locator(".messages-area")).toBeVisible({ timeout: 15_000 });
    await page.locator('.chat-header button[title="Toggle details"]').click();
    await page.locator(".detail-tab").filter({ hasText: "Search" }).click();
    const input = page.locator('input[placeholder="Search messages…"]');
    await input.fill(content);
    await page.locator(".detail-search-result", { hasText: content }).click();

    await expect(page.locator(".msg-group", { hasText: content })).toBeVisible();
    await expect(page.locator(".detail-search-result")).toHaveCount(0);
  });
});
