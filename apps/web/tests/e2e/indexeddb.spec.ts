import { expect, test } from "@playwright/test";
import { expandSidebarConversationSections, login, gotoDashboard, signup, API_BASE, WEB_BASE } from "../fixtures/auth";
import { createCompany, createGroup, sendMessage, uniqueName } from "../fixtures/api";

test("returning to the dashboard merges delayed cached history without replacing its newer preview", async ({ page }) => {
  const { token, principal } = await signup(page, uniqueName("cache-merge"), "Owned-password-123!");
  const company = await createCompany(page, token, principal.id, uniqueName("Cache owner"));
  let releaseHistory = () => {};
  try {
    const group = await createGroup(page, token, principal.id, uniqueName("Cache history"), [], company.id);
    const oldMessages = await Promise.all(["first", "second"].map((content) =>
      sendMessage(page, token, principal.id, group.id, content)));
    await gotoDashboard(page);
    await page.locator(".company-selector-btn").click();
    await page.locator(".company-dropdown-item-name").filter({ hasText: company.name }).click();
    await page.goto(`${WEB_BASE}/docs`);
    await page.evaluate(async (messages) => {
      const db = await new Promise<IDBDatabase>((resolve, reject) => {
        const request = indexedDB.open("choruz_messages");
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
      });
      try {
        await new Promise<void>((resolve, reject) => {
          const transaction = db.transaction("messages", "readwrite");
          transaction.oncomplete = () => resolve();
          transaction.onerror = () => reject(transaction.error);
          for (const message of messages) transaction.objectStore("messages").put(message);
        });
      } finally { db.close(); }
    }, oldMessages);
    const latest = `new-${crypto.randomUUID().slice(0, 6)}`;
    await sendMessage(page, token, principal.id, group.id, latest);
    await page.addInitScript(() => {
      const browser = window as typeof window & { releaseMessageCache?: () => void; messageCacheHeld?: boolean };
      const open = indexedDB.open.bind(indexedDB);
      const descriptor = Object.getOwnPropertyDescriptor(IDBRequest.prototype, "onsuccess")!;
      let released = false;
      const waiting: Array<() => void> = [];
      browser.releaseMessageCache = () => { released = true; waiting.splice(0).forEach((resume) => resume()); };
      indexedDB.open = ((name: string, version?: number) => {
        const request = version === undefined ? open(name) : open(name, version);
        if (name === "choruz_messages") Object.defineProperty(request, "onsuccess", {
          set(handler: ((this: IDBRequest, event: Event) => void) | null) {
            descriptor.set!.call(request, handler && function (event: Event) {
              if (released) handler.call(request, event);
              else {
                browser.messageCacheHeld = true;
                waiting.push(() => handler.call(request, event));
              }
            });
          },
        });
        return request;
      }) as typeof indexedDB.open;
    });
    await gotoDashboard(page);
    const item = page.locator(`[data-conversation-id="${group.id}"]`);
    await expect(item.locator(".conv-preview")).toContainText(latest);
    await expect.poll(() => page.evaluate(() => (window as typeof window & { messageCacheHeld?: boolean }).messageCacheHeld)).toBe(true);
    await page.evaluate(() => (window as typeof window & { releaseMessageCache: () => void }).releaseMessageCache());
    let historyRequested!: () => void;
    const requested = new Promise<void>((resolve) => { historyRequested = resolve; });
    const historyBarrier = new Promise<void>((resolve) => { releaseHistory = resolve; });
    await page.route(`**/api/v1/conversations/${group.id}/message-page?*`, async (route) => {
      historyRequested();
      await historyBarrier;
      await route.continue();
    });
    await item.click();
    await requested;
    await expect(page.locator(".messages-area")).toContainText("first");
    await expect(page.locator(".messages-area")).toContainText("second");
    await expect(page.locator(".messages-area")).toContainText(latest);
  } finally {
    releaseHistory();
    await page.close();
    expect((await page.request.delete(`${API_BASE}/v1/companies/${company.id}`, { headers: { Authorization: `Bearer ${token}` } })).ok()).toBeTruthy();
  }
});

