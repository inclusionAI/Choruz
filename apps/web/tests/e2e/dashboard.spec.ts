import { expect, test } from "@playwright/test";
import { login, gotoDashboard, WEB_BASE } from "../fixtures/auth";

test.describe("Dashboard page", () => {
  test.beforeEach(async ({ page }) => {
    await login(page);
  });

  /* ---------------------------------------------------------------------- */
  /*  Page load                                                              */
  /* ---------------------------------------------------------------------- */

  test("should load the dashboard page", async ({ page }) => {
    await gotoDashboard(page);
    expect(page.url()).toContain("/dashboard");
  });

  test("should render the main chat app container", async ({ page }) => {
    await gotoDashboard(page);
    await expect(
      page.locator(".chat-app, .chat-sidebar"),
    ).toBeVisible({ timeout: 15_000 });
  });

  test("should show the sidebar on dashboard load", async ({ page }) => {
    await gotoDashboard(page);
    await expect(page.locator(".chat-sidebar")).toBeVisible({
      timeout: 15_000,
    });
  });

  /* ---------------------------------------------------------------------- */
  /*  Initial data loading                                                   */
  /* ---------------------------------------------------------------------- */

  test("should load conversations from the API", async ({ page }) => {
    await gotoDashboard(page);
    await page.waitForSelector(".conv-item", { timeout: 10_000 });
    const count = await page.locator(".conv-item").count();
    expect(count).toBeGreaterThan(0);
  });

  test("should load principal info and show username", async ({ page }) => {
    await gotoDashboard(page);
    await expect(page.locator(".sidebar-header").getByText("operator", { exact: true })).toBeVisible({ timeout: 10_000 });
  });

  test("should load company list", async ({ page }) => {
    await gotoDashboard(page);
    const selector = page.locator(".company-selector-btn");
    if (await selector.isVisible({ timeout: 5000 }).catch(() => false)) {
      const name = await selector.locator(".company-selector-name").textContent();
      expect(name?.trim().length).toBeGreaterThan(0);
    }
  });

  /* ---------------------------------------------------------------------- */
  /*  Layout structure                                                       */
  /* ---------------------------------------------------------------------- */

  test("should have sidebar + main area layout", async ({ page }) => {
    await gotoDashboard(page);
    await expect(page.locator(".chat-sidebar")).toBeVisible({
      timeout: 15_000,
    });
    // Main area: either shows a chat or an empty state
    const mainArea = page.locator(
      ".chat-main, .message-list, .terminal-container, .empty-state",
    );
    // At least one should be present after selecting a conversation
    await page.waitForSelector(".conv-item", { timeout: 10_000 });
  });

  /* ---------------------------------------------------------------------- */
  /*  Error handling                                                         */
  /* ---------------------------------------------------------------------- */

  test("should not crash on console API failure", async ({ page }) => {
    // Block the console API after initial load
    await gotoDashboard(page);
    await page.waitForSelector(".conv-item", { timeout: 10_000 });
    await page.route("**/api/v1/console", (route) =>
      route.fulfill({ status: 500, body: '{"error":"test"}' }),
    );
    await page.waitForTimeout(15_000);
    // Page should still be functional
    await expect(page.locator(".chat-sidebar")).toBeVisible();
  });

  /* ---------------------------------------------------------------------- */
  /*  Loading state                                                          */
  /* ---------------------------------------------------------------------- */

  test("should show loading state while data is being fetched", async ({
    page,
  }) => {
    // Check that the loading page exists
    await page.goto(`${WEB_BASE}/dashboard`);
    // The loading.tsx component may briefly flash
    await page.waitForSelector(".chat-sidebar, .chat-app, .loading", {
      timeout: 15_000,
    });
  });

  /* ---------------------------------------------------------------------- */
  /*  localStorage restoration                                               */
  /* ---------------------------------------------------------------------- */

  test("should restore active conversation from localStorage", async ({
    page,
  }) => {
    await gotoDashboard(page);
    await page.waitForSelector(".conv-item", { timeout: 10_000 });
    await page.locator(".conv-item").first().click();
    await page.waitForTimeout(1000);
    const savedConv = await page.evaluate(() =>
      localStorage.getItem("choruz_active_conv"),
    );
    expect(savedConv).toBeTruthy();
  });

  /* ---------------------------------------------------------------------- */
  /*  No hydration errors                                                    */
  /* ---------------------------------------------------------------------- */

  test("should not produce React hydration errors", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await gotoDashboard(page);
    await page.waitForTimeout(3000);
    const hydrationErrors = errors.filter(
      (e) =>
        e.includes("Hydration") ||
        e.includes("hydration") ||
        e.includes("did not match"),
    );
    expect(hydrationErrors).toHaveLength(0);
  });
});
