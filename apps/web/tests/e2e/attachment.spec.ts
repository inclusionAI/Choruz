import { Buffer } from "node:buffer";
import { expect, test } from "@playwright/test";
import { login, gotoDashboard, API_BASE } from "../fixtures/auth";
import {
  createAndOpenGroup,
  createGroup,
  sendMessage,
  uniqueName,
  createCompany,
  deleteCompany,
  provisionAgent,
} from "../fixtures/api";

test("attachment drafts survive terminal DM and group transitions", async ({ page }) => {
  const { token, principal } = await login(page);
  const company = await createCompany(page, token, principal.id, uniqueName("draft-company"));
  try {
    const agent = await provisionAgent(page, token, uniqueName("draft-agent"), { workspaceId: company.id });
    const first = await createGroup(page, token, principal.id, uniqueName("draft-first"), [], company.id);
    const second = await createGroup(page, token, principal.id, uniqueName("draft-second"), [], company.id);
    await page.routeWebSocket((url) => url.pathname.startsWith("/v1/ws/terminals/"), (socket) => socket.close());
    await gotoDashboard(page);
    await page.locator(".company-selector-btn").click();
    await page.locator(".company-dropdown-item-name").filter({ hasText: company.name }).click();
    const open = async (id: string) => page.locator(`[data-conversation-id="${id}"]`).click();
    await open(first.id);
    await page.locator(".chat-input-bar textarea").fill("First draft");
    await page.locator(".chat-input-bar input[type=file]").setInputFiles([
      { name: "keep.txt", mimeType: "text/plain", buffer: Buffer.from("Owned retained bytes") },
      { name: "remove.txt", mimeType: "text/plain", buffer: Buffer.from("Do not send") },
    ]);
    await open(agent.conversationId!);
    await expect(page.locator(".chat-input-bar")).toHaveCount(0);
    await open(second.id);
    await expect(page.locator(".attachment-queue-item")).toHaveCount(0);
    await page.locator(".chat-input-bar textarea").fill("Second draft");
    await page.locator(".chat-input-bar input[type=file]").setInputFiles({ name: "second.txt", mimeType: "text/plain", buffer: Buffer.from("Second only") });
    await open(first.id);
    await expect(page.locator(".chat-input-bar textarea")).toHaveValue("First draft");
    await expect(page.locator(".attachment-queue-name")).toHaveText(["keep.txt", "remove.txt"]);
    const messages = async (id: string) => {
      const response = await page.request.get(`${API_BASE}/v1/conversations/${id}/messages?principal_id=${principal.id}`, { headers: { Authorization: `Bearer ${token}` } });
      expect(response.ok()).toBeTruthy();
      return await response.json() as Array<{ content: string; metadata: { attachment_id?: string } }>;
    };
    expect(await messages(first.id)).toHaveLength(0);
    await page.getByRole("button", { name: "Remove remove.txt", exact: true }).click();
    await page.locator(".send-btn").click();
    await expect(page.locator(".chat-input-bar textarea")).toHaveValue("");
    await expect.poll(async () => (await messages(first.id)).length).toBe(2);
    const sent = await messages(first.id);
    expect(sent.map((message) => message.content)).toEqual(["Attachment: keep.txt", "First draft"]);
    const download = await page.request.get(`${API_BASE}/v1/attachments/${sent[0].metadata.attachment_id}?actor_id=${principal.id}`, { headers: { Authorization: `Bearer ${token}` } });
    expect(download.ok()).toBeTruthy();
    expect(await download.text()).toBe("Owned retained bytes");
    await open(second.id);
    await expect(page.locator(".chat-input-bar textarea")).toHaveValue("Second draft");
    await expect(page.locator(".attachment-queue-name")).toHaveText("second.txt");
    expect(await messages(second.id)).toHaveLength(0);
  } finally {
    await deleteCompany(page, token, company.id);
  }
});

