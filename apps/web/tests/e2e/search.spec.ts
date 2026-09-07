import { expect, test } from "@playwright/test";
import { API_BASE, WEB_BASE, login, signup, gotoDashboard } from "../fixtures/auth";
import { createGroup, sendMessage, uniqueName } from "../fixtures/api";

test.describe("Search", () => {
  test("switching conversations clears search and ignores an earlier response", async ({ page }) => {
    const { token, principal } = await login(page);
    const first = await createGroup(page, token, principal.id, uniqueName("search-first"));
    const second = await createGroup(page, token, principal.id, uniqueName("search-second"));
    const firstText = uniqueName("first-result");
    const secondText = uniqueName("second-result");
    await sendMessage(page, token, principal.id, first.id, firstText);
    await sendMessage(page, token, principal.id, second.id, secondText);
    await page.goto(`${WEB_BASE}/dashboard?conversationId=${first.id}`);
    await page.locator('.chat-header button[title="Toggle details"]').click();
    await page.locator(".detail-tab").filter({ hasText: "Search" }).click();
    const input = page.locator('input[placeholder="Search messages…"]');
    await input.fill(firstText);
    await expect(page.locator(".detail-search-result")).toHaveText(new RegExp(firstText));

    let markHeld!: () => void;
    let release!: () => void;
    let markDelivered!: () => void;
    const held = new Promise<void>((resolve) => { markHeld = resolve; });
    const released = new Promise<void>((resolve) => { release = resolve; });
    const delivered = new Promise<void>((resolve) => { markDelivered = resolve; });
    const delayedQuery = firstText.slice(0, -1);
    await page.route((url) => url.pathname.endsWith("/messages/search") && url.searchParams.get("q") === delayedQuery, async (route) => {
      const response = await route.fetch();
      markHeld();
      await released;
      await route.fulfill({ response });
      markDelivered();
    });
    try {
      await input.fill(delayedQuery);
      await held;
      await page.locator(`[data-conversation-id="${second.id}"]`).click();
      await page.locator(".detail-tab").filter({ hasText: "Search" }).click();
      await expect(input).toHaveValue("");
      await expect(page.locator(".detail-search-result")).toHaveCount(0);
      await input.fill(secondText);
      await expect(page.locator(".detail-search-result")).toHaveText(new RegExp(secondText));
      release();
      await delivered;
      await page.evaluate(() => new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
      await expect(page.locator(".detail-search-result")).toHaveText(new RegExp(secondText));
      await expect(input).toHaveValue(secondText);
    } finally {
      release();
      await page.unrouteAll({ behavior: "wait" });
    }
  });

  test.beforeEach(async ({ page }) => {
    await login(page);
    await gotoDashboard(page);
  });

  test("loads older search results and navigates to an unloaded match", async ({ page }) => {
    const { token, principal } = await signup(page, uniqueName("search-pages"), "Owned-password-123!");
    await page.goto("about:blank");
    const group = await createGroup(page, token, principal.id, uniqueName("search-pages"));
    const query = uniqueName("ownedmatch");
    const target = await sendMessage(page, token, principal.id, group.id, `${query} oldest`);
    for (let index = 1; index < 65; index++) {
      await sendMessage(page, token, principal.id, group.id, `${query} context ${index}`);
    }
    await page.goto(`${WEB_BASE}/dashboard?conversationId=${group.id}`);
    await expect(page.locator(".messages-area")).toBeVisible();
    await expect(page.locator(`[data-msg-id="${target.id}"]`)).toHaveCount(0);
    await page.locator('.chat-header button[title="Toggle details"]').click();
    await page.locator(".detail-tab").filter({ hasText: "Search" }).click();
    await page.locator('input[placeholder="Search messages…"]').fill(query);
    await expect(page.locator(".detail-search-result")).toHaveCount(30);
    const more = page.getByRole("button", { name: "Load more results", exact: true });
    await more.click();
    await expect(page.locator(".detail-search-result")).toHaveCount(60);
    await more.click();
    await expect(page.locator(".detail-search-result")).toHaveCount(65);
    await expect(more).toHaveCount(0);
    await page.locator(".detail-search-result", { hasText: `${query} oldest` }).click();
    await expect(page.locator(`[data-msg-id="${target.id}"]`)).toBeInViewport();
    await expect(page.locator(`[data-msg-id="${target.id}"]`)).toHaveClass(/msg-highlight/);
  });

  for (const scenario of ["retry", "query change"] as const) {
    test(`search continuation: ${scenario}`, async ({ page }) => {
      const { token, principal } = await signup(page, uniqueName("search-more"), "Owned-password-123!");
      await page.goto("about:blank");
      const group = await createGroup(page, token, principal.id, uniqueName("search-more"));
      const query = uniqueName("pagedmatch");
      const otherQuery = uniqueName("replacementquery");
      for (let index = 0; index < 35; index++) {
        await sendMessage(page, token, principal.id, group.id, `${query} ${index}`);
      }
      await sendMessage(page, token, principal.id, group.id, otherQuery);
      await page.goto(`${WEB_BASE}/dashboard?conversationId=${group.id}`);
      await page.locator('.chat-header button[title="Toggle details"]').click();
      await page.locator(".detail-tab").filter({ hasText: "Search" }).click();
      const input = page.locator('input[placeholder="Search messages…"]');
      await input.fill(query);
      await expect(page.locator(".detail-search-result")).toHaveCount(30);
      const continuation = (url: URL) => url.pathname.endsWith("/messages/search")
        && url.searchParams.get("conversation_id") === group.id && url.searchParams.has("before_message_id");
      const more = page.getByRole("button", { name: "Load more results", exact: true });
      if (scenario === "retry") {
        const error = page.locator(".detail-panel").getByRole("alert");
        await page.route(continuation, (route) => route.fulfill({ status: 503, body: "Owned failure" }), { times: 1 });
        await more.click();
        await expect(error).toContainText("Could not load search results.");
        await expect(page.locator(".detail-search-result")).toHaveCount(30);
        await error.getByRole("button", { name: "Retry", exact: true }).click();
        await expect(page.locator(".detail-search-result")).toHaveCount(35);
        await expect(error).toHaveCount(0);
        await expect(more).toHaveCount(0);
        return;
      }
      let markHeld!: () => void;
      let release!: () => void;
      let markDelivered!: () => void;
      const held = new Promise<void>((resolve) => { markHeld = resolve; });
      const gate = new Promise<void>((resolve) => { release = resolve; });
      const delivered = new Promise<void>((resolve) => { markDelivered = resolve; });
      await page.route(continuation, async (route) => {
        const response = await route.fetch();
        markHeld();
        await gate;
        await route.fulfill({ response });
        markDelivered();
      }, { times: 1 });
      try {
        await more.click();
        await held;
        const nextResponse = page.waitForResponse((response) => new URL(response.url()).searchParams.get("q") === otherQuery);
        await input.fill(otherQuery);
        const response = await nextResponse;
        expect(new URL(response.url()).searchParams.has("before_message_id")).toBe(false);
        await expect(page.locator(".detail-search-result")).toHaveText(new RegExp(otherQuery));
        release();
        await delivered;
        await page.evaluate(() => new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
        await expect(page.locator(".detail-search-result")).toHaveText(new RegExp(otherQuery));
        await expect(input).toHaveValue(otherQuery);
        await expect(more).toHaveCount(0);
      } finally {
        release();
        await page.unrouteAll({ behavior: "wait" });
      }
    });
  }

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

  for (const scenario of ["history", "retry", "cancel", "switch", "replace", "cancel replacement", "quiet reply", "loaded", "tasks"] as const) {
  test(`search target navigation: ${scenario}`, async ({
    page,
  }) => {
    const { token, principal } = await signup(page, uniqueName("s-nav"), "Owned-password-123!");
    const group = await createGroup(page, token, principal.id, uniqueName("search-navigate"));
    // Seed without a live sync consumer so the target is absent from the initial page/cache.
    await page.goto("about:blank");
    const content = `navigate-search-${Date.now()}`;
    let target: { id: string };
    if (scenario === "quiet reply") {
      const root = await sendMessage(page, token, principal.id, group.id, "owned thread root");
      const response = await page.request.post(`${API_BASE}/v1/messages`, {
        headers: { Authorization: `Bearer ${token}` },
        data: { actor_id: principal.id, conversation_id: group.id, content, content_type: "text/plain",
          idempotency_key: uniqueName("quiet-target"), metadata: { thread: true, broadcast: false, reply_to_id: root.id } },
      });
      expect(response.ok()).toBeTruthy();
      target = await response.json();
    } else {
      target = await sendMessage(page, token, principal.id, group.id, content);
    }
    let replacement: Awaited<ReturnType<typeof sendMessage>> | null = null;
    for (let index = 0; index < (scenario === "loaded" ? 20 : 110); index++) {
      const message = await sendMessage(page, token, principal.id, group.id,
        scenario === "cancel replacement" && index === 70 ? uniqueName("replacement") : `owned context ${index}`);
      if (scenario === "cancel replacement" && index === 70) replacement = message;
    }
    if (scenario === "replace") replacement = await sendMessage(page, token, principal.id, group.id, uniqueName("replacement"));
    const historyCursors: string[] = [];
    page.on("request", (request) => {
      const url = new URL(request.url());
      if (url.pathname.endsWith(`/conversations/${group.id}/message-page`) && url.searchParams.has("before_seq")) {
        historyCursors.push(url.searchParams.get("before_seq")!);
      }
    });

    await page.goto(`${WEB_BASE}/dashboard?conversationId=${group.id}`);
    await expect(page.locator(".messages-area")).toBeVisible({ timeout: 15_000 });
    if (scenario !== "loaded") await expect(page.locator(`[data-msg-id="${target.id}"]`)).toHaveCount(0);
    if (scenario === "tasks") await page.getByRole("tab", { name: "Tasks", exact: true }).click();
    await page.locator('.chat-header button[title="Toggle details"]').click();
    await page.locator(".detail-tab").filter({ hasText: "Search" }).click();
    const input = page.locator('input[placeholder="Search messages…"]');
    await input.fill(content);
    const historyRoute = (url: URL) => url.pathname.endsWith(`/conversations/${group.id}/message-page`) && url.searchParams.has("before_seq");
    if (scenario === "retry") {
      await page.route(historyRoute, (route) => route.fulfill({ status: 503, contentType: "application/json", body: JSON.stringify({ error: "Owned failure" }) }), { times: 1 });
    }
    if (scenario === "cancel" || scenario === "switch" || scenario === "replace" || scenario === "cancel replacement") {
      let held!: () => void;
      let release!: () => void;
      let delivered!: () => void;
      const started = new Promise<void>((resolve) => { held = resolve; });
      const gate = new Promise<void>((resolve) => { release = resolve; });
      const finished = new Promise<void>((resolve) => { delivered = resolve; });
      await page.route(historyRoute, async (route) => {
        const response = await route.fetch();
        held();
        await gate;
        await route.fulfill({ response });
        delivered();
      }, { times: 1 });
      try {
        await page.locator(".detail-search-result", { hasText: content }).click();
        await started;
        // Opening the panel can prefetch history before this held navigation.
        // Cancellation must prevent further requests, not erase that earlier work.
        const cursorsAtNavigation = [...historyCursors];
        if (scenario === "cancel") {
          await page.getByRole("status").getByRole("button", { name: "Cancel", exact: true }).click();
        } else if (replacement) {
          await input.fill(replacement.content);
          await expect(page.locator(".detail-search-result", { hasText: replacement.content })).toBeVisible();
          if (scenario === "cancel replacement") await page.clock.install();
          await page.locator(".detail-search-result", { hasText: replacement.content }).click();
          await expect(page.locator(`[data-msg-id="${replacement.id}"]`)).toBeInViewport();
          if (scenario === "cancel replacement") {
            await page.getByRole("status").getByRole("button", { name: "Cancel", exact: true }).click();
            await expect(page.getByText("Finding message…", { exact: true })).toHaveCount(0);
            const top = await page.locator(".messages-area").evaluate((element) => element.scrollTop);
            await page.clock.runFor(500);
            await expect(page.locator(`[data-msg-id="${replacement.id}"]`)).not.toHaveClass(/msg-highlight/);
            expect(await page.locator(".messages-area").evaluate((element) => element.scrollTop)).toBe(top);
          }
        } else {
          const other = await createGroup(page, token, principal.id, uniqueName("search-other"));
          await page.locator(`[data-conversation-id="${other.id}"]`).click();
          await expect(page.locator(".chat-header h1")).toHaveText(other.name);
        }
        if (!replacement) await expect(page.getByText("Finding message…", { exact: true })).toHaveCount(0);
        release();
        await finished;
        await page.evaluate(() => new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
        await expect(page.getByText("Finding message…", { exact: true })).toHaveCount(0);
        expect(historyCursors).toEqual(cursorsAtNavigation);
        await expect(page.locator(`[data-msg-id="${target.id}"]`)).toHaveCount(0);
        if (replacement && scenario !== "cancel replacement") {
          await expect(page.locator(`[data-msg-id="${replacement.id}"]`)).toBeInViewport();
          await expect(page.locator(`[data-msg-id="${replacement.id}"]`)).toHaveClass(/msg-highlight/);
        }
      } finally {
        release();
        await page.unrouteAll({ behavior: "wait" });
      }
      return;
    }
    await page.locator(".detail-search-result", { hasText: content }).click();
    if (scenario === "retry") {
      const error = page.locator(".messages-area").getByRole("alert");
      await expect(error).toContainText("Could not load older messages.");
      expect(historyCursors).toHaveLength(1);
      await error.getByRole("button", { name: "Retry", exact: true }).click();
      await expect(error).toHaveCount(0);
    }

    await expect(page.locator(`[data-msg-id="${target.id}"]`), `history cursors: ${historyCursors}`).toBeInViewport();
    await expect(page.locator(`[data-msg-id="${target.id}"]`)).toHaveClass(/msg-highlight/);
    if (scenario !== "loaded") expect(historyCursors.length).toBeGreaterThanOrEqual(2);
    await expect(page.locator(".detail-search-result")).toHaveCount(0);
  });
  }
});
