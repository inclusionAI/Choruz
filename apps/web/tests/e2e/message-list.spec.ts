import { expect, test } from "@playwright/test";
import { login, WEB_BASE } from "../fixtures/auth";
import { addGroupMember, createAgent, createGroup, getMessages, sendMessage, uniqueName } from "../fixtures/api";

test("messages render content, timestamp and an empty conversation", async ({ page }) => {
  const { token, principal } = await login(page);
  const group = await createGroup(page, token, principal.id, uniqueName("message-render"));
  await page.goto(`${WEB_BASE}/dashboard?conversationId=${group.id}`);
  await expect(page.locator(".messages-area .empty-state")).toContainText("No messages yet");
  await sendMessage(page, token, principal.id, group.id, "```js\nconsole.log('owned');\n```");
  const bubble = page.locator(".msg-group").filter({ hasText: "console.log('owned')" });
  await expect(bubble.locator("pre code")).toHaveText("console.log('owned');\n");
  await expect(bubble.locator(".msg-time")).toHaveText(/\d{2}:\d{2}/);
});

test("reading history stays anchored until a local send, which reveals the latest message", async ({ page }) => {
  const { token, principal } = await login(page);
  const group = await createGroup(page, token, principal.id, uniqueName("message-scroll"));
  await page.setViewportSize({ width: 900, height: 500 });
  for (let i = 0; i < 30; i++) {
    await sendMessage(page, token, principal.id, group.id, `Owned message ${i}\n${"detail\n".repeat(5)}`);
  }
  await page.goto(`${WEB_BASE}/dashboard?conversationId=${group.id}`);
  const area = page.locator(".messages-area");
  await expect.poll(() => area.evaluate(el => el.scrollHeight - el.clientHeight)).toBeGreaterThan(500);
  const distance = () => area.evaluate(el => el.scrollHeight - el.scrollTop - el.clientHeight);
  await expect.poll(distance).toBeLessThan(100);
  await area.hover();
  await page.mouse.wheel(0, -10000);
  const oldest = area.locator(".msg-markdown").filter({ hasText: /^Owned message 0\b/ });
  await expect(oldest).toBeInViewport({ ratio: 1 });
  await sendMessage(page, token, principal.id, group.id, "Message from another client");
  await expect(page.locator(`[data-conversation-id="${group.id}"]`)).toContainText("Message from another client");
  await expect(oldest).toBeInViewport({ ratio: 1 });
  await expect(area.getByText("Message from another client", { exact: true })).not.toBeInViewport();
  const input = page.locator(".chat-input-bar textarea");
  await input.fill("Owned newest message");
  const sent = page.waitForResponse(response => response.request().method() === "POST" && response.url().includes("/messages"));
  await input.press("Enter");
  expect((await sent).ok()).toBeTruthy();
  await expect(area.getByText("Owned newest message", { exact: true })).toBeInViewport({ ratio: 1 });
  await area.hover();
  await page.mouse.wheel(0, -10000);
  await expect.poll(distance).toBeGreaterThan(200);
  await expect(area.getByText("Owned newest message", { exact: true })).not.toBeInViewport();
  await sendMessage(page, token, principal.id, group.id, "Later message from another client");
  await expect(page.locator(`[data-conversation-id="${group.id}"]`)).toContainText("Later message from another client");
  await expect.poll(distance).toBeGreaterThan(200);
  await expect(area.getByText("Later message from another client", { exact: true })).not.toBeInViewport();
  await area.hover();
  await page.mouse.wheel(0, 10000);
  await expect(area.getByText("Later message from another client", { exact: true })).toBeInViewport({ ratio: 1 });
  await page.reload();
  await expect(area.getByText("Later message from another client", { exact: true })).toBeInViewport({ ratio: 1 });
});

test("message presentation cleans control sequences without changing stored content", async ({ page }) => {
  const { token, principal } = await login(page);
  const group = await createGroup(page, token, principal.id, uniqueName("message-ansi"));
  const agent = await createAgent(page, token, principal.id, uniqueName("message-author"));
  await addGroupMember(page, token, group.id, principal.id, [agent.principal.id]);
  const raw = "{{CHORUZ_REPLY group=test}}normal \u001b[31mred text\u001b[0m end{{/CHORUZ_REPLY}}";
  const sent = await sendMessage(page, agent.secret, agent.principal.id, group.id, raw);
  await page.goto(`${WEB_BASE}/dashboard?conversationId=${group.id}`);
  const bubble = page.locator(".msg-group").filter({ hasText: "red text" });
  await expect(bubble).toBeVisible();
  expect(await bubble.textContent()).not.toContain("\u001b[");
  await expect(bubble.locator(".msg-markdown")).toHaveText("normal red text end");
  expect((await getMessages(page, token, principal.id, group.id)).find(message => message.id === sent.id)?.content).toBe(raw);
  await page.screenshot({ path: test.info().outputPath("message-ansi.png") });
});
