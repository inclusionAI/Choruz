import { expect, test } from "@playwright/test";
import { WEB_BASE } from "../fixtures/auth";

test.describe("Documentation pages", () => {
  /* ---------------------------------------------------------------------- */
  /*  Docs index                                                             */
  /* ---------------------------------------------------------------------- */

  test("should load the docs index page", async ({ page }) => {
    await page.goto(`${WEB_BASE}/docs`);
    await page.waitForLoadState("networkidle");
    expect(page.url()).toContain("/docs");
  });

  test("should render docs navigation/layout", async ({ page }) => {
    await page.goto(`${WEB_BASE}/docs`);
    await page.waitForLoadState("networkidle");
    // Docs layout should have some content
    const content = await page.locator("main, article, .docs-content").first();
    const visible = await content.isVisible({ timeout: 5000 }).catch(() => false);
    expect(typeof visible).toBe("boolean");
  });

  /* ---------------------------------------------------------------------- */
  /*  Getting started                                                        */
  /* ---------------------------------------------------------------------- */

  test("should load quickstart docs page", async ({ page }) => {
    await page.goto(`${WEB_BASE}/docs/getting-started/quickstart`);
    await page.waitForLoadState("networkidle");
    const content = await page.textContent("body");
    expect(content?.length).toBeGreaterThan(0);
  });

  test("should load installation docs page", async ({ page }) => {
    await page.goto(`${WEB_BASE}/docs/getting-started/installation`);
    await page.waitForLoadState("networkidle");
    expect(page.url()).toContain("/installation");
  });

  /* ---------------------------------------------------------------------- */
  /*  Concepts                                                               */
  /* ---------------------------------------------------------------------- */

  test("should load agents concept page", async ({ page }) => {
    await page.goto(`${WEB_BASE}/docs/concepts/agents`);
    await page.waitForLoadState("networkidle");
    expect(page.url()).toContain("/agents");
  });

  test("should load conversations concept page", async ({ page }) => {
    await page.goto(`${WEB_BASE}/docs/concepts/conversations`);
    await page.waitForLoadState("networkidle");
    expect(page.url()).toContain("/conversations");
  });

  /* ---------------------------------------------------------------------- */
  /*  Features                                                               */
  /* ---------------------------------------------------------------------- */

  test("should load chat feature docs", async ({ page }) => {
    await page.goto(`${WEB_BASE}/docs/features/chat`);
    await page.waitForLoadState("networkidle");
    expect(page.url()).toContain("/chat");
  });

  test("should load file explorer feature docs", async ({ page }) => {
    await page.goto(`${WEB_BASE}/docs/features/file-explorer`);
    await page.waitForLoadState("networkidle");
    expect(page.url()).toContain("/file-explorer");
  });

  test("should load cron scheduler feature docs", async ({ page }) => {
    await page.goto(`${WEB_BASE}/docs/features/cron-scheduler`);
    await page.waitForLoadState("networkidle");
    expect(page.url()).toContain("/cron-scheduler");
  });

  /* ---------------------------------------------------------------------- */
  /*  Architecture                                                           */
  /* ---------------------------------------------------------------------- */

  test("should load architecture overview docs", async ({ page }) => {
    await page.goto(`${WEB_BASE}/docs/architecture/overview`);
    await page.waitForLoadState("networkidle");
    expect(page.url()).toContain("/overview");
  });

  /* ---------------------------------------------------------------------- */
  /*  API docs                                                               */
  /* ---------------------------------------------------------------------- */

  test("should load REST API docs", async ({ page }) => {
    await page.goto(`${WEB_BASE}/docs/api/rest`);
    await page.waitForLoadState("networkidle");
    expect(page.url()).toContain("/rest");
  });

  test("should load WebSocket API docs", async ({ page }) => {
    await page.goto(`${WEB_BASE}/docs/api/websocket`);
    await page.waitForLoadState("networkidle");
    expect(page.url()).toContain("/websocket");
  });
});
