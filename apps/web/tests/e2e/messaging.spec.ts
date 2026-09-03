import { Buffer } from "node:buffer";
import { expect, test } from "@playwright/test";
import { login, gotoDashboard, API_BASE } from "../fixtures/auth";
import {
  createGroup,
  createAndOpenGroup,
  provisionAgent,
  sendMessage,
  getMessages,
  uniqueName,
} from "../fixtures/api";

test.describe("Messaging", () => {
  /* ---------------------------------------------------------------------- */
  /*  Helpers                                                                */
  /* ---------------------------------------------------------------------- */

  async function createEmptyGroupForUpload(
    page: import("@playwright/test").Page,
    token: string,
    principalId: string,
    prefix: string,
  ) {
    const companyRes = await page.request.get(`${API_BASE}/v1/companies?principal_id=${principalId}`, {
      headers: { Authorization: `Bearer ${token}` },
    });
    expect(companyRes.ok()).toBeTruthy();
    const companies = (await companyRes.json()) as Array<{ id: string; name: string; slug?: string | null }>;
    const activeCompany = companies.find((c) => c.slug === "default") ?? companies[0];
    expect(activeCompany).toBeTruthy();

    const groupName = uniqueName(prefix);
    const groupRes = await page.request.post(`${API_BASE}/v1/groups`, {
      headers: { Authorization: `Bearer ${token}` },
      data: {
        actor_id: principalId,
        name: groupName,
        description: null,
        avatar_url: null,
        member_ids: [],
        workspace_id: activeCompany.id,
      },
    });
    expect(groupRes.ok()).toBeTruthy();

    await gotoDashboard(page);
    await page.waitForSelector(".conv-item", { timeout: 15_000 });
    await page.locator(".conv-item").filter({ hasText: groupName }).first().click();
    return { groupName };
  }

  /* ---------------------------------------------------------------------- */
  /*  Send message                                                           */
  /* ---------------------------------------------------------------------- */

  test("should send a message and see it in the chat", async ({ page }) => {
    const { token, principal } = await login(page);
    await createAndOpenGroup(page, token, principal.id, "msg-send");

    const testMsg = `e2e-msg-${Date.now()}`;
    const textarea = page.locator("textarea").first();
    await textarea.fill(testMsg);
    await textarea.press("Enter");

    // Wait for the message to appear in the message list
    await expect(page.locator(".messages-area").getByText(testMsg)).toBeVisible({
      timeout: 10_000,
    });
  });

  test("restores the draft and removes the optimistic bubble when sending fails", async ({ page }) => {
    const { token, principal } = await login(page);
    const group = await createAndOpenGroup(page, token, principal.id, "msg-failure");
    const replyTarget = `reply-target-${Date.now()}`;
    await sendMessage(page, token, principal.id, group.id, replyTarget);
    const targetMessage = page.locator(".msg-group").filter({ hasText: replyTarget }).first();
    await expect(targetMessage).toBeVisible({ timeout: 10_000 });
    await targetMessage.hover();
    await targetMessage.locator('[aria-label="Message actions"]').click();
    await page.getByRole("menuitem", { name: "Reply", exact: true }).click();
    await page.route("**/api/v1/messages", (route) =>
      route.fulfill({ status: 500, contentType: "application/json", body: JSON.stringify({ error: "temporary failure" }) }),
    );

    const content = `must-survive-${Date.now()}`;
    const textarea = page.locator("textarea").first();
    await textarea.fill(content);
    await textarea.press("Enter");

    await expect(textarea).toHaveValue(content);
    await expect(page.getByRole("alert").filter({ hasText: "Message not sent" })).toBeVisible();
    await expect(page.locator(".messages-area").getByText(content)).toHaveCount(0);
    await expect(page.locator(".reply-preview-text").filter({ hasText: replyTarget })).toBeVisible();
    await expect(textarea).toBeFocused();
  });

  test("keeps a separate draft while switching between conversations", async ({ page }) => {
    const { token, principal } = await login(page);
    const first = await createGroup(page, token, principal.id, uniqueName("draft-first"));
    const second = await createGroup(page, token, principal.id, uniqueName("draft-second"));
    await gotoDashboard(page);
    await page.locator(".conv-item").filter({ hasText: first.name }).first().click();
    const textarea = page.locator("textarea").first();
    await textarea.fill("first conversation draft");
    await page.locator(".conv-item").filter({ hasText: second.name }).first().click();
    await expect(textarea).toHaveValue("");
    await textarea.fill("second conversation draft");
    await page.locator(".conv-item").filter({ hasText: first.name }).first().click();
    await expect(textarea).toHaveValue("first conversation draft");
    await page.locator(".conv-item").filter({ hasText: second.name }).first().click();
    await expect(textarea).toHaveValue("second conversation draft");
  });

  test("should send message via API and retrieve it", async ({ page }) => {
    const { token, principal } = await login(page);
    const group = await createGroup(page, token, principal.id, uniqueName("msg-api"));

    const content = `api-msg-${Date.now()}`;
    await sendMessage(page, token, principal.id, group.id, content);
    const msgs = await getMessages(page, token, principal.id, group.id);
    const found = msgs.some((m) => m.content === content);
    expect(found).toBeTruthy();
  });

  test("loads older history when the message list reaches the top", async ({ page }) => {
    const { token, principal } = await login(page);
    const group = await createGroup(page, token, principal.id, uniqueName("msg-history"));
    const prefix = `history-${Date.now()}`;
    for (let index = 1; index <= 55; index += 1) {
      await sendMessage(page, token, principal.id, group.id, `${prefix}-${index}`);
    }

    await gotoDashboard(page);
    const initialPage = page.waitForResponse((response) =>
      response.url().includes(`/v1/conversations/${group.id}/message-page`) &&
      !response.url().includes("before_seq="),
    );
    await page.locator(".conv-item").filter({ hasText: group.name }).first().click();
    expect((await initialPage).ok()).toBeTruthy();
    const messageArea = page.locator(".messages-area");
    await expect(messageArea.getByText(`${prefix}-55`, { exact: true })).toBeVisible();
    await expect(messageArea.getByText(`${prefix}-1`, { exact: true })).toHaveCount(0);

    const olderPage = page.waitForResponse((response) =>
      response.url().includes(`/v1/conversations/${group.id}/message-page`) &&
      response.url().includes("before_seq="),
    );
    await messageArea.evaluate((element) => {
      element.scrollTop = 0;
      element.dispatchEvent(new Event("scroll", { bubbles: true }));
    });
    expect((await olderPage).ok()).toBeTruthy();

    await messageArea.evaluate((element) => {
      element.scrollTop = 0;
      element.dispatchEvent(new Event("scroll", { bubbles: true }));
    });
    await expect(messageArea.getByText(`${prefix}-1`, { exact: true })).toBeVisible();
  });

  /* ---------------------------------------------------------------------- */
  /*  Message dedup                                                          */
  /* ---------------------------------------------------------------------- */

  test("should show exactly one bubble per message (no duplicates)", async ({
    page,
  }) => {
    const { token, principal } = await login(page);
    await createAndOpenGroup(page, token, principal.id, "msg-dedup");

    const testMsg = `dedup-${Date.now()}`;
    const textarea = page.locator("textarea").first();
    await textarea.fill(testMsg);
    await textarea.press("Enter");

    await page.waitForTimeout(4000);

    const bubbleCount = await page
      .locator(`.msg-group:has-text("${testMsg}")`)
      .count();
    expect(bubbleCount).toBe(1);
  });

  test("should return one persisted message for an idempotent API retry", async ({ page }) => {
    const { token, principal } = await login(page);
    const group = await createGroup(page, token, principal.id, uniqueName("msg-api-dedup"));
    const content = `api-dedup-${Date.now()}`;
    const payload = {
      actor_id: principal.id,
      conversation_id: group.id,
      content,
      content_type: "text/plain",
      idempotency_key: uniqueName("api-dedup-key"),
      metadata: {},
    };

    const first = await page.request.post(`${API_BASE}/v1/messages`, {
      headers: { Authorization: `Bearer ${token}` },
      data: payload,
    });
    const retry = await page.request.post(`${API_BASE}/v1/messages`, {
      headers: { Authorization: `Bearer ${token}` },
      data: payload,
    });
    expect(first.ok()).toBeTruthy();
    expect(retry.ok()).toBeTruthy();
    expect((await retry.json()).id).toBe((await first.json()).id);

    const messages = await getMessages(page, token, principal.id, group.id, 100);
    expect(messages.filter((message) => message.content === content)).toHaveLength(1);
  });

  /* ---------------------------------------------------------------------- */
  /*  @mention                                                               */
  /* ---------------------------------------------------------------------- */

  test("should show mention dropdown when typing @", async ({ page }) => {
    const { token, principal } = await login(page);
    const agent = await provisionAgent(page, token, uniqueName("mention-agent"));
    const group = await createGroup(page, token, principal.id, uniqueName("msg-mention"), [
      agent.agentId,
    ]);
    await gotoDashboard(page);
    await page.locator(".conv-item").filter({ hasText: group.name }).first().click();

    const textarea = page.locator("textarea").first();
    await expect(textarea).toBeVisible({ timeout: 10_000 });
    await textarea.fill("@");
    const dropdown = page.locator(".mention-dropdown");
    await expect(dropdown).toBeVisible({ timeout: 5000 });
    await expect(dropdown.locator(".mention-item").filter({ hasText: agent.agentName })).toBeVisible();
    await expect(dropdown.locator(".mention-item").filter({ hasText: agent.agentName })).toContainText(
      agent.agentName,
    );
  });

  test("should insert mention when selecting from dropdown", async ({
    page,
  }) => {
    const { token, principal } = await login(page);
    const agent = await provisionAgent(page, token, uniqueName("mention-select-agent"));
    const group = await createGroup(
      page,
      token,
      principal.id,
      uniqueName("msg-mention-select"),
      [agent.agentId],
    );
    await gotoDashboard(page);
    await page.locator(".conv-item").filter({ hasText: group.name }).first().click();

    const textarea = page.locator("textarea").first();
    await expect(textarea).toBeVisible({ timeout: 10_000 });
    await textarea.fill("@");
    const dropdown = page.locator(".mention-dropdown");
    await expect(dropdown).toBeVisible({ timeout: 5000 });
    await dropdown.locator(".mention-item").filter({ hasText: agent.agentName }).click();
    await expect(textarea).toHaveValue(`@${agent.agentName} `);
  });

  /* ---------------------------------------------------------------------- */
  /*  Markdown rendering                                                     */
  /* ---------------------------------------------------------------------- */

  test("should render markdown in message bubbles", async ({ page }) => {
    const { token, principal } = await login(page);
    const group = await createGroup(page, token, principal.id, uniqueName("msg-md"));

    // Send a markdown message via API
    const mdContent = "**bold text** and `inline code`";
    await sendMessage(page, token, principal.id, group.id, mdContent);

    await gotoDashboard(page);
    await page.waitForSelector(".conv-item", { timeout: 10_000 });
    // Click the group conversation
    const items = page.locator(".conv-item");
    const count = await items.count();
    for (let i = 0; i < count; i++) {
      const txt = await items.nth(i).textContent();
      if (txt?.includes(group.name ?? "")) {
        await items.nth(i).click();
        break;
      }
    }

    await expect(page.locator(".messages-area strong")).toContainText("bold text", {
      timeout: 10_000,
    });
    await expect(page.locator(".messages-area")).not.toContainText("**bold text**");
  });

  test("should render untrusted HTML as inert text", async ({ page }) => {
    await page.addInitScript(() => {
      (window as typeof window & { __choruzXssTriggered?: boolean }).__choruzXssTriggered = false;
    });
    const { token, principal } = await login(page);
    const group = await createGroup(page, token, principal.id, uniqueName("msg-xss"));
    const payload = '<img src="x" onerror="window.__choruzXssTriggered=true">';
    const message = await sendMessage(page, token, principal.id, group.id, payload);

    await gotoDashboard(page);
    await page.locator(".conv-item").filter({ hasText: group.name }).first().click();
    const messageElement = page.locator(`[data-msg-id="${message.id}"]`);
    await expect(messageElement).toBeVisible({ timeout: 10_000 });
    await expect(messageElement).toContainText(payload);
    await expect(messageElement.locator("img")).toHaveCount(0);
    expect(
      await page.evaluate(
        () => (window as typeof window & { __choruzXssTriggered?: boolean }).__choruzXssTriggered,
      ),
    ).toBe(false);
  });

  /* ---------------------------------------------------------------------- */
  /*  Reply / quote                                                          */
  /* ---------------------------------------------------------------------- */

  test("should show reply preview when replying to a message", async ({
    page,
  }) => {
    const { token, principal } = await login(page);
    const group = await createAndOpenGroup(page, token, principal.id, "msg-reply");
    const content = `reply-target-${Date.now()}`;
    await sendMessage(page, token, principal.id, group.id, content);

    const message = page.locator(".msg-group").filter({ hasText: content }).first();
    await expect(message).toBeVisible({ timeout: 10_000 });
    await message.hover();
    await message.locator('[aria-label="Message actions"]').click();
    // exact: true — the menu also contains "Reply in thread", which a
    // substring match would ambiguously hit (strict-mode violation).
    await page.getByRole("menuitem", { name: "Reply", exact: true }).click();
    await expect(page.locator(".reply-preview")).toBeVisible();
    await expect(page.locator(".reply-preview-text")).toContainText(content);
  });

  /* ---------------------------------------------------------------------- */
  /*  Send button state                                                      */
  /* ---------------------------------------------------------------------- */

  test("should disable send button when input is empty", async ({ page }) => {
    const { token, principal } = await login(page);
    await createAndOpenGroup(page, token, principal.id, "msg-empty");

    const sendBtn = page.locator(".send-btn");
    await expect(sendBtn).toBeDisabled();
  });

  test("should enable send button when input has text", async ({ page }) => {
    const { token, principal } = await login(page);
    await createAndOpenGroup(page, token, principal.id, "msg-enabled");

    const textarea = page.locator("textarea").first();
    await textarea.fill("hello");
    const sendBtn = page.locator(".send-btn");
    await expect(sendBtn).toBeEnabled();
  });

  test("should upload and render an image attachment from group chat input", async ({ page }) => {
    const { token, principal } = await login(page);
    await createEmptyGroupForUpload(page, token, principal.id, "attach-ui");

    await expect(page.locator(".chat-input-bar .attach-btn")).toBeVisible({ timeout: 10_000 });
    await expect(page.locator(".chat-input-bar input[type='file']")).toHaveCount(1);

    const tinyPng = Buffer.from(
      "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a7WQAAAAASUVORK5CYII=",
      "base64",
    );
    await page
      .locator(".chat-input-bar input[type='file']")
      .setInputFiles({
        name: "tiny.png",
        mimeType: "image/png",
        buffer: tinyPng,
      });

    await expect(page.locator(".attachment-queue-name")).toHaveText("tiny.png");
    await page.locator(".send-btn").click();

    await expect(page.locator(".msg-attachment-image img[alt='tiny.png']")).toBeVisible({
      timeout: 15_000,
    });
    await expect
      .poll(async () => {
        return page.locator(".msg-attachment-image img[alt='tiny.png']").evaluate((img) => {
          const image = img as HTMLImageElement;
          return image.complete && image.naturalWidth > 0;
        });
      })
      .toBe(true);
  });

  test("should reject oversized attachments in the group chat input", async ({ page }) => {
    const { token, principal } = await login(page);
    await createEmptyGroupForUpload(page, token, principal.id, "attach-too-large");

    await page.locator(".chat-input-bar input[type='file']").setInputFiles({
      name: "too-large.txt",
      mimeType: "text/plain",
      buffer: Buffer.alloc(5 * 1024 * 1024 + 1, 0x61),
    });

    await expect(page.locator(".attachment-queue-name")).toHaveText("too-large.txt");
    await page.locator(".send-btn").click();

    await expect(page.locator(".reply-preview-label")).toHaveText("Message not sent");
    await expect(page.locator(".reply-preview-text")).toContainText("5 MB");
  });

  test("should roll back uploaded bytes when creating the attachment message fails", async ({ page }) => {
    const { token, principal } = await login(page);
    await createEmptyGroupForUpload(page, token, principal.id, "attach-rollback");

    let uploadedAttachmentId: string | null = null;
    await page.route("**/api/v1/attachments", async (route) => {
      const response = await route.fetch();
      const payload = (await response.json()) as { id?: string };
      uploadedAttachmentId = payload.id ?? null;
      await route.fulfill({ response });
    });
    await page.route("**/api/v1/messages", async (route) => {
      await route.fulfill({
        status: 500,
        contentType: "application/json",
        body: JSON.stringify({ error: "message post failed for test" }),
      });
    });

    await page.locator(".chat-input-bar input[type='file']").setInputFiles({
      name: "rollback.png",
      mimeType: "image/png",
      buffer: Buffer.from(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a7WQAAAAASUVORK5CYII=",
        "base64",
      ),
    });

    await page.locator(".send-btn").click();

    await expect(page.locator(".reply-preview-label")).toHaveText("Message not sent");
    await expect(page.locator(".reply-preview-text")).toContainText("message post failed for test");
    await expect(page.getByText("Attachment: rollback.png")).toHaveCount(0);
    await expect.poll(() => uploadedAttachmentId).not.toBeNull();

    const attachmentGet = await page.request.get(
      `${API_BASE}/v1/attachments/${uploadedAttachmentId}?actor_id=${principal.id}`,
      { headers: { Authorization: `Bearer ${token}` } },
    );
    expect(attachmentGet.status()).toBe(404);
  });

  test("should render non-image attachments as downloadable files", async ({ page }) => {
    const { token, principal } = await login(page);
    await createEmptyGroupForUpload(page, token, principal.id, "attach-file");

    await page.locator(".chat-input-bar input[type='file']").setInputFiles({
      name: "notes.txt",
      mimeType: "text/plain",
      buffer: Buffer.from("hello from attachment\n", "utf8"),
    });

    await page.locator(".send-btn").click();

    const fileCard = page.locator(".msg-attachment-file .msg-attachment-link").filter({ hasText: "notes.txt" }).first();
    await expect(fileCard).toBeVisible({ timeout: 15_000 });
    await expect(fileCard).toHaveAttribute("href", /\/api\/attachments\//);
  });

  /* ---------------------------------------------------------------------- */
  /*  Optimistic update                                                      */
  /* ---------------------------------------------------------------------- */

  test("should show message immediately (optimistic update)", async ({
    page,
  }) => {
    const { token, principal } = await login(page);
    await createAndOpenGroup(page, token, principal.id, "msg-optimistic");

    const testMsg = `optimistic-${Date.now()}`;
    const textarea = page.locator("textarea").first();
    await textarea.fill(testMsg);
    await textarea.press("Enter");
    // Should appear near-instantly (optimistic)
    await expect(page.locator(".messages-area").getByText(testMsg)).toBeVisible({
      timeout: 2000,
    });
  });

  /* ---------------------------------------------------------------------- */
  /*  Message via API (no CHORUZ_REPLY tags)                                  */
  /* ---------------------------------------------------------------------- */

  test("should not contain CHORUZ_REPLY tags in messages", async ({ page }) => {
    const { token, principal } = await login(page);
    const group = await createGroup(page, token, principal.id, uniqueName("msg-reply-tags"));
    const msgs = await getMessages(page, token, principal.id, group.id);
    for (const m of msgs) {
      expect(m.content).not.toContain("{{CHORUZ_REPLY}}");
      expect(m.content).not.toContain("{{/CHORUZ_REPLY}}");
    }
  });

  /* ---------------------------------------------------------------------- */
  /*  Textarea auto-resize                                                   */
  /* ---------------------------------------------------------------------- */

  test("should auto-resize textarea on multiline input", async ({ page }) => {
    const { token, principal } = await login(page);
    await createAndOpenGroup(page, token, principal.id, "msg-resize");

    const textarea = page.locator("textarea").first();
    const heightBefore = await textarea.evaluate((el) => el.offsetHeight);
    await textarea.fill("line1\nline2\nline3\nline4");
    await textarea.dispatchEvent("input");
    await page.waitForTimeout(200);
    const heightAfter = await textarea.evaluate((el) => el.offsetHeight);
    expect(heightAfter).toBeGreaterThanOrEqual(heightBefore);
  });
});
