import { expect, test, type Page } from "@playwright/test";
import { API_BASE, CREDENTIALS, WEB_BASE } from "../fixtures/auth";
import { createCompany, createGroup, deleteCompany } from "../fixtures/api";

/**
 * Message threads E2E.
 *
 * Seeds a fresh group + root message + one QUIET threaded reply via the
 * API, then drives the UI: the timeline must hide the quiet reply but
 * show the "1 reply" rollup chip; clicking it opens the thread panel
 * with the reply; sending from the panel composer appends to the thread.
 */

async function loginAndSetCookie(page: Page) {
  const res = await page.request.post(`${API_BASE}/v1/auth/local/login`, {
    data: { username: CREDENTIALS.username, password: CREDENTIALS.password },
  });
  expect(res.ok()).toBeTruthy();
  const payload = await res.json();
  const token: string = payload.session_token;
  const principalId: string = payload.principal.id;

  await page.context().addCookies([
    {
      name: "choruz_session",
      value: token,
      url: WEB_BASE,
      httpOnly: true,
      sameSite: "Lax",
      expires: Math.floor(Date.now() / 1000) + 60 * 60,
    },
  ]);

  return { token, principalId };
}

async function sendMessage(
  page: Page,
  token: string,
  principalId: string,
  conversationId: string,
  content: string,
  metadata: Record<string, unknown> = {},
) {
  const res = await page.request.post(`${API_BASE}/v1/messages`, {
    headers: { Authorization: `Bearer ${token}` },
    data: {
      actor_id: principalId,
      conversation_id: conversationId,
      idempotency_key: `e2e-thread-${Date.now()}-${Math.random().toString(16).slice(2)}`,
      content,
      content_type: "text",
      metadata,
    },
  });
  expect(res.ok(), `send "${content}": ${res.status()}`).toBeTruthy();
  return (await res.json()) as { id: string };
}

test("thread rollup, side panel, and quiet-reply timeline filtering", async ({ page }) => {
  const { token, principalId } = await loginAndSetCookie(page);

  // Fresh group so prior data can't interfere.
  const groupName = `threads-e2e-${Date.now()}`;
  const groupRes = await page.request.post(`${API_BASE}/v1/groups`, {
    headers: { Authorization: `Bearer ${token}` },
    data: {
      actor_id: principalId,
      name: groupName,
      description: null,
      avatar_url: null,
      member_ids: [],
    },
  });
  expect(groupRes.ok()).toBeTruthy();
  const convId = (await groupRes.json()).id as string;

  const rootText = `thread-root-${Date.now()}`;
  const quietText = `quiet-reply-${Date.now()}`;
  const root = await sendMessage(page, token, principalId, convId, rootText);
  await sendMessage(page, token, principalId, convId, quietText, {
    reply_to_id: root.id,
    thread: true,
  });

  await page.goto(`${WEB_BASE}/dashboard?conversationId=${convId}`);
  await page.waitForSelector(".messages-area", { timeout: 10000 });

  // Root visible on the timeline; quiet reply hidden behind the rollup.
  await expect(page.locator(`.msg-group:has-text("${rootText}")`)).toBeVisible({
    timeout: 10000,
  });
  await expect(page.locator(`.msg-group:has-text("${quietText}")`)).toHaveCount(0);

  // Rollup chip shows "1 reply" and opens the panel.
  const rollup = page.locator(".msg-thread-rollup");
  await expect(rollup).toBeVisible({ timeout: 10000 });
  await expect(rollup).toContainText("1 reply");
  await rollup.click();

  const panel = page.locator(".thread-panel");
  await expect(panel).toBeVisible();
  await expect(panel).toContainText(rootText);
  await expect(panel.locator(`.msg-group:has-text("${quietText}")`)).toBeVisible({
    timeout: 10000,
  });

  // Reply from the composer (quiet — checkbox unchecked by default).
  const panelReply = `panel-reply-${Date.now()}`;
  await panel.locator("textarea").fill(panelReply);
  const composing = await panel.locator("textarea").evaluate((element) =>
    element.dispatchEvent(new KeyboardEvent("keydown", {
      key: "Enter", bubbles: true, cancelable: true, isComposing: true,
    })),
  );
  expect(composing, "IME confirmation must not submit or cancel the input event").toBe(true);
  await expect(panel.locator("textarea")).toHaveValue(panelReply);
  await panel.locator("textarea").press("Enter");
  await expect(panel.locator(`.msg-group:has-text("${panelReply}")`)).toBeVisible({
    timeout: 10000,
  });

  // Still quiet: the main timeline must not show the panel reply.
  await expect(
    page.locator(`.chat-primary .msg-group:has-text("${panelReply}")`),
  ).toHaveCount(0);

  // Rollup count reflects the second reply.
  await expect(page.locator(".msg-thread-rollup")).toContainText("2 replies", {
    timeout: 10000,
  });

  // Close the panel.
  await panel.locator(".thread-panel-close").click();
  await expect(page.locator(".thread-panel")).toHaveCount(0);

  await page.screenshot({ path: "test-results/threads-e2e.png", fullPage: false });
});

