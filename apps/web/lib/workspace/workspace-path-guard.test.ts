import { mkdtemp, mkdir, symlink, writeFile } from "fs/promises";
import { tmpdir } from "os";
import * as path from "path";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  isPathInsideRoot,
  requirePathInsideWorkspace,
  workspaceRoots,
} from "./workspace-path-guard";

describe("workspace path guard", () => {
  afterEach(() => {
    vi.restoreAllMocks();
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
    const root = await mkdtemp(path.join(tmpdir(), "echat-ws-root-"));
    const outside = await mkdtemp(path.join(tmpdir(), "echat-ws-outside-"));
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
    const root = await mkdtemp(path.join(tmpdir(), "echat-ws-allowed-"));
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
