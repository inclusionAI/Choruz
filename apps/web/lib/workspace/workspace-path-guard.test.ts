import { mkdtemp, mkdir, rm, symlink, writeFile } from "fs/promises";
import { tmpdir } from "os";
import * as path from "path";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  isPathInsideRoot,
  requirePathInsideWorkspace,
  workspaceRoots,
} from "./workspace-path-guard";

describe("workspace path guard", () => {
  const roots: string[] = [];
  async function tempRoot(prefix: string) {
    const root = await mkdtemp(path.join(tmpdir(), prefix));
    roots.push(root);
    return root;
  }
  afterEach(async () => {
    vi.restoreAllMocks();
    await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })));
  });

  it("matches paths by path component, not string prefix", () => {
    expect(isPathInsideRoot("/work/acme/file.ts", "/work/acme")).toBe(true);
    expect(isPathInsideRoot("/work/acme2/file.ts", "/work/acme")).toBe(false);
  });

  it("derives roots only from the requested workspace", () => {
    expect(
      workspaceRoots(
        [
          { id: "ws-a", folder_path: "/work/a", deleted_at: null },
          { id: "ws-b", folder_path: "/work/b", deleted_at: null },
          { id: "ws-a", folder_path: null, deleted_at: null },
          { id: "ws-a", folder_path: "/work/deleted", deleted_at: "2026-05-15T00:00:00Z" },
        ],
        "ws-a",
      ),
    ).toEqual(["/work/a"]);
  });

  it("rejects a real path that escapes the workspace through a symlink", async () => {
    const root = await tempRoot("echat-ws-root-");
    const outside = await tempRoot("echat-ws-outside-");
    await writeFile(path.join(outside, "secret.txt"), "nope");
    await symlink(outside, path.join(root, "link"));

    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => [{ id: "ws-a", folder_path: root, deleted_at: null }],
    });

    const decision = await requirePathInsideWorkspace(
      "session-token",
      "ws-a",
      path.join(root, "link", "secret.txt"),
      fetchMock,
    );

    expect(decision).toEqual({
      ok: false,
      status: 403,
      error: "Path is outside the requested workspace",
    });
  });

  it("allows files inside the requested workspace root", async () => {
    const root = await tempRoot("echat-ws-allowed-");
    await mkdir(path.join(root, "src"));
    const file = path.join(root, "src", "index.ts");
    await writeFile(file, "export {};\n");

    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => [{ id: "ws-a", folder_path: root, deleted_at: null }],
    });

    await expect(
      requirePathInsideWorkspace("session-token", "ws-a", file, fetchMock),
    ).resolves.toEqual({ ok: true });
  });
});
