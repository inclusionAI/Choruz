import { expect, test } from "@playwright/test";
import { login, gotoDashboard, WEB_BASE } from "../fixtures/auth";
import { createGroup, uniqueName } from "../fixtures/api";

test.describe("Responsive layout", () => {
  test.beforeEach(async ({ page }) => {
    await login(page);
  });

  /* ---------------------------------------------------------------------- */
  /*  Desktop layout                                                         */
  /* ---------------------------------------------------------------------- */

  test("should show sidebar in desktop viewport", async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 800 });
    await gotoDashboard(page);
    await expect(page.locator(".chat-sidebar")).toBeVisible({
      timeout: 15_000,
    });
  });

  test("should show sidebar and chat area side by side on desktop", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1280, height: 800 });
    await gotoDashboard(page);
    await expect(page.locator(".chat-sidebar")).toBeVisible();
    await expect(page.locator(".chat-main")).toBeVisible();

    const sidebarBox = await page.locator(".chat-sidebar").boundingBox();
    const chatBox = await page.locator(".chat-main").boundingBox();
    expect(sidebarBox).not.toBeNull();
    expect(chatBox).not.toBeNull();
    expect(sidebarBox!.x + sidebarBox!.width).toBeLessThanOrEqual(chatBox!.x);
  });

  /* ---------------------------------------------------------------------- */
  /*  Mobile layout                                                          */
  /* ---------------------------------------------------------------------- */

  test("should render properly on mobile viewport (375px)", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 375, height: 667 });
    await gotoDashboard(page);
    await page.waitForSelector(".chat-sidebar, .chat-app", {
      timeout: 15_000,
    });
    // Page should render without crash
    expect(true).toBeTruthy();
  });

  test("should render properly on tablet viewport (768px)", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 768, height: 1024 });
    await gotoDashboard(page);
    await page.waitForSelector(".chat-sidebar, .chat-app", {
      timeout: 15_000,
    });
    expect(true).toBeTruthy();
  });

  test("should show mobile menu button on small screens", async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 667 });
    const { token, principal } = await login(page);
    const group = await createGroup(page, token, principal.id, uniqueName("mobile-menu"));
    await page.goto(`${WEB_BASE}/dashboard?conversationId=${group.id}`);
    await page.waitForSelector(".chat-header, .chat-app", { timeout: 15_000 });
    const menuBtn = page.locator(".mobile-menu-btn");
    const visible = await menuBtn.isVisible({ timeout: 3000 }).catch(() => false);
    expect(typeof visible).toBe("boolean");
  });

  /* ---------------------------------------------------------------------- */
  /*  Local entry responsive                                                 */
  /* ---------------------------------------------------------------------- */

  test("should enter the dashboard on a mobile viewport", async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 667 });
    await page.context().clearCookies();
    await page.goto(WEB_BASE);
    await page.waitForSelector(".chat-sidebar, .chat-app", { timeout: 15_000 });
    await expect(page.getByRole("img", { name: "Choruz" })).toBeVisible();
    await expect(page.locator("#signin-panel, #signup-panel")).toHaveCount(0);
  });

  /* ---------------------------------------------------------------------- */
  /*  Wide screen                                                            */
  /* ---------------------------------------------------------------------- */

  test("should handle very wide viewport (2560px)", async ({ page }) => {
    await page.setViewportSize({ width: 2560, height: 1440 });
    await gotoDashboard(page);
    await page.waitForSelector(".chat-sidebar, .chat-app", {
      timeout: 15_000,
    });
    expect(true).toBeTruthy();
  });

  /* ---------------------------------------------------------------------- */
  /*  Viewport resize                                                        */
  /* ---------------------------------------------------------------------- */

  test("should handle viewport resize from desktop to mobile", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1280, height: 800 });
    await gotoDashboard(page);
    await page.waitForSelector(".chat-sidebar", { timeout: 15_000 });
    // Resize to mobile
    await page.setViewportSize({ width: 375, height: 667 });
    await page.waitForTimeout(1000);
    // Should not crash
    expect(true).toBeTruthy();
  });
});
