import { expect, test } from "@playwright/test";
import { login, WEB_BASE } from "../fixtures/auth";
import { createAgent, getConsoleSnapshot, getMessages, uniqueName } from "../fixtures/api";

test("local entry creates a group and preserves its first message", async ({ page }) => {
  await page.goto(WEB_BASE);
  await expect(page.getByRole("img", { name: "Choruz" })).toBeVisible();
  await expect(page).toHaveURL(url => url.pathname === "/dashboard");
  const { token, principal } = await login(page);
  const name = uniqueName("journey-group");
  const member = await createAgent(page, token, principal.id, uniqueName("journey-member"));
  await page.reload();
  await page.getByRole("button", { name: "Actions menu", exact: true }).click();
  await page.getByRole("button", { name: "New Group", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Create Group", exact: true });
  await dialog.getByRole("textbox", { name: "Group name", exact: true }).fill(name);
  await dialog.getByRole("button").filter({ hasText: member.principal.name }).click();
  await dialog.getByRole("button", { name: "Create (1 selected)", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole("heading", { name, level: 1 })).toBeVisible();
  const content = "Keep this owned journey message after reload.";
  const input = page.locator(".chat-input-bar textarea");
  await input.fill(content);
  await input.press("Enter");
  await expect(page.locator(".messages-area").getByText(content, { exact: true })).toBeVisible();
  const group = (await getConsoleSnapshot(page, token)).conversations.find(item => item.name === name);
  expect(group).toBeDefined();
  await expect.poll(async () => (await getMessages(page, token, principal.id, group!.id))
    .filter(message => message.content === content).length).toBe(1);
  await page.reload();
  await expect(page.getByRole("heading", { name, level: 1 })).toBeVisible();
  await expect(page.locator(".messages-area").getByText(content, { exact: true })).toBeVisible();
});
