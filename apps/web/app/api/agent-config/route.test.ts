import { mkdtemp, readFile, rm, symlink, writeFile } from "node:fs/promises";
import * as path from "node:path";
import { NextRequest } from "next/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { requireAuth } from "../../../lib/api/api-auth";
import { GET, POST } from "./route";

vi.mock("../../../lib/api/api-auth", () => ({
  requireAuth: vi.fn(),
}));

describe("/api/agent-config", () => {
  const cleanupPaths: string[] = [];

  beforeEach(() => {
    vi.mocked(requireAuth).mockResolvedValue({
      token: "session-token",
      claims: {
        principal_id: "human-1",
        workspace_id: "workspace-1",
        display_name: "Alice",
        expires_at_epoch_s: 1,
      },
    });
  });

  afterEach(async () => {
    vi.restoreAllMocks();
    await Promise.all(cleanupPaths.splice(0).map((entry) => rm(entry, { recursive: true, force: true })));
  });

  async function workspace(): Promise<string> {
    const result = await mkdtemp(path.join("/tmp", "choruz-agent-config-"));
    cleanupPaths.push(result);
    return result;
  }

  it("does not move an alternate instruction file into an imported session's workspace", async () => {
    const root = await workspace();
    await writeFile(path.join(root, "CLAUDE.md"), "preserved instructions", "utf-8");

    const response = await GET(
      new NextRequest(
        `http://localhost/api/agent-config?workspace_path=${encodeURIComponent(root)}&driver_type=pi_terminal`,
      ),
    );

    expect(response.status).toBe(200);
    expect(await response.json()).toMatchObject({
      filename: "AGENTS.md",
      content: "",
      exists: false,
      format: "raw",
    });
    await expect(readFile(path.join(root, "CLAUDE.md"), "utf-8")).resolves.toBe("preserved instructions");
    await expect(readFile(path.join(root, "AGENTS.md"), "utf-8")).rejects.toMatchObject({ code: "ENOENT" });
  });

  it("falls back to raw CLAUDE.md for imported Claude headless sessions", async () => {
    const root = await workspace();
    await writeFile(path.join(root, "CLAUDE.md"), "# Existing project instructions\n\nKeep changes small.", "utf-8");

    const response = await GET(
      new NextRequest(
        `http://localhost/api/agent-config?workspace_path=${encodeURIComponent(root)}&driver_type=claude_print`,
      ),
    );

    expect(response.status).toBe(200);
    expect(await response.json()).toMatchObject({
      filename: "CLAUDE.md",
      format: "raw",
      content: "# Existing project instructions\n\nKeep changes small.",
    });
  });

  it("keeps a native AGENTS.md as raw content for non-Claude harnesses", async () => {
    const root = await workspace();
    await writeFile(path.join(root, "AGENTS.md"), "# Native instructions\n\nUse the existing workflow.", "utf-8");

    const response = await GET(
      new NextRequest(
        `http://localhost/api/agent-config?workspace_path=${encodeURIComponent(root)}&driver_type=codex_exec`,
      ),
    );

    expect(response.status).toBe(200);
    expect(await response.json()).toMatchObject({
      filename: "AGENTS.md",
      format: "raw",
      content: "# Native instructions\n\nUse the existing workflow.",
    });
  });

  it("returns the selected native filename to every concurrent reader", async () => {
    const root = await workspace();
    await writeFile(path.join(root, "CLAUDE.md"), "shared instructions", "utf-8");
    const url = `http://localhost/api/agent-config?workspace_path=${encodeURIComponent(root)}&driver_type=grok_terminal`;

    const responses = await Promise.all(
      Array.from({ length: 12 }, () => GET(new NextRequest(url))),
    );
    const bodies = await Promise.all(responses.map((response) => response.json()));

    expect(responses.every((response) => response.status === 200)).toBe(true);
    expect(bodies.every((body) => body.content === "" && body.exists === false)).toBe(true);
  });

  it("rejects instruction-file symlinks for reads and writes", async () => {
    const root = await workspace();
    const target = path.join(root, "outside.md");
    await writeFile(target, "secret", "utf-8");
    await symlink(target, path.join(root, "AGENTS.md"));

    const getResponse = await GET(
      new NextRequest(
        `http://localhost/api/agent-config?workspace_path=${encodeURIComponent(root)}&driver_type=opencode_terminal`,
      ),
    );
    expect(getResponse.status).toBe(500);

    const postResponse = await POST(
      new NextRequest("http://localhost/api/agent-config", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          workspace_path: root,
          driver_type: "opencode_terminal",
          content: "overwrite",
        }),
      }),
    );
    expect(postResponse.status).toBe(500);
    await expect(readFile(target, "utf-8")).resolves.toBe("secret");
  });
});
