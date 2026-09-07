import { expect, test } from "@playwright/test";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { login, gotoDashboard, API_BASE } from "../fixtures/auth";
import { createGroup, deleteCompany } from "../fixtures/api";

test("owned conversation and file tabs switch, deduplicate, close and restore", async ({ page }) => {
  const { token, principal } = await login(page);
  const root = await mkdtemp(join(homedir(), "choruz-tabs-test-"));
  const name = root.split("/").pop()!;
  let companyId: string | undefined;
  try {
    await writeFile(join(root, "owned.txt"), "Owned tab content");
    const response = await page.request.post(`${API_BASE}/v1/companies`, {
      headers: { Authorization: `Bearer ${token}` },
      data: { actor_id: principal.id, name, folder_path: root },
    });
    expect(response.ok(), await response.text()).toBeTruthy();
    companyId = (await response.json()).id;
    const first = await createGroup(page, token, principal.id, name + "-a", [], companyId);
    const second = await createGroup(page, token, principal.id, name + "-b", [], companyId);
    await gotoDashboard(page);
    await page.locator(".company-selector-btn").click();
    await page.locator(".company-dropdown-item").filter({ hasText: name })
      .locator(".company-dropdown-item-name").click();
    const row = (label: string) => page.locator(".conv-item").filter({ hasText: label });
    const tab = (label: string) => page.locator(".editor-tab").filter({ hasText: label });
    await row(first.name).click();
    await row(second.name).click();
    await expect(tab(first.name)).toHaveCount(1);
    await expect(tab(second.name)).toHaveClass(/active/);
    await row(second.name).click();
    await expect(tab(second.name)).toHaveCount(1);
    await page.getByRole("treeitem").filter({ hasText: "owned.txt" }).click();
    await expect(page.locator(".cm-content")).toHaveText("Owned tab content");
    await expect(tab("owned.txt")).toHaveClass(/active/);
    await tab(first.name).locator(".editor-tab-main").click();
    await expect(page.locator(".chat-header")).toContainText(first.name);
    await tab("owned.txt").locator(".editor-tab-main").click();
    await expect(page.locator(".cm-content")).toHaveText("Owned tab content");
    await page.getByRole("button", { name: "Close owned.txt", exact: true }).click();
    await expect(tab("owned.txt")).toHaveCount(0);
    await tab(second.name).locator(".editor-tab-main").click();
    await page.reload();
    await expect(page.locator(".chat-header")).toContainText(second.name);
  } finally {
    try {
      if (companyId) await deleteCompany(page, token, companyId);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  }
});
