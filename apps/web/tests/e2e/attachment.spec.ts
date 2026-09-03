import { Buffer } from "node:buffer";
import { expect, test } from "@playwright/test";
import { login, gotoDashboard, API_BASE } from "../fixtures/auth";
import {
  createAndOpenGroup,
  createGroup,
  sendMessage,
  uniqueName,
} from "../fixtures/api";

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
