import { expect, test } from "@playwright/test";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { login, gotoDashboard, API_BASE } from "../fixtures/auth";
import { deleteCompany } from "../fixtures/api";

test("Explorer reloads after workspace switches and retries failed directory reads", async ({ page }) => {
  const { token, principal } = await login(page);
  const root = await mkdtemp(join(homedir(), "choruz-explorer-test-"));
  const companies: Array<{ id: string; name: string }> = [];
  const headers = { Authorization: `Bearer ${token}` };
  const switchTo = async (company: { name: string }) => {
    await page.locator(".company-selector-btn").click();
    await page.locator(".company-dropdown-item").filter({ hasText: company.name })
      .locator(".company-dropdown-item-name").click();
    await expect(page.locator(".company-selector-name")).toHaveText(company.name);
  };
  const folder = (name: string) => page.getByRole("treeitem").filter({ hasText: name });
  try {
    for (const name of ["a", "b"]) {
      const path = join(root, name);
      await mkdir(join(path, "src"), { recursive: true });
      await writeFile(join(path, "src", `${name}.txt`), `owned by ${name}\n`);
      const response = await page.request.post(`${API_BASE}/v1/companies`, {
        headers, data: { actor_id: principal.id, name: `${root.split("/").pop()}-${name}`, folder_path: path },
      });
      expect(response.ok(), await response.text()).toBeTruthy();
      companies.push(await response.json());
    }
    await gotoDashboard(page);
    await switchTo(companies[0]);
    await folder("src").click();
    await expect(folder("a.txt")).toBeVisible();
    await switchTo(companies[1]);
    await folder("src").click();
    await expect(folder("b.txt")).toBeVisible();
    await expect(folder("a.txt")).toHaveCount(0);
    await switchTo(companies[0]);
    await folder("src").click();
    await expect(folder("a.txt")).toBeVisible();

    // The directory is removed on disk, not replaced with a successful route mock.
    await rm(join(root, "a", "src"), { recursive: true });
    await page.getByRole("button", { name: "Refresh file tree" }).click();
    await expect(page.locator(".file-tree-error")).toBeVisible();
    await mkdir(join(root, "a", "src"));
    await writeFile(join(root, "a", "src", "recovered.txt"), "recovered\n");
    await page.getByRole("button", { name: "Retry", exact: true }).click();
    await expect(folder("recovered.txt")).toBeVisible();
    await expect(page.locator(".file-tree-error")).toHaveCount(0);
    await folder("recovered.txt").click();
    await expect(page.locator(".cm-content")).toContainText("recovered");
  } finally {
    for (const company of companies) await deleteCompany(page, token, company.id);
    await rm(root, { recursive: true, force: true });
  }
});
