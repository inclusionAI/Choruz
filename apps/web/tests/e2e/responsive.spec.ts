import { expect, test } from "@playwright/test";
import { login, WEB_BASE } from "../fixtures/auth";
import { createGroup, uniqueName } from "../fixtures/api";

test("chat layout survives desktop, rail and mobile resizing", async ({ page }) => {
  const { token, principal } = await login(page);
  const group = await createGroup(page, token, principal.id, uniqueName("responsive"));
  await page.goto(`${WEB_BASE}/dashboard?conversationId=${group.id}`);
  await expect(page.locator(".chat-header")).toContainText(group.name);

  for (const width of [1280, 768, 375, 2560]) {
    await page.setViewportSize({ width, height: 900 });
    const main = page.locator(".chat-main");
    await expect(main).toBeVisible();
    await expect.poll(async () => {
      const box = await main.boundingBox();
      return box ? box.x >= 0 && box.x + box.width <= width + 1 : false;
    }).toBe(true);
    await expect.poll(() => page.evaluate(() =>
      document.documentElement.scrollWidth <= window.innerWidth,
    )).toBe(true);

    if (width <= 900) {
      const menu = page.locator(".mobile-menu-btn");
      await expect(menu).toBeVisible();
      await menu.click();
      await expect(page.locator(".chat-sidebar")).toHaveClass(/\bopen\b/);
      await expect(page.locator(".mobile-backdrop")).toBeVisible();
      if (width <= 600) {
        await page.locator(`[data-conversation-id="${group.id}"]`).click();
      } else {
        await page.locator(".mobile-backdrop").click({ position: { x: width - 5, y: 400 } });
      }
      await expect(page.locator(".chat-sidebar")).not.toHaveClass(/\bopen\b/);
      await expect(page.locator(".chat-header")).toContainText(group.name);
    } else {
      const sidebar = await page.locator(".chat-sidebar").boundingBox();
      const chat = await main.boundingBox();
      expect(sidebar).not.toBeNull();
      expect(chat).not.toBeNull();
      expect(sidebar!.x + sidebar!.width).toBeLessThanOrEqual(chat!.x + 1);
    }
  }
});
