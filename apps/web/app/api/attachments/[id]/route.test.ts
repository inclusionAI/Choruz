import { NextRequest, NextResponse } from "next/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import { requireAuth } from "../../../../lib/api/api-auth";
import { GET } from "./route";

vi.mock("../../../../lib/api/api-auth", () => ({
  requireAuth: vi.fn(),
}));

describe("/api/attachments/[id]", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("proxies an authorised attachment with the authenticated Choruz session and private rendering headers", async () => {
    vi.mocked(requireAuth).mockResolvedValue({
      token: "synthetic-session-token",
      claims: {
        principal_id: "human-a",
        workspace_id: "company-a",
        display_name: "Human A",
        expires_at_epoch_s: 1,
      },
    });
    const fetchMock = vi.fn().mockResolvedValue(
      new Response("synthetic-png-bytes", {
        status: 200,
        headers: { "content-type": "image/png" },
      }),
    );
    vi.stubGlobal("fetch", fetchMock);

    const response = await GET(
      new NextRequest("http://localhost/api/attachments/attachment-a"),
      { params: Promise.resolve({ id: "attachment-a" }) },
    );

    expect(response.status).toBe(200);
    expect(response.headers.get("content-type")).toBe("image/png");
    expect(response.headers.get("cache-control")).toBe("private, no-store");
    expect(response.headers.get("vary")).toBe("Cookie");
    expect(await response.text()).toBe("synthetic-png-bytes");
    expect(fetchMock).toHaveBeenCalledWith(
      expect.stringMatching(/\/v1\/attachments\/attachment-a\?actor_id=human-a$/),
      { headers: { Cookie: "choruz_session=synthetic-session-token" } },
    );
  });

  it("does not proxy an attachment when the request has no valid Choruz session", async () => {
    const forbidden = NextResponse.json({ error: "Unauthorized" }, { status: 401 });
    vi.mocked(requireAuth).mockResolvedValue(forbidden as never);
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);

    const response = await GET(
      new NextRequest("http://localhost/api/attachments/attachment-a"),
      { params: Promise.resolve({ id: "attachment-a" }) },
    );

    expect(response.status).toBe(401);
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
