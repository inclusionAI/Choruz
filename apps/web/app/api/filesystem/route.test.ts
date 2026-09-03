import { mkdtemp } from "fs/promises";
import { tmpdir } from "os";
import * as path from "path";
import { NextRequest } from "next/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import { requireAuth } from "../../../lib/api/api-auth";
import { GET, POST } from "./route";

vi.mock("../../../lib/api/api-auth", () => ({
  requireAuth: vi.fn(),
}));

describe("/api/filesystem workspace path guard", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("rejects workspace-scoped reads outside the company folder before proxying", async () => {
    const root = await mkdtemp(path.join(tmpdir(), "echat-fs-root-"));
    const outside = await mkdtemp(path.join(tmpdir(), "echat-fs-outside-"));
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
      `http://localhost/api/filesystem?action=read&workspace_id=ws-a&path=${encodeURIComponent(outside)}`,
    );

    const response = await GET(request);
    const body = await response.json();

    expect(response.status).toBe(403);
    expect(body.error).toBe("Path is outside the requested workspace");
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("requires workspace_id for file reads", async () => {
    vi.mocked(requireAuth).mockResolvedValue({
      token: "session-token",
      claims: {
        principal_id: "user-a",
        workspace_id: "ws-a",
        display_name: "Alice",
        expires_at_epoch_s: 1,
      },
    });
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);

    const request = new NextRequest(
      "http://localhost/api/filesystem?action=read&path=/tmp/secret.txt",
    );

    const response = await GET(request);
    const body = await response.json();

    expect(response.status).toBe(400);
    expect(body.error).toBe("workspace_id is required");
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("rejects workspace-scoped writes outside the company folder before proxying", async () => {
    const root = await mkdtemp(path.join(tmpdir(), "echat-fs-root-"));
    const outside = await mkdtemp(path.join(tmpdir(), "echat-fs-outside-"));
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

    const request = new NextRequest("http://localhost/api/filesystem", {
      method: "POST",
      body: JSON.stringify({
        path: path.join(outside, "leak.txt"),
        workspace_id: "ws-a",
        content: "nope",
      }),
      headers: { "content-type": "application/json" },
    });

    const response = await POST(request);
    const body = await response.json();

    expect(response.status).toBe(403);
    expect(body.error).toBe("Path is outside the requested workspace");
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("requires workspace_id for file writes", async () => {
    vi.mocked(requireAuth).mockResolvedValue({
      token: "session-token",
      claims: {
        principal_id: "user-a",
        workspace_id: "ws-a",
        display_name: "Alice",
        expires_at_epoch_s: 1,
      },
    });
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);

    const request = new NextRequest("http://localhost/api/filesystem", {
      method: "POST",
      body: JSON.stringify({
        path: "/tmp/secret.txt",
        content: "nope",
      }),
      headers: { "content-type": "application/json" },
    });

    const response = await POST(request);
    const body = await response.json();

    expect(response.status).toBe(400);
    expect(body.error).toBe("workspace_id is required");
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
