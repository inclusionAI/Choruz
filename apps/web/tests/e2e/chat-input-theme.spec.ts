import { expect, test } from "@playwright/test";

import { createGroup, uniqueName } from "../fixtures/api";
import { gotoDashboard, login } from "../fixtures/auth";

test("keeps the focused composer light after the pointer leaves", async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("theme", "light"));
  const { token, principal } = await login(page);
  const group = await createGroup(
    page,
    token,
    principal.id,
    uniqueName("light-composer"),
  );
  await gotoDashboard(page);

  await page.locator(".conv-item").filter({ hasText: group.name }).click();
  const textarea = page.locator(".chat-input-row textarea");
  const composer = page.locator(".chat-input-row");
  await expect(textarea).toBeVisible();

  await textarea.focus();
  await composer.hover();
  await page.mouse.move(0, 0);

  await expect(textarea).toBeFocused();
  await expect(composer).toHaveCSS(
    "background-color",
    "rgba(255, 255, 255, 0.72)",
  );
});
