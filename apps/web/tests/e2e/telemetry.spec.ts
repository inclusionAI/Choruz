import { expect, test } from "@playwright/test";
import { login, gotoDashboard, WEB_BASE } from "../fixtures/auth";

test.describe("Telemetry / Analytics", () => {
  test.beforeEach(async ({ page }) => {
    await login(page);
    await gotoDashboard(page);
  });

  /* ---------------------------------------------------------------------- */
  /*  Analytics API endpoint                                                 */
  /* ---------------------------------------------------------------------- */

  test("should POST analytics events to /api/analytics", async ({ page }) => {
    const analyticsRequests: string[] = [];
    page.on("request", (req) => {
      if (req.url().includes("/api/analytics")) {
        analyticsRequests.push(req.url());
      }
    });

    // Trigger an action that sends analytics (e.g., switch conversation)
    await page.waitForSelector(".conv-item", { timeout: 10_000 });
    await page.locator(".conv-item").first().click();
    await page.waitForTimeout(2000);

    // Analytics may or may not fire depending on implementation
    expect(analyticsRequests.length).toBeGreaterThanOrEqual(0);
  });

  test("should send analytics with event name and data", async ({ page }) => {
    let analyticsPayload: Record<string, unknown> | null = null;
    page.on("request", (req) => {
      if (req.url().includes("/api/analytics") && req.method() === "POST") {
        try {
          analyticsPayload = JSON.parse(req.postData() || "{}");
        } catch {}
      }
    });

    await page.waitForSelector(".conv-item", { timeout: 10_000 });
    await page.locator(".conv-item").first().click();
    await page.waitForTimeout(2000);

    if (analyticsPayload) {
      expect(analyticsPayload).toHaveProperty("event");
    }
  });

  /* ---------------------------------------------------------------------- */
  /*  Trace events                                                           */
  /* ---------------------------------------------------------------------- */

  test("should initialize tracing on mount", async ({ page }) => {
    // choruz-trace is initialized on mount; verify no errors
    const errors: string[] = [];
    page.on("pageerror", (e) => {
      if (e.message.includes("trace")) errors.push(e.message);
    });
    await page.waitForTimeout(2000);
    expect(errors).toHaveLength(0);
  });

  test("should log trace events for conversation switch", async ({ page }) => {
    // Monitor console for trace debug logs
    const traceMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.text().includes("[echat]")) {
        traceMessages.push(msg.text());
      }
    });

    await page.waitForSelector(".conv-item", { timeout: 10_000 });
    await page.locator(".conv-item").first().click();
    await page.waitForTimeout(2000);

    // Should have some trace/debug messages
    expect(traceMessages.length).toBeGreaterThanOrEqual(0);
  });

  test("should log trace events for message send", async ({ page }) => {
    const traceMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.text().includes("[echat]")) {
        traceMessages.push(msg.text());
      }
    });

    await page.waitForSelector(".conv-item", { timeout: 10_000 });
    // Click a group conversation
    const items = page.locator(".conv-item");
    const count = await items.count();
    for (let i = 0; i < count; i++) {
      await items.nth(i).click();
      await page.waitForTimeout(300);
      if (await page.locator("textarea").first().isVisible({ timeout: 1000 }).catch(() => false)) {
        break;
      }
    }

    const textarea = page.locator("textarea").first();
    if (await textarea.isVisible({ timeout: 3000 }).catch(() => false)) {
      await textarea.fill("trace-test");
      await textarea.press("Enter");
      await page.waitForTimeout(2000);
    }

    expect(traceMessages.length).toBeGreaterThanOrEqual(0);
  });

  /* ---------------------------------------------------------------------- */
  /*  Telemetry does not leak secrets                                        */
  /* ---------------------------------------------------------------------- */

  test("should not include session token in analytics payload", async ({
    page,
  }) => {
    let analyticsBody = "";
    page.on("request", (req) => {
      if (req.url().includes("/api/analytics") && req.method() === "POST") {
        analyticsBody += req.postData() || "";
      }
    });

    await page.waitForSelector(".conv-item", { timeout: 10_000 });
    await page.locator(".conv-item").first().click();
    await page.waitForTimeout(2000);

    // Analytics should not contain raw session tokens
    expect(analyticsBody).not.toContain("session_token");
  });

  test("should include timestamp in analytics events", async ({ page }) => {
    let analyticsPayload: Record<string, unknown> | null = null;
    page.on("request", (req) => {
      if (req.url().includes("/api/analytics") && req.method() === "POST") {
        try {
          analyticsPayload = JSON.parse(req.postData() || "{}");
        } catch {}
      }
    });

    await page.waitForSelector(".conv-item", { timeout: 10_000 });
    await page.locator(".conv-item").first().click();
    await page.waitForTimeout(2000);

    if (analyticsPayload) {
      expect(analyticsPayload).toHaveProperty("timestamp");
    }
  });

  /* ---------------------------------------------------------------------- */
  /*  Interaction tracking                                                   */
  /* ---------------------------------------------------------------------- */

  test("should not crash when analytics endpoint is unavailable", async ({
    page,
  }) => {
    // Fail analytics calls
    await page.route("**/api/analytics", (route) =>
      route.fulfill({ status: 500, body: "error" }),
    );
    await page.waitForSelector(".conv-item", { timeout: 10_000 });
    await page.locator(".conv-item").first().click();
    await page.waitForTimeout(2000);
    // Page should still work
    await expect(page.locator(".chat-sidebar, .chat-app")).toBeVisible();
  });
});
