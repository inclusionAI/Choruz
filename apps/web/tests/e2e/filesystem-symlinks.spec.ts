import { mkdir, mkdtemp, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { expect, test } from "@playwright/test";
import { API_BASE, login } from "../fixtures/auth";

test("directory listing follows allowed symlinks and refuses targets outside browse roots", async ({ page }) => {
  const root = await mkdtemp(join(homedir(), ".choruz-symlink-test-"));
  const outside = await mkdtemp(join(tmpdir(), "choruz-symlink-outside-"));
  try {
    const canonicalRoot = await realpath(root);
    const target = join(canonicalRoot, "target");
    const alias = join(canonicalRoot, "alias");
    const forbidden = join(canonicalRoot, "forbidden");
    await mkdir(target);
    await writeFile(join(target, "owned.txt"), "owned");
    await writeFile(join(canonicalRoot, "plain.txt"), "plain");
    await symlink(target, alias, "dir");
    await symlink(outside, forbidden, "dir");
    await symlink(join(root, "missing"), join(root, "broken"), "dir");
    const { token } = await login(page);
    const headers = { Authorization: `Bearer ${token}` };
    const list = (path: string, includeFiles: boolean) => page.request.get(
      `${API_BASE}/v1/filesystem/list?path=${encodeURIComponent(path)}&include_files=${includeFiles}`,
      { headers },
    );
    for (const includeFiles of [false, true]) {
      const response = await list(canonicalRoot, includeFiles);
      expect(response.ok()).toBeTruthy();
      const listing = await response.json() as { entries: Array<{ name: string; type: string; path: string }> };
      expect(listing.entries).toContainEqual({ name: "alias", type: "directory", path: alias });
      expect(listing.entries.some((entry) => entry.name === "forbidden" || entry.name === "broken")).toBe(false);
      expect(listing.entries.some((entry) => entry.name === "plain.txt")).toBe(includeFiles);
    }
    const navigated = await list(alias, true);
    expect(navigated.ok()).toBeTruthy();
    expect(await navigated.json()).toMatchObject({
      path: target,
      entries: [{ name: "owned.txt", type: "file", path: join(target, "owned.txt") }],
    });
    expect((await list(forbidden, false)).status()).toBe(403);
  } finally {
    await rm(root, { recursive: true, force: true });
    await rm(outside, { recursive: true, force: true });
  }
});
