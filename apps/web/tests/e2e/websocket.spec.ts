import { expect, test } from "@playwright/test";
import { login, gotoDashboard, WEB_BASE } from "../fixtures/auth";
import { createAndOpenGroup, createGroup, sendMessage, uniqueName } from "../fixtures/api";

async function readAcknowledgedSyncCursor(page: import("@playwright/test").Page, principalId: string) {
  return page.evaluate(async (id) => new Promise<number>((resolve, reject) => {
    const request = indexedDB.open("choruz_messages");
    request.onerror = () => reject(request.error);
    request.onsuccess = () => {
      const db = request.result;
      const transaction = db.transaction("syncState", "readonly");
      const getRequest = transaction.objectStore("syncState").get(id);
      getRequest.onerror = () => reject(getRequest.error);
      getRequest.onsuccess = () => resolve(getRequest.result?.ack_cursor ?? 0);
    };
  }), principalId);
}

test.describe("durable dashboard sync", () => {
  test("opens an acknowledged sync WebSocket", async ({ page }) => {
    const syncSocket = page.waitForEvent("websocket", {
      predicate: (ws) => ws.url().includes("/v1/ws/sync"),
      timeout: 10_000,
    });
    const { token, principal } = await login(page);
    await createAndOpenGroup(page, token, principal.id, "ws-connect");

    const ws = await syncSocket;
    expect(ws.url()).toContain("device_id=");
    expect(ws.url()).toContain("cursor=");
  });

  test("delivers API messages in real time", async ({ page }) => {
    const { token, principal } = await login(page);
    const group = await createAndOpenGroup(page, token, principal.id, "ws-realtime");
    const content = `ws-realtime-${Date.now()}`;
    await sendMessage(page, token, principal.id, group.id, content);
    await expect(page.locator(".messages-area").getByText(content)).toBeVisible({ timeout: 15_000 });
  });

  test("converges on one message in two tabs", async ({ page, context }) => {
    const { token, principal } = await login(page);
    const group = await createGroup(page, token, principal.id, uniqueName("ws-two-tabs"));
    await context.addInitScript(() => {
      const state = window as typeof window & { __choruzSyncSocketOpened?: boolean };
      const NativeWebSocket = window.WebSocket;
      state.__choruzSyncSocketOpened = false;
      window.WebSocket = function trackedSyncWebSocket(url: string | URL, protocols?: string | string[]) {
        const socket = protocols === undefined ? new NativeWebSocket(url) : new NativeWebSocket(url, protocols);
        if (String(url).includes("/v1/ws/sync")) {
          socket.addEventListener("open", () => { state.__choruzSyncSocketOpened = true; });
        }
        return socket;
      } as typeof WebSocket;
      window.WebSocket.prototype = NativeWebSocket.prototype;
    });
    const secondPage = await context.newPage();
    try {
      await Promise.all([
        page.goto(`${WEB_BASE}/dashboard?conversationId=${group.id}`),
        secondPage.goto(`${WEB_BASE}/dashboard?conversationId=${group.id}`),
      ]);
      await Promise.all([page, secondPage].map((candidate) =>
        expect.poll(() => candidate.evaluate(() =>
          (window as typeof window & { __choruzSyncSocketOpened?: boolean }).__choruzSyncSocketOpened,
        ), { timeout: 15_000 }).toBe(true),
      ));
      const content = `two-tab-${Date.now()}`;
      await sendMessage(page, token, principal.id, group.id, content);
      await Promise.all([page, secondPage].map((candidate) =>
        expect(candidate.locator(".msg-group", { hasText: content })).toHaveCount(1, { timeout: 15_000 }),
      ));
    } finally {
      await secondPage.close();
    }
  });

  test("reconnects without starting console or transcript polling", async ({ page }) => {
    await page.addInitScript(() => {
      const NativeWebSocket = window.WebSocket;
      window.WebSocket = function blockedSyncWebSocket(url: string | URL, protocols?: string | string[]) {
        if (String(url).includes("/v1/ws/sync")) throw new Error("blocked sync socket");
        return protocols === undefined ? new NativeWebSocket(url) : new NativeWebSocket(url, protocols);
      } as typeof WebSocket;
      window.WebSocket.prototype = NativeWebSocket.prototype;
    });
    const { token, principal } = await login(page);
    const group = await createGroup(page, token, principal.id, uniqueName("ws-reconnect"));
    await gotoDashboard(page);
    await page.locator(".conv-item").filter({ hasText: group.name }).first().click();
    await expect(page.locator("textarea").first()).toBeVisible({ timeout: 10_000 });

    const backgroundReads: string[] = [];
    page.on("request", (request) => {
      if (request.method() === "GET" && (request.url().includes("/v1/console") || request.url().includes("/message-page"))) {
        backgroundReads.push(request.url());
      }
    });
    await expect(page.getByText("Reconnecting…")).toBeVisible({ timeout: 10_000 });
    await page.waitForTimeout(5_000);
    expect(backgroundReads).toEqual([]);
  });

  test("does not mark own messages in another conversation unread", async ({ page }) => {
    const { token, principal } = await login(page);
    const first = await createGroup(page, token, principal.id, uniqueName("ws-unread-a"));
    const second = await createGroup(page, token, principal.id, uniqueName("ws-unread-b"));
    await gotoDashboard(page);
    await page.locator(".conv-item").filter({ hasText: first.name }).first().click();
    const cursorBeforeSend = await readAcknowledgedSyncCursor(page, principal.id);
    const content = `own-${Date.now()}`;
    await sendMessage(page, token, principal.id, second.id, content);
    const inactive = page.locator(".conv-item").filter({ hasText: second.name }).first();
    await expect(inactive).toBeVisible({ timeout: 10_000 });
    await expect(inactive).toContainText(content, { timeout: 15_000 });
    await expect.poll(
      () => readAcknowledgedSyncCursor(page, principal.id),
      { timeout: 15_000 },
    ).toBeGreaterThan(cursorBeforeSend);
    await expect(inactive.locator(".conv-unread")).toHaveCount(0);
  });

  test("deduplicates optimistic messages with sync confirmations", async ({ page }) => {
    const { token, principal } = await login(page);
    await createAndOpenGroup(page, token, principal.id, "ws-dedup");
    const cursorBeforeSend = await readAcknowledgedSyncCursor(page, principal.id);
    const content = `dedup-ws-${Date.now()}`;
    await page.locator("textarea").first().fill(content);
    await page.locator("textarea").first().press("Enter");
    await expect(page.locator(".msg-group", { hasText: content })).toHaveCount(1, { timeout: 10_000 });
    await expect.poll(
      () => readAcknowledgedSyncCursor(page, principal.id),
      { timeout: 15_000 },
    ).toBeGreaterThan(cursorBeforeSend);
    await expect(page.locator(".msg-group", { hasText: content })).toHaveCount(1);
  });

  test("preserves messages without legacy console polls", async ({ page }) => {
    const { token, principal } = await login(page);
    await createAndOpenGroup(page, token, principal.id, "ws-no-console");
    const content = `persist-${Date.now()}`;
    await page.locator("textarea").first().fill(content);
    await page.locator("textarea").first().press("Enter");
    await expect(page.locator(".messages-area").getByText(content)).toBeVisible();
    const consoleRequests: string[] = [];
    page.on("request", (request) => {
      if (request.url().includes("/v1/console")) consoleRequests.push(request.url());
    });
    await page.waitForTimeout(5_000);
    await expect(page.locator(".messages-area").getByText(content)).toBeVisible();
    expect(consoleRequests).toEqual([]);
  });
});