test.describe("Attachments", () => {
  test.beforeEach(async ({ page }) => {
    await login(page);
    await gotoDashboard(page);
  });

  /* ---------------------------------------------------------------------- */
  /*  Attachment API                                                         */
  /* ---------------------------------------------------------------------- */

  test("should have an attachment API endpoint", async ({ page }) => {
    const { token } = await login(page);
    // The attachment endpoint exists at /api/attachments/[id]
    // Verify it returns a proper response for a non-existent ID
    const res = await page.request.fetch(
      `${API_BASE}/v1/attachments/nonexistent`,
      {
        method: "GET",
        headers: { Authorization: `Bearer ${token}` },
      },
    );
    // Should return 404 or similar, not 500
    expect(res.status()).toBeLessThan(500);
  });

  /* ---------------------------------------------------------------------- */
  /*  Upload UI                                                              */
  /* ---------------------------------------------------------------------- */

  test("should show attachment/upload button or area in chat input", async ({
    page,
  }) => {
    const { token, principal } = await login(page);
    await createAndOpenGroup(page, token, principal.id, "attach-button");
    const uploadBtn = page.locator(".attach-btn");
    await expect(uploadBtn).toBeVisible({ timeout: 5000 });
    await expect(uploadBtn).toBeEnabled();
  });

  test("should have a hidden file input for uploads", async ({ page }) => {
    const { token, principal } = await login(page);
    await createAndOpenGroup(page, token, principal.id, "attach-input");
    const fileInput = page.locator('input[type="file"]');
    expect(await fileInput.count()).toBeGreaterThan(0);
    await expect(fileInput.first()).toHaveAttribute(
      "aria-label",
      "Upload attachment",
    );
    await expect(fileInput.first()).toBeHidden();
  });

  test("queues multiple files until the user sends them", async ({ page }) => {
    const { token, principal } = await login(page);
    await createAndOpenGroup(page, token, principal.id, "attach-queue");
    const fileInput = page.locator(".chat-input-bar input[type='file']");
    await fileInput.setInputFiles([
      { name: "first.txt", mimeType: "text/plain", buffer: Buffer.from("one") },
      { name: "second.py", mimeType: "text/x-python-script", buffer: Buffer.from("print('two')") },
    ]);

    await expect(page.locator(".attachment-queue-item")).toHaveCount(2);
    await expect(page.getByText("first.txt")).toBeVisible();
    await expect(page.getByText("second.py")).toBeVisible();
    await expect(page.locator(".msg-attachment-file")).toHaveCount(0);
    await expect(page.locator(".send-btn")).toBeEnabled();

    await page.getByRole("button", { name: "Remove first.txt" }).click();
    await expect(page.locator(".attachment-queue-item")).toHaveCount(1);
    await expect(page.getByText("second.py")).toBeVisible();
  });

  test("retry sends only attachments that failed and retains the text", async ({ page }) => {
    const { token, principal } = await login(page);
    const group = await createAndOpenGroup(page, token, principal.id, "attach-retry");
    let failSecond = true;
    await page.route("**/api/v1/attachments", async (route) => {
      if (failSecond && route.request().postDataJSON()?.filename === "second.txt") {
        failSecond = false;
        await route.fulfill({ status: 503, contentType: "application/json", body: JSON.stringify({ error: "Upload temporarily unavailable" }) });
      } else await route.continue();
    });
    await page.locator(".chat-input-bar input[type='file']").setInputFiles([
      { name: "first.txt", mimeType: "text/plain", buffer: Buffer.from("one") },
      { name: "second.txt", mimeType: "text/plain", buffer: Buffer.from("two") },
    ]);
    await page.locator(".chat-input-bar textarea").fill("Retained caption");
    await page.locator(".send-btn").click();
    await expect(page.locator(".chat-input-bar textarea")).toHaveValue("Retained caption");
    await expect(page.locator(".attachment-queue-name")).toHaveText(["second.txt"]);
    let failCaption = true;
    await page.route("**/api/v1/messages", async (route) => {
      if (failCaption && route.request().postDataJSON()?.content === "Retained caption") {
        failCaption = false;
        await route.fulfill({ status: 503, contentType: "application/json", body: JSON.stringify({ error: "Message temporarily unavailable" }) });
      } else await route.continue();
    });
    const captionFailed = page.waitForResponse((response) => response.url().endsWith("/api/v1/messages") && response.request().postDataJSON()?.content === "Retained caption");
    await page.locator(".send-btn").click();
    expect((await captionFailed).status()).toBe(503);
    await expect(page.locator(".chat-input-bar textarea")).toHaveValue("Retained caption");
    await expect(page.locator(".attachment-queue-item")).toHaveCount(0);
    const captionSent = page.waitForResponse((response) => response.url().endsWith("/api/v1/messages") && response.request().postDataJSON()?.content === "Retained caption");
    await page.locator(".send-btn").click();
    expect((await captionSent).ok()).toBe(true);
    await expect(page.locator(".attachment-queue-item")).toHaveCount(0);
    await expect(page.locator(".chat-input-bar textarea")).toHaveValue("");
    const result = await page.request.get(`${API_BASE}/v1/conversations/${group.id}/messages?principal_id=${principal.id}`, { headers: { Authorization: `Bearer ${token}` } });
    expect(result.ok()).toBe(true);
    const payload = await result.json();
    const messages = Array.isArray(payload) ? payload : payload.messages;
    expect(messages.filter((message: { content: string }) => message.content === "Attachment: first.txt")).toHaveLength(1);
    expect(messages.filter((message: { content: string }) => message.content === "Attachment: second.txt")).toHaveLength(1);
    expect(messages.filter((message: { content: string }) => message.content === "Retained caption")).toHaveLength(1);
  });

  test("attachment-only replies retain the quote and clear the composer reply", async ({ page }) => {
    const { token, principal } = await login(page);
    const group = await createAndOpenGroup(page, token, principal.id, "attach-reply");
    const root = await sendMessage(page, token, principal.id, group.id, "Attachment reply target");
    const message = page.locator(".msg-group").filter({ hasText: root.content });
    await message.hover();
    await message.locator('[aria-label="Message actions"]').click();
    await page.getByRole("menuitem", { name: "Reply", exact: true }).click();
    await expect(page.locator(".reply-preview")).toBeVisible();
    await page.locator(".chat-input-bar input[type='file']").setInputFiles({ name: "reply.txt", mimeType: "text/plain", buffer: Buffer.from("reply") });
    const attachmentSent = page.waitForResponse((response) => response.url().endsWith("/api/v1/messages") && response.request().postDataJSON()?.content === "Attachment: reply.txt");
    await page.locator(".send-btn").click();
    expect((await attachmentSent).ok()).toBe(true);
    const result = await page.request.get(`${API_BASE}/v1/conversations/${group.id}/messages?principal_id=${principal.id}`, { headers: { Authorization: `Bearer ${token}` } });
    expect(result.ok()).toBe(true);
    const messages = await result.json();
    expect(messages.find((item: { content: string }) => item.content === "Attachment: reply.txt").metadata.reply_to_id).toBe(root.id);
    await expect(page.locator(".reply-preview")).toHaveCount(0);
    await page.locator(".chat-input-bar textarea").fill("Unrelated next message");
    const textSent = page.waitForResponse((response) => response.url().endsWith("/api/v1/messages") && response.request().postDataJSON()?.content === "Unrelated next message");
    await page.locator(".send-btn").click();
    expect((await textSent).ok()).toBe(true);
    const latest = await page.request.get(`${API_BASE}/v1/conversations/${group.id}/messages?principal_id=${principal.id}`, { headers: { Authorization: `Bearer ${token}` } });
    expect((await latest.json()).find((item: { content: string }) => item.content === "Unrelated next message").metadata.reply_to_id).toBeUndefined();
  });

  /* ---------------------------------------------------------------------- */
  /*  Inline image rendering                                                 */
  /* ---------------------------------------------------------------------- */

  test("should render inline images in message bubbles", async ({ page }) => {
    const { token, principal } = await login(page);
    await createAndOpenGroup(page, token, principal.id, "attach-inline");

    const tinyPng = Buffer.from(
      "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a7WQAAAAASUVORK5CYII=",
      "base64",
    );
    await page.locator(".chat-input-bar input[type='file']").setInputFiles({
      name: "inline.png",
      mimeType: "image/png",
      buffer: tinyPng,
    });

    await expect(page.locator(".attachment-queue-name")).toHaveText("inline.png");
    await expect(page.locator(".msg-attachment-image img[alt='inline.png']")).toHaveCount(0);
    await page.locator(".send-btn").click();

    const image = page.locator(".msg-attachment-image img[alt='inline.png']");
    await expect(image).toBeVisible({ timeout: 15_000 });
    await expect
      .poll(async () => {
        return image.evaluate((img) => {
          const htmlImage = img as HTMLImageElement;
          return htmlImage.complete && htmlImage.naturalWidth > 0;
        });
      })
      .toBe(true);
  });

  test("should proxy markdown attachment image URLs in messages", async ({ page }) => {
    const { token, principal } = await login(page);
    const group = await createGroup(page, token, principal.id, uniqueName("attach-md"));
    await sendMessage(
      page,
      token,
      principal.id,
      group.id,
      "![test-image](/v1/attachments/nonexistent)",
    );
    await gotoDashboard(page);
    const item = page.locator(
      `.conv-item-main[data-conversation-id="${group.id}"]`,
    );
    await expect(item).toBeVisible({ timeout: 10_000 });
    await item.click();
    const image = page.locator("img[alt='test-image']");
    await expect(image).toHaveCount(1);
    await expect(image).toHaveAttribute("src", /\/api\/attachments\/nonexistent$/);
  });

  /* ---------------------------------------------------------------------- */
  /*  Content type handling                                                  */
  /* ---------------------------------------------------------------------- */

  test("should handle text/plain content type", async ({ page }) => {
    const { token, principal } = await login(page);
    const group = await createGroup(page, token, principal.id, uniqueName("attach-content"));
    const res = await page.request.post(`${API_BASE}/v1/messages`, {
      headers: { Authorization: `Bearer ${token}` },
      data: {
        actor_id: principal.id,
        conversation_id: group.id,
        content: "plain text message",
        content_type: "text/plain",
        idempotency_key: `content-type-${Date.now()}`,
        metadata: {},
      },
    });
    expect(res.ok()).toBeTruthy();
  });

  test("should handle text content type", async ({ page }) => {
    const { token, principal } = await login(page);
    const group = await createGroup(page, token, principal.id, uniqueName("attach-text"));
    const res = await page.request.post(`${API_BASE}/v1/messages`, {
      headers: { Authorization: `Bearer ${token}` },
      data: {
        actor_id: principal.id,
        conversation_id: group.id,
        content: "text content message",
        content_type: "text",
        idempotency_key: `content-type-text-${Date.now()}`,
        metadata: {},
      },
    });
    expect(res.ok()).toBeTruthy();
  });

  /* ---------------------------------------------------------------------- */
  /*  Drag and drop                                                          */
  /* ---------------------------------------------------------------------- */

  test("should not crash on drag-and-drop over chat area", async ({
    page,
  }) => {
    const { token, principal } = await login(page);
    await createAndOpenGroup(page, token, principal.id, "attach-drag");
    // Simulate dragenter event
    await page.evaluate(() => {
      const chatArea = document.querySelector(
        ".chat-input-bar, .message-list",
      );
      if (chatArea) {
        chatArea.dispatchEvent(
          new DragEvent("dragenter", { bubbles: true }),
        );
        chatArea.dispatchEvent(
          new DragEvent("dragleave", { bubbles: true }),
        );
      }
    });
    // Should not crash
    await expect(page.locator("textarea").first()).toBeEnabled();
  });
});