test("an acknowledged unpin survives an older bootstrap response", async ({ page }) => {
  const { token, principal } = await signup(page, uniqueName("flag-order"), "Owned-password-123!");
  const company = await createCompany(page, token, principal.id, uniqueName("Flag owner"));
  const group = await createGroup(page, token, principal.id, uniqueName("Pinned owned"), [], company.id);
  const headers = { Authorization: `Bearer ${token}` };
  expect((await page.request.put(`${API_BASE}/v1/conversations/${group.id}/pin`, { headers })).ok()).toBeTruthy();
  // Hold the server echo, not the mutation: a later sync repair must not hide
  // a stale bootstrap overwriting the already acknowledged user action.
  await page.addInitScript((conversationId) => {
    const descriptor = Object.getOwnPropertyDescriptor(WebSocket.prototype, "onmessage")!;
    const waiting: Array<() => void> = [];
    let released = false;
    Object.assign(window, { releasePinEcho: () => { released = true; waiting.splice(0).forEach((resume) => resume()); } });
    Object.defineProperty(WebSocket.prototype, "onmessage", {
      get() { return descriptor.get!.call(this); },
      set(handler: ((this: WebSocket, event: MessageEvent) => void) | null) {
        const socket = this as WebSocket;
        descriptor.set!.call(socket, handler && function (event: MessageEvent) {
          if (typeof event.data !== "string" || !event.data.startsWith("{")) return handler.call(socket, event);
          const frame = JSON.parse(event.data);
          if (!released && frame.changes?.some((change: { conversation_id: string; event_type: string }) =>
            change.conversation_id === conversationId && change.event_type === "conversation.pin_removed")) {
            waiting.push(() => handler.call(socket, event));
          } else handler.call(socket, event);
        });
      },
    });
  }, group.id);
  await gotoDashboard(page);
  await page.locator(".company-selector-btn").click();
  await page.locator(".company-dropdown-item-name").filter({ hasText: company.name }).click();
  const unpin = page.getByRole("button", { name: `Unpin chat: ${group.name}` });
  await expect(unpin).toBeVisible();
  let release!: () => void;
  let held!: () => void;
  const captured = new Promise<void>((resolve) => { held = resolve; });
  const barrier = new Promise<void>((resolve) => { release = resolve; });
  let intercepted = false;
  await page.route("**/api/v1/bootstrap?limit=100", async (route) => {
    if (intercepted) return route.continue();
    intercepted = true;
    const response = await route.fetch();
    held();
    await barrier;
    await route.fulfill({ response });
  });
  try {
    const added = await createGroup(page, token, principal.id, uniqueName("Structural refresh"), [], company.id);
    await captured;
    const mutation = page.waitForResponse((response) => response.url().includes(`/conversations/${group.id}/pin`) && response.request().method() === "DELETE");
    await unpin.click();
    expect((await mutation).ok()).toBeTruthy();
    await expect(page.getByRole("button", { name: `Pin chat: ${group.name}`, exact: true })).toBeEnabled();
    const response = page.waitForResponse((response) => response.url().includes("/api/v1/bootstrap?limit=100"));
    release();
    await response;
    await expect(page.locator(`[data-conversation-id="${added.id}"]`)).toBeVisible();
    await expect(page.getByRole("button", { name: `Pin chat: ${group.name}`, exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: `Unpin chat: ${group.name}`, exact: true })).toHaveCount(0);
    await page.evaluate(() => (window as typeof window & { releasePinEcho: () => void }).releasePinEcho());
    await page.reload();
    await expect(page.locator(".chat-sidebar")).toBeVisible();
    await expandSidebarConversationSections(page);
    await expect(page.getByRole("button", { name: `Pin chat: ${group.name}`, exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: `Unpin chat: ${group.name}`, exact: true })).toHaveCount(0);
  } finally {
    release();
    await page.unroute("**/api/v1/bootstrap?limit=100");
    expect((await page.request.delete(`${API_BASE}/v1/companies/${company.id}`, { headers })).ok()).toBeTruthy();
  }
});

test("sent messages persist in the browser database after closing the dashboard", async ({ page }) => {
  const { token, principal } = await login(page);
  const group = await createGroup(page, token, principal.id, uniqueName("cache-owned"));
  await gotoDashboard(page);
  await page.locator(".conv-item").filter({ hasText: group.name }).click();
  const content = uniqueName("cached-message");
  await page.locator(".chat-input-row textarea").fill(content);
  await page.locator(".chat-input-row textarea").press("Enter");

  const readStored = () => page.evaluate(async (conversationId) => {
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      const request = indexedDB.open("choruz_messages");
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
    try {
      if (!db.objectStoreNames.contains("messages")) return [];
      return await new Promise<Array<{ content: string; server_seq: number }>>((resolve, reject) => {
        const transaction = db.transaction("messages", "readonly");
        const request = transaction.objectStore("messages").index("conversation_id").getAll(conversationId);
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
      });
    } finally { db.close(); }
  }, group.id);

  await expect.poll(async () => (await readStored()).map((message) => message.content)).toContain(content);
  const before = await readStored();
  expect(before.filter((message) => message.content === content)).toHaveLength(1);
  // A non-dashboard document cannot repopulate the cache from the message API.
  await page.goto(`${WEB_BASE}/docs`);
  expect(await readStored()).toEqual(before);
});
