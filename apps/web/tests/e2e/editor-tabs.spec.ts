import { expect, test } from "@playwright/test";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { login, gotoDashboard, API_BASE } from "../fixtures/auth";
import { createCompany, createGroup, deleteCompany } from "../fixtures/api";

test("owned tabs switch and close while groups restore only through an explicit link", async ({ page }) => {
  const { token, principal } = await login(page);
  const root = await mkdtemp(join(homedir(), "choruz-tabs-test-"));
  const name = root.split("/").pop()!;
  let companyId: string | undefined;
  let otherCompanyId: string | undefined;
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
    const other = await createCompany(page, token, principal.id, name + "-other");
    otherCompanyId = other.id;
    const selectProject = async (label: string) => {
      await page.locator(".company-selector-btn").click();
      await page.locator(".company-dropdown-item-name").getByText(label, { exact: true }).click();
    };
    await gotoDashboard(page);
    await selectProject(name);
    await page.getByRole("button", { name: "Project files", exact: true }).click();
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
    await page.locator(".cm-content").fill("Unsaved owned tab content");
    await expect(page.locator(".file-editor-dirty")).toHaveText("Modified");
    await tab(first.name).locator(".editor-tab-main").click();
    await expect(page.locator(".chat-header")).toContainText(first.name);
    await tab("owned.txt").locator(".editor-tab-main").click();
    await expect(page.locator(".cm-content")).toHaveText("Unsaved owned tab content");
    expect(await readFile(join(root, "owned.txt"), "utf8")).toBe("Owned tab content");
    await selectProject(other.name);
    await selectProject(name);
    await page.getByRole("treeitem").filter({ hasText: "owned.txt" }).click();
    await expect(page.locator(".cm-content")).toHaveText("Unsaved owned tab content");
    await page.reload();
    await page.getByRole("button", { name: "Project files", exact: true }).click();
    await page.getByRole("treeitem").filter({ hasText: "owned.txt" }).click();
    await expect(page.locator(".cm-content")).toHaveText("Unsaved owned tab content");
    page.once("dialog", (dialog) => dialog.dismiss());
    await page.getByRole("button", { name: "Close owned.txt", exact: true }).click();
    await expect(tab("owned.txt")).toHaveCount(1);
    await page.locator(".file-editor-actions").getByRole("button", { name: "Save", exact: true }).click();
    await expect.poll(() => readFile(join(root, "owned.txt"), "utf8")).toBe("Unsaved owned tab content");
    await page.getByRole("button", { name: "Close owned.txt", exact: true }).click();
    await expect(tab("owned.txt")).toHaveCount(0);
    await page.evaluate((id) => localStorage.setItem("choruz_active_conv", id), second.id);
    await page.reload();
    await expect(page.getByRole("heading", { name: "What would you like to work on?" })).toBeVisible();
    await expect(page.getByRole("group", { name: "Group Conversations", exact: true })).toHaveCount(0);
    const deepLink = new URL(page.url());
    deepLink.searchParams.set("conversationId", second.id);
    await page.goto(deepLink.toString());
    await expect(page.locator(".chat-header")).toContainText(second.name);
  } finally {
    try {
      if (companyId) await deleteCompany(page, token, companyId);
      if (otherCompanyId) await deleteCompany(page, token, otherCompanyId);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  }
});
