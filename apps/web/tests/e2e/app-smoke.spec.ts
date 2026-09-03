import { expect, test } from "@playwright/test";
import { API_BASE, CREDENTIALS, WEB_BASE } from "../fixtures/auth";

test("local login reaches the dashboard and sends a group message", async ({ page }) => {
  await page.goto("/");

  await expect(page.getByRole("img", { name: "Choruz" })).toBeVisible();

  const apiLogin = await page.request.post(`${API_BASE}/v1/auth/local/login`, {
    data: {
      username: CREDENTIALS.username,
      password: CREDENTIALS.password,
    },
  });
  expect(apiLogin.ok()).toBeTruthy();
  const loginPayload = await apiLogin.json();
  await page.context().addCookies([
    {
      name: "choruz_session",
      value: loginPayload.session_token,
      url: WEB_BASE,
      httpOnly: true,
      sameSite: "Lax",
      expires: Math.floor(Date.now() / 1000) + 60 * 60,
    },
  ]);

  const staleGroupName = `Playwright Stale Group ${Date.now()}`;
  const createStaleGroup = await page.request.post(`${API_BASE}/v1/groups`, {
    headers: { Authorization: `Bearer ${loginPayload.session_token}` },
    data: {
      actor_id: loginPayload.principal.id,
      name: staleGroupName,
      description: null,
      avatar_url: null,
      member_ids: [],
    },
  });
  expect(createStaleGroup.ok()).toBeTruthy();
  const staleGroup = await createStaleGroup.json();

  const groupName = `Playwright Group ${Date.now()}`;
  const createGroup = await page.request.post(`${API_BASE}/v1/groups`, {
    headers: { Authorization: `Bearer ${loginPayload.session_token}` },
    data: {
      actor_id: loginPayload.principal.id,
      name: groupName,
      description: null,
      avatar_url: null,
      member_ids: [],
    },
  });
  expect(createGroup.ok()).toBeTruthy();
  const group = await createGroup.json();

  await page.evaluate((conversationId) => {
    localStorage.setItem("choruz_active_conv", conversationId);
  }, staleGroup.id);
  await page.goto(`/dashboard?conversationId=${group.id}`);
  await expect(page.locator(".sidebar-header").getByText("operator", { exact: true })).toBeVisible();
  await expect(page.getByRole("heading", { name: groupName, level: 1 })).toBeVisible();
  await expect(page.getByRole("heading", { name: staleGroupName, level: 1 })).toHaveCount(0);
  await expect.poll(() => page.evaluate(() => localStorage.getItem("choruz_active_conv"))).toBe(group.id);
  const groupItem = page.locator(".conv-item").filter({ hasText: groupName }).first();
  await expect(groupItem).toBeVisible();
  await groupItem.click();

  await page.getByPlaceholder(`Message ${groupName}...`).fill("hello from playwright");
  await page.getByRole("button", { name: "Send message" }).click();
  await expect(page.locator(".msg-group").filter({ hasText: "hello from playwright" }).first()).toBeVisible();
});
