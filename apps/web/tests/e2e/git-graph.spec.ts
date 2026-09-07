import { expect, test } from "@playwright/test";
import { execFile } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import { login, API_BASE, WEB_BASE } from "../fixtures/auth";
import { deleteCompany } from "../fixtures/api";

test("Git API returns owned repository commits and branch divergence", async ({ page }) => {
  const { token, principal } = await login(page);
  const root = await mkdtemp(join(homedir(), "choruz-git-test-"));
  const git = (...args: string[]) => promisify(execFile)("git", args, { cwd: root });
  let companyId: string | undefined;
  try {
    await git("init", "-b", "main");
    await git("config", "user.name", "Choruz test");
    await git("config", "user.email", "test@example.invalid");
    await writeFile(join(root, "owned.txt"), "base");
    await git("add", ".");
    await git("commit", "-m", "Owned base");
    const base = (await git("rev-parse", "HEAD")).stdout.trim();
    await git("checkout", "-b", "choruz/test-agent-abcd1234");
    await writeFile(join(root, "owned.txt"), "branch");
    await git("commit", "-am", "Owned branch change");
    const head = (await git("rev-parse", "HEAD")).stdout.trim();
    const shortHead = (await git("rev-parse", "--short", "HEAD")).stdout.trim();
    await git("checkout", "main");
    const created = await page.request.post(`${API_BASE}/v1/companies`, {
      headers: { Authorization: `Bearer ${token}` },
      data: { actor_id: principal.id, name: root.split("/").pop(), folder_path: root },
    });
    expect(created.ok(), await created.text()).toBeTruthy();
    companyId = (await created.json()).id;
    const params = new URLSearchParams({ workspace_id: companyId!, repo_path: root });
    const response = await page.request.get(`${WEB_BASE}/api/git-graph?${params}`);
    expect(response.status(), await response.text()).toBe(200);
    const graph = await response.json();
    expect(graph.mainHead).toBe(base);
    expect(graph.mainCommits).toEqual([expect.objectContaining({ hash: base, message: "Owned base" })]);
    expect(graph.branches).toEqual([expect.objectContaining({
      name: "choruz/test-agent-abcd1234", head: shortHead, aheadMain: 1, behindMain: 0,
      commits: [expect.objectContaining({ hash: head, message: "Owned branch change", filesChanged: 1 })],
    })]);
  } finally {
    try {
      if (companyId) await deleteCompany(page, token, companyId);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  }
});
