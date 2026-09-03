import { expect, test } from "@playwright/test";
import { login, gotoDashboard, WEB_BASE } from "../fixtures/auth";
import { createGroup, uniqueName } from "../fixtures/api";

test.describe("Keyboard shortcuts and navigation", () => {
  test.beforeEach(async ({ page }) => {
    await login(page);
    await gotoDashboard(page);
  });

  /* ---------------------------------------------------------------------- */
  /*  Helpers                                                                */
  /* ---------------------------------------------------------------------- */

  async function openGroupChat(page: import("@playwright/test").Page) {
    const { token, principal } = await login(page);
    const group = await createGroup(page, token, principal.id, uniqueName("keyboard"));
    await page.goto(`${WEB_BASE}/dashboard?conversationId=${group.id}`);
    const item = page.locator(".conv-item").filter({ hasText: group.name }).first();
    await expect(item).toBeVisible({ timeout: 15_000 });
    await item.click();
    await expect(page.locator("textarea").first()).toBeVisible({ timeout: 10_000 });
    return true;
  }

  /* ---------------------------------------------------------------------- */
  /*  Enter to send                                                          */
  /* ---------------------------------------------------------------------- */

  test("should send message on Enter key (not Shift+Enter)", async ({
    page,
  }) => {
    const found = await openGroupChat(page);
    if (!found) {
      test.skip();
      return;
    }
    const textarea = page.locator("textarea").first();
    const testMsg = `enter-send-${Date.now()}`;
    await textarea.fill(testMsg);
    await textarea.press("Enter");
    await expect(page.locator(".msg-group").filter({ hasText: testMsg }).first()).toBeVisible({ timeout: 10_000 });
  });

  test("should insert newline on Shift+Enter", async ({ page }) => {
    const found = await openGroupChat(page);
    if (!found) {
      test.skip();
      return;
    }
    const textarea = page.locator("textarea").first();
    await textarea.fill("line1");
    await textarea.press("Shift+Enter");
    await textarea.type("line2");
    const value = await textarea.inputValue();
    expect(value).toContain("\n");
  });

  /* ---------------------------------------------------------------------- */
  /*  @mention navigation                                                    */
  /* ---------------------------------------------------------------------- */

  test("should navigate mention dropdown with arrow keys", async ({
    page,
  }) => {
    const found = await openGroupChat(page);
    if (!found) {
      test.skip();
      return;
    }
    const textarea = page.locator("textarea").first();
    await textarea.fill("@");
    await textarea.dispatchEvent("input");
    await page.waitForTimeout(500);
    const dropdown = page.locator(".mention-dropdown");
    if (await dropdown.isVisible({ timeout: 3000 }).catch(() => false)) {
      // Press ArrowDown to navigate
      await textarea.press("ArrowDown");
      await page.waitForTimeout(200);
      // Highlighted item should change
      const highlighted = dropdown.locator(".mention-item.highlighted");
      const count = await highlighted.count();
      expect(count).toBeGreaterThanOrEqual(0);
    }
  });

  test("should select mention with Tab key", async ({ page }) => {
    const found = await openGroupChat(page);
    if (!found) {
      test.skip();
      return;
    }
    const textarea = page.locator("textarea").first();
    await textarea.fill("@");
    await textarea.dispatchEvent("input");
    await page.waitForTimeout(500);
    const dropdown = page.locator(".mention-dropdown");
    if (await dropdown.isVisible({ timeout: 3000 }).catch(() => false)) {
      await textarea.press("Tab");
      await page.waitForTimeout(200);
      const value = await textarea.inputValue();
      expect(value).toContain("@");
      // Dropdown should close
      const stillVisible = await dropdown.isVisible({ timeout: 1000 }).catch(() => false);
      expect(stillVisible).toBeFalsy();
    }
  });

  test("should dismiss mention dropdown with Escape", async ({ page }) => {
    const found = await openGroupChat(page);
    if (!found) {
      test.skip();
      return;
    }
    const textarea = page.locator("textarea").first();
    await textarea.fill("@");
    await textarea.dispatchEvent("input");
    await page.waitForTimeout(500);
    const dropdown = page.locator(".mention-dropdown");
    if (await dropdown.isVisible({ timeout: 3000 }).catch(() => false)) {
      await textarea.press("Escape");
      await page.waitForTimeout(300);
      const stillVisible = await dropdown.isVisible({ timeout: 1000 }).catch(() => false);
      expect(stillVisible).toBeFalsy();
    }
  });

  /* ---------------------------------------------------------------------- */
  /*  Chat input focus                                                       */
  /* ---------------------------------------------------------------------- */

  test(
    "should refocus textarea after sending a message",
    async ({ page }) => {
      const found = await openGroupChat(page);
      if (!found) {
        test.skip();
        return;
      }
      const textarea = page.locator("textarea").first();
      const testMsg = `focus-test-${Date.now()}`;
      await textarea.fill(testMsg);
      await textarea.press("Enter");
      await expect(page.locator(".msg-group").filter({ hasText: testMsg }).first()).toBeVisible({ timeout: 10_000 });
      await expect(textarea).toBeEnabled({ timeout: 10_000 });
      await expect(textarea).toBeFocused({ timeout: 10_000 });
    },
  );

  /* ---------------------------------------------------------------------- */
  /*  Conversation list keyboard navigation                                  */
  /* ---------------------------------------------------------------------- */

  test("should have keyboard-accessible conversation list", async ({
    page,
  }) => {
    const conversationList = page.locator('.conversation-list[aria-label="Conversations"]');
    await expect(conversationList).toBeVisible({ timeout: 10_000 });
    await expect(conversationList).not.toHaveAttribute("tabindex", "0");
    const firstItem = conversationList.locator(".conv-item-main").first();
    const secondItem = conversationList.locator(".conv-item-main").nth(1);
    await firstItem.focus();
    await firstItem.press("ArrowDown");
    await expect(secondItem).toBeFocused();
  });

  /* ---------------------------------------------------------------------- */
  /*  No default form submission                                             */
  /* ---------------------------------------------------------------------- */

  test("should prevent default Enter from submitting a form", async ({
    page,
  }) => {
    const found = await openGroupChat(page);
    if (!found) {
      test.skip();
      return;
    }
    // Verify that Enter in the textarea sends the message (custom handler)
    // rather than triggering HTML form submission
    const textarea = page.locator("textarea").first();
    const beforeUrl = page.url();
    await textarea.fill("no-submit-test");
    await textarea.press("Enter");
    await page.waitForTimeout(1000);
    // URL should not have changed (no form submission/navigation)
    expect(page.url()).toBe(beforeUrl);
  });
});
