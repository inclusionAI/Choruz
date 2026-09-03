import { expect, test, type Page } from "@playwright/test";
import { API_BASE, CREDENTIALS, WEB_BASE } from "../fixtures/auth";

/**
 * Quote-reply preview E2E (WeChat/Feishu-style): replying to a message
 * that has fallen OUTSIDE the loaded history window must still show the
 * original's content in the quote block — fetched on demand from
 * GET /v1/conversations/{id}/messages/{message_id} — instead of a
 * "not loaded" placeholder.
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
      idempotency_key: `e2e-quote-${Date.now()}-${Math.random().toString(16).slice(2)}`,
      content,
      content_type: "text",
      metadata,
    },
  });
  expect(res.ok(), `send "${content}": ${res.status()}`).toBeTruthy();
  return (await res.json()) as { id: string };
}

test("quote block fetches an original outside the loaded history window", async ({ page }) => {
  const { token, principalId } = await loginAndSetCookie(page);

  const groupRes = await page.request.post(`${API_BASE}/v1/groups`, {
    headers: { Authorization: `Bearer ${token}` },
    data: {
      actor_id: principalId,
      name: `quotes-e2e-${Date.now()}`,
      description: null,
      avatar_url: null,
      member_ids: [],
    },
  });
  expect(groupRes.ok()).toBeTruthy();
  const convId = (await groupRes.json()).id as string;

  // The original, then 60 filler messages to push it well past the
  // 50-message initial history window.
  const originalText = `ancient-original-${Date.now()}`;
  const original = await sendMessage(page, token, principalId, convId, originalText);
  for (let i = 0; i < 60; i++) {
    await sendMessage(page, token, principalId, convId, `filler ${i}`);
  }
  const replyText = `reply-to-ancient-${Date.now()}`;
  await sendMessage(page, token, principalId, convId, replyText, {
    reply_to_id: original.id,
  });

  await page.goto(`${WEB_BASE}/dashboard?conversationId=${convId}`);
  const replyBubble = page.locator(`.msg-group:has-text("${replyText}")`);
  await expect(replyBubble).toBeVisible({ timeout: 15000 });

  // Sanity: the original itself must NOT be in the loaded timeline (it
  // fell outside the window) — otherwise this test isn't exercising the
  // on-demand path. The reply's QUOTE block also contains the original
  // text, so assert on message BODIES (.msg-markdown), not whole groups.
  await expect(
    page.locator(`.msg-markdown:has-text("${originalText}")`),
  ).toHaveCount(0);

  // The quote block must show the original's real content, fetched on
  // demand — not a "not loaded" / "unavailable" placeholder.
  const quote = replyBubble.locator(".msg-quote");
  await expect(quote).toBeVisible({ timeout: 10000 });
  await expect(quote).toContainText(originalText, { timeout: 10000 });

  await page.screenshot({ path: "test-results/quotes-e2e.png", fullPage: false });
});

test("quote block marks a deleted original as unavailable", async ({ page }) => {
  const { token, principalId } = await loginAndSetCookie(page);

  const groupRes = await page.request.post(`${API_BASE}/v1/groups`, {
    headers: { Authorization: `Bearer ${token}` },
    data: {
      actor_id: principalId,
      name: `quotes-e2e-missing-${Date.now()}`,
      description: null,
      avatar_url: null,
      member_ids: [],
    },
  });
  expect(groupRes.ok()).toBeTruthy();
  const convId = (await groupRes.json()).id as string;

  // Reply to an id that never existed in this conversation — the fetch
  // 404s and the block must settle on "unavailable", not spin forever.
  const replyText = `reply-to-ghost-${Date.now()}`;
  await sendMessage(page, token, principalId, convId, replyText, {
    reply_to_id: "msg-never-existed",
  });

  await page.goto(`${WEB_BASE}/dashboard?conversationId=${convId}`);
  const replyBubble = page.locator(`.msg-group:has-text("${replyText}")`);
  await expect(replyBubble).toBeVisible({ timeout: 15000 });
  await expect(replyBubble.locator(".msg-quote")).toContainText(
    "original message unavailable",
    { timeout: 10000 },
  );
});
