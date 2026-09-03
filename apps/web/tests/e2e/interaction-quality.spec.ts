import { expect, test } from "@playwright/test";

import { API_BASE, login, gotoDashboard } from "../fixtures/auth";
import { createGroup, sendMessage, uniqueName } from "../fixtures/api";

test("sidebar menus close with Escape and restore focus", async ({ page }) => {
  await login(page);
  await gotoDashboard(page);
  const actions = page.getByRole("button", { name: "Actions menu" });
  await actions.click();
  await expect(page.getByRole("button", { name: "New Group" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("button", { name: "New Group" })).toBeHidden();
  await expect(actions).toBeFocused();

  const userMenu = page.getByRole("button", { name: "User menu" });
  await userMenu.focus();
  await userMenu.press("Enter");
  await expect(page.getByRole("link", { name: "Documentation" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(userMenu).toBeFocused();
});

test("own messages stay aligned to the right edge of a wide conversation", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const { token, principal } = await login(page);
  const group = await createGroup(page, token, principal.id, uniqueName("self-align"));
  const content = uniqueName("right-edge-message");

  try {
    await sendMessage(page, token, principal.id, group.id, content);
    await page.goto(`/dashboard?conversationId=${group.id}`);
    const ownMessage = page.locator(".msg-group.self").filter({ hasText: content });
    await expect(ownMessage).toBeVisible({ timeout: 15_000 });

    const [messageBox, areaBox] = await Promise.all([
      ownMessage.boundingBox(),
      page.locator(".messages-area").boundingBox(),
    ]);
    expect(messageBox).not.toBeNull();
    expect(areaBox).not.toBeNull();
    const rightGap = areaBox!.x + areaBox!.width - (messageBox!.x + messageBox!.width);
    expect(rightGap).toBeGreaterThanOrEqual(10);
    expect(rightGap).toBeLessThanOrEqual(32);
  } finally {
    const cleanup = await page.request.post(`${API_BASE}/v1/agents/batch-disable`, {
      headers: { authorization: `Bearer ${token}` },
      data: {
        actor_id: principal.id,
        agent_ids: [],
        conversation_ids: [group.id],
      },
    });
    expect(cleanup.ok()).toBe(true);
  }
});
