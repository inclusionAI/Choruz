import { expect, test } from "@playwright/test";
import { login, gotoDashboard, WEB_BASE } from "../fixtures/auth";
import { createGroup, uniqueName } from "../fixtures/api";

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
