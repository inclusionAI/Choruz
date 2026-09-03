import { expect, test, type Page } from "@playwright/test";
import { API_BASE, CREDENTIALS, WEB_BASE } from "../fixtures/auth";

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

test("sending a message should show exactly one bubble, not duplicated", async ({ page }) => {
  const { token, principalId } = await loginAndSetCookie(page);

  // Find a group conversation via API
  const consoleRes = await page.request.get(`${API_BASE}/v1/console`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  const snapshot = await consoleRes.json();
  const groupConv = snapshot.conversations?.find(
    (c: { conversation_type: string }) => c.conversation_type === "group",
  );
  if (!groupConv) {
    console.log("No group conversation found, skipping");
    test.skip();
    return;
  }

  await page.goto(`${WEB_BASE}/dashboard?conversationId=${groupConv.id}`);
  await page.waitForSelector(".conv-item", { timeout: 10000 });

  if (groupConv.name) {
    await expect(page.getByRole("heading", { name: groupConv.name, level: 1 })).toBeVisible();
  }

  // Wait for chat input to appear (group chats have text input, not terminal)
  const textarea = page.locator("textarea");
  try {
    await textarea.first().waitFor({ state: "visible", timeout: 5000 });
  } catch {
    console.log("No text input found (terminal chat), skipping");
    test.skip();
    return;
  }

  const testMsg = `dedup-test-${Date.now()}`;
  await textarea.first().fill(testMsg);
  await textarea.first().press("Enter");

  // Wait for WS to push confirmation
  await page.waitForTimeout(4000);

  // Screenshot for visual verification
  await page.screenshot({ path: "test-results/message-dedup.png", fullPage: false });

  // Count bubbles
  const bubbles = await page.locator(`.msg-group:has-text("${testMsg}")`).count();
  console.log(`Message "${testMsg}" appeared ${bubbles} time(s)`);
  expect(bubbles).toBe(1);
});
