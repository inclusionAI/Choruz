import { mkdtemp } from "fs/promises";
import { tmpdir } from "os";
import * as path from "path";
import { NextRequest } from "next/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import { requireAuth } from "../../../lib/api/api-auth";
import { GET } from "./route";

vi.mock("../../../lib/api/api-auth", () => ({
  requireAuth: vi.fn(),
}));

describe("/api/git-graph workspace path guard", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("rejects repositories outside the requested workspace before invoking git", async () => {
    const root = await mkdtemp(path.join(tmpdir(), "echat-git-root-"));
    const outside = await mkdtemp(path.join(tmpdir(), "echat-git-outside-"));
    vi.mocked(requireAuth).mockResolvedValue({
      token: "session-token",
      claims: {
        principal_id: "user-a",
        workspace_id: "ws-a",
        display_name: "Alice",
        expires_at_epoch_s: 1,
      },
    });
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => [{ id: "ws-a", folder_path: root, deleted_at: null }],
    });
    vi.stubGlobal("fetch", fetchMock);

    const request = new NextRequest(
      `http://localhost/api/git-graph?workspace_id=ws-a&repo_path=${encodeURIComponent(outside)}`,
    );

    const response = await GET(request);
    const body = await response.json();

    expect(response.status).toBe(403);
    expect(body.error).toBe("Path is outside the requested workspace");
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });
});
