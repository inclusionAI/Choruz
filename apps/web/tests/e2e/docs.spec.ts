import { expect, test } from "@playwright/test";
import { WEB_BASE } from "../fixtures/auth";

test("public documentation routes return their expected content, not a rendered 404", async ({ page }) => {
  const routes = [
    ["", "Run coding agents as a team"],
    ["/getting-started/quickstart", "Your first 10 minutes"],
    ["/getting-started/installation", "Install and run Choruz"],
    ["/agents/drivers", "Supported CLIs"],
    ["/concepts/conversations", "Conversations"],
    ["/features/chat", "Chat & Messaging"],
    ["/features/file-explorer", "Files"],
    ["/features/cron-scheduler", "Cron Scheduler"],
    ["/api/rest", "REST Endpoints"],
    ["/api/websocket", "WebSocket Events"],
  ];
  for (const [route, heading] of routes) {
    await test.step(route || "index", async () => {
      const response = await page.goto(`${WEB_BASE}/docs${route}`);
      expect(response?.status()).toBe(200);
      await expect(page.getByRole("heading", { level: 1, name: heading, exact: true })).toBeVisible();
    });
  }
});
