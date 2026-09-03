import { NextRequest, NextResponse } from "next/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import { requireAuth } from "../../../../lib/api/api-auth";
import { getDriverAvailability } from "../../../../lib/drivers/driver-availability";
import { GET } from "./route";

vi.mock("../../../../lib/api/api-auth", () => ({
  requireAuth: vi.fn(),
}));

vi.mock("../../../../lib/drivers/driver-availability", () => ({
  getDriverAvailability: vi.fn(),
}));

describe("/api/drivers/availability", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("requires auth before returning driver availability", async () => {
    vi.mocked(requireAuth).mockResolvedValue({
      token: "session-token",
      claims: {
        principal_id: "user-a",
        workspace_id: "ws-a",
        display_name: "Alice",
        expires_at_epoch_s: 1,
      },
    });
    vi.mocked(getDriverAvailability).mockResolvedValue([
      {
        label: "Codex Terminal",
          driverId: "codex_terminal",
          available: true,
          status: "available",
          reason: "Codex CLI is available.",
          setupHint: "Install Codex.",
          envVar: "CHORUZ_CODEX_BINARY",
        },
      ]);

    const response = await GET(new NextRequest("http://localhost/api/drivers/availability"));
    expect(response.status).toBe(200);
    await expect(response.json()).resolves.toEqual({
      drivers: [
        {
          label: "Codex Terminal",
          driverId: "codex_terminal",
          status: "available",
          reason: "Codex CLI is available.",
          setupHint: "Install Codex.",
          envVar: "CHORUZ_CODEX_BINARY",
        },
      ],
    });
    expect(getDriverAvailability).toHaveBeenCalledTimes(1);
  });

  it("passes through unauthorized auth responses without checking drivers", async () => {
    vi.mocked(requireAuth).mockResolvedValue(
      NextResponse.json({ error: "Unauthorized" }, { status: 401 }),
    );

    const response = await GET(new NextRequest("http://localhost/api/drivers/availability"));

    expect(response.status).toBe(401);
    await expect(response.json()).resolves.toEqual({ error: "Unauthorized" });
    expect(getDriverAvailability).not.toHaveBeenCalled();
  });

  it("redacts server binary paths from the public response shape", async () => {
    vi.mocked(requireAuth).mockResolvedValue({
      token: "session-token",
      claims: {
        principal_id: "user-a",
        workspace_id: "ws-a",
        display_name: "Alice",
        expires_at_epoch_s: 1,
      },
    });
    vi.mocked(getDriverAvailability).mockResolvedValue([
      {
        label: "Claude",
        driverId: "claude_terminal",
        available: true,
        status: "available",
        reason: "Claude CLI is available.",
        setupHint: "Install Claude.",
        envVar: "CHORUZ_CLAUDE_BINARY",
        binaryPath: "/Users/alice/bin/claude",
      } as never,
    ]);

    const response = await GET(new NextRequest("http://localhost/api/drivers/availability"));
    const body = await response.json();

    expect(response.status).toBe(200);
    expect(body).toEqual({
      drivers: [
        {
          label: "Claude",
          driverId: "claude_terminal",
          status: "available",
          reason: "Claude CLI is available.",
          setupHint: "Install Claude.",
          envVar: "CHORUZ_CLAUDE_BINARY",
        },
      ],
    });
    expect(JSON.stringify(body)).not.toContain("/Users/alice/bin/claude");
    expect(JSON.stringify(body)).not.toContain("binaryPath");
  });
});
