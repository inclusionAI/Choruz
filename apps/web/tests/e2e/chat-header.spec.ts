import { expect, test } from "@playwright/test";
import { login, WEB_BASE } from "../fixtures/auth";
import { createGroup, uniqueName } from "../fixtures/api";

test("group header identifies the selected conversation and opens its details", async ({ page }) => {
  const { token, principal } = await login(page);
  const group = await createGroup(page, token, principal.id, uniqueName("header"));
  await page.goto(`${WEB_BASE}/dashboard?conversationId=${group.id}`);
  const header = page.locator(".chat-header");
  await expect(header.getByRole("heading", { name: group.name, exact: true })).toBeVisible();
  await expect(header.locator(".avatar")).toBeVisible();
  await expect(header.locator(".chat-subtitle")).toContainText("1 member");
  await header.getByTitle("Toggle details").click();
  await expect(page.locator(".detail-identity-title")).toHaveText(group.name);
});
