import { expect, test } from "@playwright/test";
import { mkdtemp, mkdir, writeFile, readFile, rm } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { login, gotoDashboard, API_BASE } from "../fixtures/auth";
import { deleteCompany } from "../fixtures/api";

for (const scenario of ["save after undo", "retry a failed save", "ignore a previous file read", "resolve external changes", "read code across theme changes"] as const) {
  test(`File editor can ${scenario}`, async ({ page }, testInfo) => {
    const { token, principal } = await login(page);
    const root = await mkdtemp(join(homedir(), "choruz-editor-test-"));
    const filename = scenario === "read code across theme changes" ? "owned.tsx" : "owned.txt";
    const path = join(root, filename);
    let companyId: string | undefined;
    let releaseRead = () => {};
    try {
      await writeFile(path, "original");
      const oldPath = join(root, "old.txt");
      await writeFile(oldPath, "previous file");
      let oldRead: Promise<import("@playwright/test").Response> | undefined;
      if (scenario === "ignore a previous file read") {
        const gate = new Promise<void>((resolve) => { releaseRead = resolve; });
        await page.route((url) => url.pathname === "/api/filesystem" && url.searchParams.get("path") === oldPath && url.searchParams.get("action") === "read", async (route) => {
          const response = await route.fetch();
          await gate;
          await route.fulfill({ response });
        });
      }
      const name = root.split("/").pop()!;
      const response = await page.request.post(`${API_BASE}/v1/companies`, {
        headers: { Authorization: `Bearer ${token}` },
        data: { actor_id: principal.id, name, folder_path: root },
      });
      expect(response.ok(), await response.text()).toBeTruthy();
      companyId = (await response.json()).id;
      await gotoDashboard(page);
      await page.locator(".company-selector-btn").click();
      await page.locator(".company-dropdown-item").filter({ hasText: name })
        .locator(".company-dropdown-item-name").click();
      await page.getByRole("button", { name: "Project files", exact: true }).click();
      if (scenario === "ignore a previous file read") {
        oldRead = page.waitForResponse((response) => new URL(response.url()).searchParams.get("path") === oldPath);
        const started = page.waitForRequest((request) => new URL(request.url()).searchParams.get("path") === oldPath);
        await page.getByRole("treeitem").filter({ hasText: "old.txt" }).click();
        await started;
      }
      await page.getByRole("treeitem").filter({ hasText: filename }).click();
      const editor = page.locator(".cm-content");
      await expect(editor).toHaveText("original");
      await editor.fill(scenario === "read code across theme changes" ? 'const answer = "ready";\n// useful note\nanswer;' : "edited");
      await expect(page.locator(".file-editor-dirty")).toHaveText("Modified");
      if (scenario === "read code across theme changes") {
        for (const theme of ["light", "dark", "light"] as const) {
          if (await page.locator("html").getAttribute("data-theme") !== theme) {
            await page.getByRole("button", { name: "User menu", exact: true }).click();
            await page.getByRole("button", { name: `Switch to ${theme} mode`, exact: true }).click();
          }
          const code = page.locator(".cm-editor");
          await expect.poll(() => code.locator(".cm-line span").filter({ hasText: /^const$/ }).evaluate((element) => getComputedStyle(element).color))
            .toBe(theme === "light" ? "rgb(115, 60, 151)" : "rgb(199, 146, 234)");
          await expect(editor).toHaveText('const answer = "ready";// useful noteanswer;');
          const contrast = await code.evaluate((element) => {
            const rgb = (text: string) => text.match(/[\d.]+/g)!.slice(0, 3).map(Number);
            const luminance = (channels: number[]) => channels.map((channel) => {
              const value = channel / 255;
              return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
            }).reduce((sum, value, index) => sum + value * [0.2126, 0.7152, 0.0722][index], 0);
            const background = luminance(rgb(getComputedStyle(element).backgroundColor));
            return Array.from(element.querySelectorAll(".cm-line span, .cm-lineNumbers .cm-gutterElement"))
              .filter((node) => node.textContent?.trim())
              .map((node) => {
                const foreground = luminance(rgb(getComputedStyle(node).color));
                return (Math.max(background, foreground) + 0.05) / (Math.min(background, foreground) + 0.05);
              });
          });
          expect(contrast.length).toBeGreaterThan(4);
          expect(Math.min(...contrast)).toBeGreaterThanOrEqual(4.5);
          await page.screenshot({ path: testInfo.outputPath(`editor-${theme}.png`) });
        }
        await editor.press("ControlOrMeta+z");
        await expect(editor).toHaveText("original");
        expect(await readFile(path, "utf8")).toBe("original");
      } else if (scenario === "save after undo") {
        await editor.press("ControlOrMeta+s");
        await expect.poll(() => readFile(path, "utf8")).toBe("edited");
        await expect(page.locator(".file-editor-dirty")).toHaveCount(0);
        await editor.press("ControlOrMeta+z");
        await expect(editor).toHaveText("original");
        await editor.press("ControlOrMeta+s");
        await expect.poll(() => readFile(path, "utf8")).toBe("original");
      } else if (scenario === "retry a failed save") {
        // A real filesystem write fails; the draft and retry must survive it.
        await rm(path);
        await mkdir(path);
        await page.getByRole("button", { name: "Save", exact: true }).click();
        await expect(page.locator(".file-editor-error")).toBeVisible();
        await expect(editor).toHaveText("edited");
        await rm(path, { recursive: true });
        await writeFile(path, "original");
        await page.getByRole("button", { name: "Save", exact: true }).click();
        await expect.poll(() => readFile(path, "utf8")).toBe("edited");
        await expect(page.locator(".file-editor-error")).toHaveCount(0);
      } else if (scenario === "resolve external changes") {
        await writeFile(path, "external edit");
        const saved = page.waitForResponse((response) => new URL(response.url()).pathname === "/api/filesystem" && response.request().method() === "POST");
        await page.getByRole("button", { name: "Save", exact: true }).click();
        await saved;
        expect(await readFile(path, "utf8")).toBe("external edit");
        await expect(page.getByRole("button", { name: "Overwrite", exact: true })).toBeVisible();
        await expect(editor).toHaveText("edited");
        expect(await readFile(path, "utf8")).toBe("external edit");
        await writeFile(path, "another external edit");
        await page.getByRole("button", { name: "Overwrite", exact: true }).click();
        await expect(page.locator(".file-editor-error")).toContainText("changed");
        expect(await readFile(path, "utf8")).toBe("another external edit");
        await page.getByRole("button", { name: "Reload", exact: true }).click();
        await expect(editor).toHaveText("another external edit");
        await editor.fill("chosen draft");
        await writeFile(path, "final external edit");
        await page.getByRole("button", { name: "Save", exact: true }).click();
        await expect(page.getByRole("button", { name: "Overwrite", exact: true })).toBeVisible();
        await page.getByRole("button", { name: "Overwrite", exact: true }).click();
        await expect.poll(() => readFile(path, "utf8")).toBe("chosen draft");
      } else {
        releaseRead();
        await (await oldRead!).finished();
        await editor.press("ControlOrMeta+s");
        await expect.poll(() => readFile(path, "utf8")).toBe("edited");
        await expect(editor).toHaveText("edited");
      }
      await expect(page.locator(".file-editor-dirty")).toHaveCount(0);
    } finally {
      releaseRead();
      try {
        if (companyId) await deleteCompany(page, token, companyId);
      } finally {
        await rm(root, { recursive: true, force: true });
      }
    }
  });
}
