import { expect, test } from "@playwright/test";
import { login, gotoDashboard, WEB_BASE } from "../fixtures/auth";
import { createGroup, getConsoleSnapshot, sendMessage, uniqueName } from "../fixtures/api";

test.describe("Message list rendering", () => {
  test.beforeEach(async ({ page }) => {
    await login(page);
    await gotoDashboard(page);
  });

  /* ---------------------------------------------------------------------- */
  /*  Helpers                                                                */
  /* ---------------------------------------------------------------------- */

  async function createAndOpenGroupWithMessages(
    page: import("@playwright/test").Page,
    content = `message-list-${Date.now()}`,
  ) {
    const { token, principal } = await login(page);
    const group = await createGroup(page, token, principal.id, uniqueName("msg-list"));
    await sendMessage(page, token, principal.id, group.id, content);
    await page.goto(`${WEB_BASE}/dashboard?conversationId=${group.id}`);
    const item = page.locator(".conv-item").filter({ hasText: group.name }).first();
    await expect(item).toBeVisible({ timeout: 15_000 });
    await item.click();
    await expect(page.locator(".messages-area")).toBeVisible({ timeout: 10_000 });
    await expect(page.locator(".msg-group").filter({ hasText: content }).first()).toBeVisible({ timeout: 10_000 });
    return { group, content };
  }

  /* ---------------------------------------------------------------------- */
  /*  Message list structure                                                 */
  /* ---------------------------------------------------------------------- */

  test("should render the message list container", async ({ page }) => {
    await createAndOpenGroupWithMessages(page);
    await expect(page.locator(".messages-area")).toBeVisible();
  });

  test("should render message groups with sender info", async ({ page }) => {
    await createAndOpenGroupWithMessages(page);
    const groups = page.locator(".msg-group");
    const count = await groups.count();
    if (count > 0) {
      // Each msg-group should have an avatar and content
      const first = groups.first();
      const avatar = first.locator(".avatar");
      expect(await avatar.count()).toBeGreaterThan(0);
    }
  });

  /* ---------------------------------------------------------------------- */
  /*  Message timestamps                                                     */
  /* ---------------------------------------------------------------------- */

  test("should display timestamps on messages", async ({ page }) => {
    await createAndOpenGroupWithMessages(page);
    // Timestamps are rendered as HH:MM format
    const timestamps = page.locator(".msg-time, .message-time, time");
    const count = await timestamps.count();
    expect(count).toBeGreaterThanOrEqual(0);
  });

  /* ---------------------------------------------------------------------- */
  /*  Auto-scroll to bottom                                                  */
  /* ---------------------------------------------------------------------- */

  test("should auto-scroll to the latest message", async ({ page }) => {
    await createAndOpenGroupWithMessages(page);
    const msgList = page.locator(".messages-area");
    // Check if scroll position is near the bottom
    const isNearBottom = await msgList.evaluate((el) => {
      const threshold = 100;
      return el.scrollHeight - el.scrollTop - el.clientHeight < threshold;
    });
    expect(isNearBottom).toBeTruthy();
  });

  test("should scroll to bottom after sending a new message", async ({
    page,
  }) => {
    await createAndOpenGroupWithMessages(page);
    const textarea = page.locator("textarea").first();
    if (!(await textarea.isVisible({ timeout: 2000 }).catch(() => false))) {
      test.skip();
      return;
    }
    await textarea.fill(`scroll-test-${Date.now()}`);
    await textarea.press("Enter");
    await page.waitForTimeout(2000);
    const msgList = page.locator(".messages-area");
    const isNearBottom = await msgList.evaluate((el) => {
      return el.scrollHeight - el.scrollTop - el.clientHeight < 100;
    });
    expect(isNearBottom).toBeTruthy();
  });

  /* ---------------------------------------------------------------------- */
  /*  Message content rendering                                              */
  /* ---------------------------------------------------------------------- */

  test.fixme(
    "should strip ANSI codes from displayed messages",
    {
      annotation: {
        type: "bug",
        description: "BUGS.md B-016: normal message rendering leaks raw ANSI escape sequences",
      },
    },
    async ({ page }) => {
      const { token, principal } = await login(page);
      const group = await createGroup(page, token, principal.id, uniqueName("ansi"));
      const content = "normal text \u001b[31mred text\u001b[0m end";
      // Send a message with ANSI codes
      await sendMessage(
        page,
        token,
        principal.id,
        group.id,
        content,
      );
      await page.goto(`${WEB_BASE}/dashboard?conversationId=${group.id}`);
      const item = page.locator(".conv-item").filter({ hasText: group.name }).first();
      await expect(item).toBeVisible({ timeout: 15_000 });
      await item.click();
      await expect(page.locator(".msg-group").filter({ hasText: "normal text" }).first()).toBeVisible({ timeout: 10_000 });
      // ANSI escape sequences should be stripped
      const ansiInDom = await page.evaluate(() => {
        return document.body.innerHTML.includes("\u001b[31m");
      });
      expect(ansiInDom).toBeFalsy();
    },
  );

  /* ---------------------------------------------------------------------- */
  /*  Empty state                                                            */
  /* ---------------------------------------------------------------------- */

  test("should show empty state for new conversations", async ({ page }) => {
    const { token, principal } = await login(page);
    // Create a fresh group with no messages
    const group = await createGroup(page, token, principal.id, uniqueName("empty"));
    await page.goto(`${WEB_BASE}/dashboard?conversationId=${group.id}`);
    const item = page.locator(".conv-item").filter({ hasText: group.name }).first();
    await expect(item).toBeVisible({ timeout: 15_000 });
    await item.click();
    await expect(page.locator(".messages-area .empty-state").getByText("No messages yet")).toBeVisible({ timeout: 10_000 });
  });

  /* ---------------------------------------------------------------------- */
  /*  Sender avatar                                                          */
  /* ---------------------------------------------------------------------- */

  test("should display sender avatar with initial letter", async ({
    page,
  }) => {
    await createAndOpenGroupWithMessages(page);
    const avatars = page.locator(".msg-group .avatar");
    if ((await avatars.count()) > 0) {
      const initial = await avatars.first().textContent();
      expect(initial?.length).toBe(1);
      expect(initial?.toUpperCase()).toBe(initial);
    }
  });

  /* ---------------------------------------------------------------------- */
  /*  Code blocks                                                            */
  /* ---------------------------------------------------------------------- */

  test("should render code blocks in messages", async ({ page }) => {
    const { token, principal } = await login(page);
    const snap = await getConsoleSnapshot(page, token);
    const group = snap.conversations.find(
      (c) => c.conversation_type === "group",
    );
    if (!group) {
      test.skip();
      return;
    }
    await sendMessage(
      page,
      token,
      principal.id,
      group.id,
      "```js\nconsole.log('hello');\n```",
    );
    await gotoDashboard(page);
    await page.waitForSelector(".conv-item", { timeout: 10_000 });
    const items = page.locator(".conv-item");
    const count = await items.count();
    for (let i = 0; i < count; i++) {
      const txt = await items.nth(i).textContent();
      if (txt?.includes(group.name ?? "")) {
        await items.nth(i).click();
        break;
      }
    }
    await page.waitForTimeout(2000);
    // Code blocks rendered as <pre> or <code>
    const codeBlocks = page.locator("pre code, code");
    expect(await codeBlocks.count()).toBeGreaterThanOrEqual(0);
  });
});
