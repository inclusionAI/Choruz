import { NextRequest, NextResponse } from "next/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import { requireAuth } from "../../../../lib/api/api-auth";
import { discoverDriverModels } from "../../../../lib/drivers/driver-models";
import { GET } from "./route";

vi.mock("../../../../lib/api/api-auth", () => ({ requireAuth: vi.fn() }));
vi.mock("../../../../lib/drivers/driver-models", async (importOriginal) => ({ ...await importOriginal<typeof import("../../../../lib/drivers/driver-models")>(), discoverDriverModels: vi.fn() }));

describe("/api/drivers/models", () => {
  afterEach(() => {
    vi.clearAllMocks();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("parses the target's CLI catalog without scanning controller models", async () => {
    vi.mocked(requireAuth).mockResolvedValue({ token: "session-token", claims: { principal_id: "user-a", workspace_id: "ws-a", display_name: "Alice", expires_at_epoch_s: 1 } });
    const request = vi.fn(async () => Response.json({ stdout: "fixture/device-b-model\n" }));
    vi.stubGlobal("fetch", request);
    const response = await GET(new NextRequest("http://localhost/api/drivers/models?driver_type=opencode_terminal&runtime_host_id=device-b"));
    expect(await response.json()).toMatchObject({ models: [{ id: "fixture/device-b-model" }], status: "available" });
    expect(discoverDriverModels).not.toHaveBeenCalled();
    expect(request).toHaveBeenCalledWith(expect.stringContaining("/runtime-hosts/device-b/operations"), expect.objectContaining({ body: JSON.stringify({ kind: "drivers.inspect", request: { driver_type: "opencode_terminal" } }) }));
  });

  it("does not replace an offline device with controller models", async () => {
    vi.mocked(requireAuth).mockResolvedValue({ token: "session-token", claims: { principal_id: "user-a", workspace_id: "ws-a", display_name: "Alice", expires_at_epoch_s: 1 } });
    vi.stubGlobal("fetch", vi.fn(async () => Response.json({ error: "device disconnected" }, { status: 409 })));
    const response = await GET(new NextRequest("http://localhost/api/drivers/models?driver_type=opencode_terminal&runtime_host_id=device-b"));
    expect(response.status).toBe(409);
    expect(discoverDriverModels).not.toHaveBeenCalled();
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
    const request = vi.fn(async () => Response.json({ models: [{ id: "sonnet", label: "Sonnet" }] }));
    vi.stubGlobal("fetch", request);

    const response = await GET(new NextRequest("http://localhost/api/drivers/models?driver_type=claude_terminal"));

    expect(response.status).toBe(200);
    await expect(response.json()).resolves.toMatchObject({
      status: "available",
      models: [{ id: "sonnet" }],
    });
    expect(discoverDriverModels).not.toHaveBeenCalled();
    expect(request).toHaveBeenCalledWith(expect.stringContaining("/v1/drivers/models?driver_type=claude_terminal"), expect.objectContaining({ headers: { Authorization: "Bearer session-token" } }));
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
