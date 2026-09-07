import { expect, test } from "@playwright/test";
import { login, gotoDashboard } from "../fixtures/auth";

test("user-menu theme selection changes the page and persists across reload", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" });
  await login(page);
  await gotoDashboard(page);
  const html = page.locator("html");
  await expect(html).toHaveAttribute("data-theme", "light");
  const lightBackground = await page.locator("body").evaluate((body) => getComputedStyle(body).backgroundColor);
  await page.getByRole("button", { name: "User menu", exact: true }).click();
  await page.getByRole("button", { name: "Switch to dark mode", exact: true }).click();
  await expect(html).toHaveAttribute("data-theme", "dark");
  await expect.poll(() => page.locator("body").evaluate((body) => getComputedStyle(body).backgroundColor)).not.toBe(lightBackground);
  await page.reload();
  await expect(html).toHaveAttribute("data-theme", "dark");
  await page.getByRole("button", { name: "User menu", exact: true }).click();
  await page.getByRole("button", { name: "Switch to light mode", exact: true }).click();
  await expect(html).toHaveAttribute("data-theme", "light");
});