test("incoming thread replies preserve reading position and follow the bottom", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 900 });
  const { token, principalId } = await loginAndSetCookie(page);
  const name = `thread-scroll-${crypto.randomUUID()}`;
  const company = await createCompany(page, token, principalId, name);
  try {
    const group = await createGroup(page, token, principalId, name, [], company.id);
    const root = await sendMessage(page, token, principalId, group.id, "Reading root");
    const metadata = { reply_to_id: root.id, thread: true };
    for (let i = 0; i < 24; i++) {
      await sendMessage(page, token, principalId, group.id, `Historical reply ${i}`, metadata);
    }
    await page.goto(`${WEB_BASE}/dashboard`);
    await page.locator(".company-selector-btn").click();
    await page.locator(".company-dropdown-item-name").filter({ hasText: name }).click();
    const groups = page.getByRole("button", { name: /Group Conversations/ });
    if (await groups.getAttribute("aria-expanded") !== "true") await groups.click();
    await page.locator(`[data-conversation-id="${group.id}"]`).click();
    await page.locator(".msg-thread-rollup").click();
    const list = page.locator(".thread-panel-messages");
    const gap = () => list.evaluate((el) => el.scrollHeight - el.clientHeight - el.scrollTop);
    await expect(list).toContainText("Historical reply 23");
    await expect.poll(gap).toBeLessThan(2);
    await list.evaluate((el) => { el.scrollTop = 80; el.dispatchEvent(new Event("scroll")); });
    await expect.poll(() => list.evaluate((el) => el.scrollTop)).toBe(80);
    await sendMessage(page, token, principalId, group.id, "Arrived while reading", metadata);
    await expect(list).toContainText("Arrived while reading");
    await expect.poll(() => list.evaluate((el) => el.scrollTop)).toBe(80);
    await expect(page.getByRole("button", { name: "New replies" })).toBeVisible();
    await page.getByRole("button", { name: "New replies" }).click();
    await expect.poll(gap).toBeLessThan(2);
    await expect(page.getByRole("button", { name: "New replies" })).toHaveCount(0);
    await list.evaluate((el) => { el.scrollTop = el.scrollHeight; el.dispatchEvent(new Event("scroll")); });
    await sendMessage(page, token, principalId, group.id, "Arrived at bottom", metadata);
    await expect(list).toContainText("Arrived at bottom");
    await expect.poll(gap).toBeLessThan(2);
    await expect(page.getByRole("button", { name: "New replies" })).toHaveCount(0);
    await list.evaluate((el) => { el.scrollTop = 80; el.dispatchEvent(new Event("scroll")); });
    await sendMessage(page, token, principalId, group.id, "Another unread reply", metadata);
    await expect(page.getByRole("button", { name: "New replies" })).toBeVisible();
    await list.evaluate((el) => { el.scrollTop = el.scrollHeight; el.dispatchEvent(new Event("scroll")); });
    await expect(page.getByRole("button", { name: "New replies" })).toHaveCount(0);
  } finally {
    await deleteCompany(page, token, company.id);
  }
});

