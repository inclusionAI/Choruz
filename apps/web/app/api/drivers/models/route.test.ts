import { NextRequest, NextResponse } from "next/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import { requireAuth } from "../../../../lib/api/api-auth";
import { discoverDriverModels } from "../../../../lib/drivers/driver-models";
import { GET } from "./route";

vi.mock("../../../../lib/api/api-auth", () => ({ requireAuth: vi.fn() }));
vi.mock("../../../../lib/drivers/driver-models", () => ({ discoverDriverModels: vi.fn() }));

describe("/api/drivers/models", () => {
  afterEach(() => {
    vi.clearAllMocks();
    vi.restoreAllMocks();
  });

  it("returns account-specific models for a supported driver", async () => {
    vi.mocked(requireAuth).mockResolvedValue({
      token: "session-token",
      claims: {
        principal_id: "user-a",
        workspace_id: "ws-a",
        display_name: "Alice",
        expires_at_epoch_s: 1,
      },
    });
    vi.mocked(discoverDriverModels).mockResolvedValue({
      driverId: "claude_terminal",
      status: "available",
      models: [{ id: "sonnet", label: "Sonnet" }],
      message: "1 models discovered from the installed harness.",
    });

    const response = await GET(new NextRequest("http://localhost/api/drivers/models?driver_type=claude_terminal"));

    expect(response.status).toBe(200);
    await expect(response.json()).resolves.toMatchObject({
      status: "available",
      models: [{ id: "sonnet" }],
    });
    expect(discoverDriverModels).toHaveBeenCalledWith("claude_terminal");
  });

  it("rejects unknown drivers before spawning a harness", async () => {
    vi.mocked(requireAuth).mockResolvedValue({
      token: "session-token",
      claims: {
        principal_id: "user-a",
        workspace_id: "ws-a",
        display_name: "Alice",
        expires_at_epoch_s: 1,
      },
    });

    const response = await GET(new NextRequest("http://localhost/api/drivers/models?driver_type=made_up"));

    expect(response.status).toBe(400);
    expect(discoverDriverModels).not.toHaveBeenCalled();
  });

  it("passes through authentication failures", async () => {
    vi.mocked(requireAuth).mockResolvedValue(
      NextResponse.json({ error: "Unauthorized" }, { status: 401 }),
    );

    const response = await GET(new NextRequest("http://localhost/api/drivers/models?driver_type=claude_terminal"));

    expect(response.status).toBe(401);
    expect(discoverDriverModels).not.toHaveBeenCalled();
  });
});
