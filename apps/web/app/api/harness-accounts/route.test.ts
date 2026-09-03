import { NextRequest, NextResponse } from "next/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import { requireAuth } from "../../../lib/api/api-auth";
import { canAccessHarnessAccountCompany } from "../../../lib/agents/harness-account-access";
import { createHarnessAccount, listHarnessAccounts, runtimeHostBelongsToCompany } from "../../../lib/agents/harness-accounts";
import { GET, POST } from "./route";

vi.mock("../../../lib/api/api-auth", () => ({ requireAuth: vi.fn() }));
vi.mock("../../../lib/agents/harness-account-access", () => ({ canAccessHarnessAccountCompany: vi.fn() }));
vi.mock("../../../lib/agents/harness-accounts", () => ({
  createHarnessAccount: vi.fn(),
  listHarnessAccounts: vi.fn(),
  runtimeHostBelongsToCompany: vi.fn(),
}));

const auth = {
  token: "session-token",
  claims: {
    principal_id: "user-a",
    workspace_id: "company-a",
    display_name: "Alice",
    expires_at_epoch_s: 1,
  },
};

describe("/api/harness-accounts", () => {
  afterEach(() => {
    vi.clearAllMocks();
  });

  it("lists only accounts for an authorized company and device", async () => {
    vi.mocked(requireAuth).mockResolvedValue(auth);
    vi.mocked(canAccessHarnessAccountCompany).mockResolvedValue(true);
    vi.mocked(listHarnessAccounts).mockResolvedValue([]);

    const response = await GET(new NextRequest(
      "http://localhost/api/harness-accounts?company_id=company-a&runtime_host_id=host-a",
    ));

    expect(response.status).toBe(200);
    expect(listHarnessAccounts).toHaveBeenCalledWith("company-a", "host-a");
    await expect(response.json()).resolves.toEqual({ accounts: [] });
  });

  it("creates a device-local account without accepting credentials", async () => {
    vi.mocked(requireAuth).mockResolvedValue(auth);
    vi.mocked(canAccessHarnessAccountCompany).mockResolvedValue(true);
    vi.mocked(createHarnessAccount).mockResolvedValue({ id: "account-a" } as never);

    const response = await POST(new NextRequest("http://localhost/api/harness-accounts", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        company_id: "company-a",
        driver_type: "codex_terminal",
        profile_kind: "isolated",
        name: "Work",
        token: "must-not-be-forwarded",
      }),
    }));

    expect(response.status).toBe(201);
    expect(createHarnessAccount).toHaveBeenCalledWith({
      companyId: "company-a",
      runtimeHostId: null,
      driverType: "codex_terminal",
      profileKind: "isolated",
      name: "Work",
    });
  });

  it("creates a remote profile only for a host in the selected company", async () => {
    vi.mocked(requireAuth).mockResolvedValue(auth);
    vi.mocked(canAccessHarnessAccountCompany).mockResolvedValue(true);
    vi.mocked(runtimeHostBelongsToCompany).mockResolvedValue(true);
    vi.mocked(createHarnessAccount).mockResolvedValue({ id: "account-remote" } as never);

    const response = await POST(new NextRequest("http://localhost/api/harness-accounts", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        company_id: "company-a",
        runtime_host_id: "host-a",
        driver_type: "claude_terminal",
        profile_kind: "isolated",
        name: "Remote work",
      }),
    }));

    expect(response.status).toBe(201);
    expect(createHarnessAccount).toHaveBeenCalledWith({
      companyId: "company-a",
      runtimeHostId: "host-a",
      driverType: "claude_terminal",
      profileKind: "isolated",
      name: "Remote work",
    });
  });

  it("rejects a runtime host outside the selected company", async () => {
    vi.mocked(requireAuth).mockResolvedValue(auth);
    vi.mocked(canAccessHarnessAccountCompany).mockResolvedValue(true);
    vi.mocked(runtimeHostBelongsToCompany).mockResolvedValue(false);

    const response = await POST(new NextRequest("http://localhost/api/harness-accounts", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        company_id: "company-a", runtime_host_id: "foreign-host", driver_type: "codex_terminal", profile_kind: "default", name: "Other",
      }),
    }));

    expect(response.status).toBe(404);
    expect(createHarnessAccount).not.toHaveBeenCalled();
  });

  it("does not read account metadata for an unauthorized company", async () => {
    vi.mocked(requireAuth).mockResolvedValue(auth);
    vi.mocked(canAccessHarnessAccountCompany).mockResolvedValue(false);

    const response = await GET(new NextRequest(
      "http://localhost/api/harness-accounts?company_id=company-b",
    ));

    expect(response.status).toBe(403);
    expect(listHarnessAccounts).not.toHaveBeenCalled();
  });

  it("passes through authentication failures", async () => {
    vi.mocked(requireAuth).mockResolvedValue(
      NextResponse.json({ error: "Unauthorized" }, { status: 401 }),
    );

    const response = await GET(new NextRequest(
      "http://localhost/api/harness-accounts?company_id=company-a",
    ));

    expect(response.status).toBe(401);
    expect(canAccessHarnessAccountCompany).not.toHaveBeenCalled();
  });
});