test("thread drafts survive closing and switching roots", async ({ page }) => {
  // Keep both columns visible; narrower layouts intentionally overlay the thread.
  await page.setViewportSize({ width: 1600, height: 900 });
  const { token, principalId } = await loginAndSetCookie(page);
  const name = `thread-drafts-${crypto.randomUUID()}`;
  const company = await createCompany(page, token, principalId, name);
  let releaseSend = () => {};
  try {
    const group = await createGroup(page, token, principalId, name, [], company.id);
    for (const text of ["first root", "second root"]) {
      const root = await sendMessage(page, token, principalId, group.id, text);
      await sendMessage(page, token, principalId, group.id, `Reply to ${text}`, { reply_to_id: root.id, thread: true });
    }
    await page.goto(`${WEB_BASE}/dashboard`);
    await page.locator(".company-selector-btn").click();
    await page.locator(".company-dropdown-item-name").filter({ hasText: name }).click();
    const groups = page.getByRole("button", { name: /Group Conversations/ });
    if (await groups.getAttribute("aria-expanded") !== "true") await groups.click();
    await page.locator(`[data-conversation-id="${group.id}"]`).click();
    const open = async (text: string) => {
      await page.locator(".chat-primary .msg-group").filter({ hasText: text }).locator(".msg-thread-rollup").click();
      await expect(page.getByRole("complementary", { name: "Thread" })).toContainText(text);
    };
    const input = page.getByRole("textbox", { name: "Reply in thread" });
    await open("first root");
    await input.fill("first unsent draft");
    await page.getByRole("button", { name: "Close thread" }).click();
    await open("first root");
    await expect(input).toHaveValue("first unsent draft");
    await open("second root");
    await expect(input).toHaveValue("");
    await input.fill("second unsent draft");
    await open("first root");
    await expect(input).toHaveValue("first unsent draft");
    await input.press("Enter");
    await expect(input).toHaveValue("");
    await expect(page.locator(".thread-panel .msg-group").filter({ hasText: "first unsent draft" })).toBeVisible();
    await open("second root");
    await expect(input).toHaveValue("second unsent draft");
    await open("first root");
    await expect(input).toHaveValue("");
    const gate = new Promise<void>((resolve) => { releaseSend = resolve; });
    await page.route("**/api/v1/messages", async (route) => {
      if (route.request().postDataJSON()?.content !== "held submission") return route.continue();
      const response = await route.fetch();
      await gate;
      await route.fulfill({ response });
    });
    const pending = page.waitForRequest((request) => request.method() === "POST" && request.url().endsWith("/api/v1/messages") && request.postDataJSON()?.content === "held submission");
    const completed = page.waitForResponse((response) => response.request().postDataJSON()?.content === "held submission");
    await input.fill("held submission");
    await input.press("Enter");
    await pending;
    await page.getByRole("button", { name: "Close thread" }).click();
    await open("first root");
    await input.fill("newer draft");
    releaseSend();
    await (await completed).finished();
    await page.getByRole("button", { name: "Close thread" }).click();
    await open("first root");
    await expect(input).toHaveValue("newer draft");
  } finally {
    releaseSend();
    await deleteCompany(page, token, company.id);
  }
});

