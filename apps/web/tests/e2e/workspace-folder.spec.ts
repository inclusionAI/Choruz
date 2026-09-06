import { mkdir, mkdtemp, realpath, rm } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { expect, test } from "@playwright/test";
import { API_BASE, login, gotoDashboard } from "../fixtures/auth";
import { createCompany, deleteCompany, uniqueName } from "../fixtures/api";

for (const scenario of ["restore", "typed"] as const) {
  test(`workspace folder ${scenario === "restore" ? "can be removed and selected again" : "confirmation validates the typed path"}`, async ({ page }) => {
    const root = await mkdtemp(join(homedir(), ".choruz-folder-test-"));
    let companyId: string | undefined;
    let token = "";
    try {
      const first = join(await realpath(root), "first");
      const second = join(await realpath(root), "second");
      await mkdir(first);
      await mkdir(second);
      await mkdir(join(first, "child"));
      const auth = await login(page);
      token = auth.token;
      const company = await createCompany(page, token, auth.principal.id, uniqueName("folder"));
      companyId = company.id;
      const headers = { Authorization: `Bearer ${token}` };
      const seeded = await page.request.patch(`${API_BASE}/v1/companies/${company.id}`, {
        headers, data: { actor_id: auth.principal.id, folder_path: first },
      });
      expect(seeded.ok()).toBeTruthy();
      const persistedFolder = async () => {
        const response = await page.request.get(`${API_BASE}/v1/companies`, { headers });
        expect(response.ok()).toBeTruthy();
        const companies = await response.json() as Array<{ id: string; folder_path: string | null }>;
        return companies.find((item) => item.id === company.id)?.folder_path;
      };
      await gotoDashboard(page);
      await page.locator(".company-selector-btn").click();
      await page.locator(".company-dropdown-item").filter({ hasText: company.name })
        .locator(".company-dropdown-item-name").click();
      await page.getByRole("button", { name: "Change workspace folder" }).click();
      const dialog = page.getByRole("dialog", { name: "Select Folder" });
      const input = dialog.getByPlaceholder("/path/to/folder");
      await expect(input).toHaveValue(first);
      if (scenario === "restore") {
        await dialog.getByRole("button", { name: "Remove workspace folder" }).click();
        await expect(dialog).toBeHidden();
        await expect.poll(persistedFolder).toBeNull();
        const chooseFolder = page.getByRole("button", { name: "Choose workspace folder" });
        await expect(chooseFolder).toBeVisible();
        await chooseFolder.click();
        await expect(dialog).toBeVisible();
      } else {
        await dialog.getByText("child", { exact: true }).click();
        await input.fill(join(root, "missing"));
        await dialog.getByRole("button", { name: "Select This Folder" }).click();
        await expect(dialog).toBeVisible();
        await expect(dialog.getByText("Cannot read this directory", { exact: true })).toBeVisible();
        expect(await persistedFolder()).toBe(first);
      }
      await input.fill(second);
      await dialog.getByRole("button", { name: "Select This Folder" }).click();
      await expect(dialog).toBeHidden();
      await expect.poll(persistedFolder).toBe(second);
      await expect(page.locator(".company-selector-name")).toHaveText(company.name);
    } finally {
      try {
        if (companyId) await deleteCompany(page, token, companyId);
      } finally {
        await rm(root, { recursive: true, force: true });
      }
    }
  });
}