test("thread unread badge: lights on agent reply, survives conversation view, clears on thread view", async ({ page }) => {
  const { token, principalId } = await loginAndSetCookie(page);

  // Agent principal so the unread is attributed to ANOTHER sender (the
  // unreads LATERAL excludes the caller's own replies).
  const agentRes = await page.request.post(`${API_BASE}/v1/agents`, {
    headers: { Authorization: `Bearer ${token}` },
    data: {
      actor_id: principalId,
      name: `threads-e2e-agent-${Date.now()}`,
      scopes: ["messages:write"],
    },
  });
  expect(agentRes.ok()).toBeTruthy();
  const { principal: agent, secret } = await agentRes.json();

  const groupName = `threads-badge-${Date.now()}`;
  const groupRes = await page.request.post(`${API_BASE}/v1/groups`, {
    headers: { Authorization: `Bearer ${token}` },
    data: {
      actor_id: principalId,
      name: groupName,
      description: null,
      avatar_url: null,
      member_ids: [agent.id],
    },
  });
  expect(groupRes.ok()).toBeTruthy();
  const convId = (await groupRes.json()).id as string;

  const rootText = `badge-root-${Date.now()}`;
  const root = await sendMessage(page, token, principalId, convId, rootText);

  // Agent (authenticated by its secret) posts a QUIET thread reply.
  const agentReply = await page.request.post(`${API_BASE}/v1/messages`, {
    headers: { Authorization: `Bearer ${secret}` },
    data: {
      actor_id: agent.id,
      conversation_id: convId,
      idempotency_key: `e2e-badge-${Date.now()}`,
      content: `agent-quiet-${Date.now()}`,
      content_type: "text",
      metadata: { reply_to_id: root.id, thread: true },
    },
  });
  expect(agentReply.ok(), `agent reply: ${agentReply.status()}`).toBeTruthy();

  // Load the dashboard (no active conversation): the initial /v1/unreads
  // fetch must light the thread badge; the conversation badge must NOT be
  // lit (quiet replies don't bump total_msg_count).
  await page.goto(`${WEB_BASE}/dashboard`);
  // Expand the group section ONLY if collapsed — an unconditional click
  // would toggle an already-expanded section closed and flake the test.
  const groupSection = page.getByRole("button", { name: /Group Conversations/ });
  await expect(groupSection).toBeVisible({ timeout: 15000 });
  if ((await groupSection.getAttribute("aria-expanded")) !== "true") {
    await groupSection.click();
  }
  const convItem = page.locator(`.conv-item:has-text("${groupName}")`);
  await expect(convItem).toBeVisible({ timeout: 15000 });
  await expect(convItem.locator(".conv-thread-unread")).toBeVisible({ timeout: 15000 });
  await expect(convItem.locator(".conv-thread-unread")).toHaveText("1");
  await expect(convItem.locator(".conv-unread")).toHaveCount(0);

  // Opening the CONVERSATION does not read the thread — badge survives.
  await convItem.click();
  await expect(page.locator(`.msg-group:has-text("${rootText}")`)).toBeVisible({
    timeout: 10000,
  });
  await expect(convItem.locator(".conv-thread-unread")).toBeVisible();

  // Opening the THREAD posts the receipt and clears the badge.
  await page.locator(".msg-thread-rollup").click();
  await expect(page.locator(".thread-panel")).toBeVisible();
  await expect(convItem.locator(".conv-thread-unread")).toHaveCount(0, {
    timeout: 15000,
  });
});

test("broadcast reply shows in both the timeline and the thread", async ({ page }) => {
  const { token, principalId } = await loginAndSetCookie(page);

  const groupRes = await page.request.post(`${API_BASE}/v1/groups`, {
    headers: { Authorization: `Bearer ${token}` },
    data: {
      actor_id: principalId,
      name: `threads-bcast-e2e-${Date.now()}`,
      description: null,
      avatar_url: null,
      member_ids: [],
    },
  });
  expect(groupRes.ok()).toBeTruthy();
  const convId = (await groupRes.json()).id as string;

  const rootText = `bcast-root-${Date.now()}`;
  const bcastText = `bcast-reply-${Date.now()}`;
  const root = await sendMessage(page, token, principalId, convId, rootText);
  await sendMessage(page, token, principalId, convId, bcastText, {
    reply_to_id: root.id,
    thread: true,
    broadcast: true,
  });

  await page.goto(`${WEB_BASE}/dashboard?conversationId=${convId}`);
  await page.waitForSelector(".messages-area", { timeout: 10000 });

  // Broadcast reply IS on the timeline…
  await expect(
    page.locator(`.chat-primary .msg-group:has-text("${bcastText}")`),
  ).toBeVisible({ timeout: 10000 });

  // …AND counted in the root's rollup; opening the thread shows it too.
  const rollup = page.locator(".msg-thread-rollup");
  await expect(rollup).toContainText("1 reply");
  await rollup.click();
  const panel = page.locator(".thread-panel");
  await expect(panel.locator(`.msg-group:has-text("${bcastText}")`)).toBeVisible({
    timeout: 10000,
  });
});
