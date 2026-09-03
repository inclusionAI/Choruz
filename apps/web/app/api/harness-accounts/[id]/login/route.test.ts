import { NextRequest } from "next/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import { requireAuth } from "../../../../../lib/api/api-auth";
import { canAccessHarnessAccountCompany } from "../../../../../lib/agents/harness-account-access";
import { getHarnessAccount } from "../../../../../lib/agents/harness-accounts";
import { POST } from "./route";

vi.mock("../../../../../lib/api/api-auth", () => ({ requireAuth: vi.fn() }));
vi.mock("../../../../../lib/agents/harness-account-access", () => ({ canAccessHarnessAccountCompany: vi.fn() }));
vi.mock("../../../../../lib/agents/harness-accounts", () => ({ getHarnessAccount: vi.fn() }));
vi.mock("../../../../../lib/api/choruz-api", () => ({ apiBaseUrl: () => "http://gateway.test" }));

const auth = {
  token: "session-token",
  claims: {
    principal_id: "user-a",
    workspace_id: "company-a",
    display_name: "Alice",
    expires_at_epoch_s: 1,
  },
};

const request = () => new NextRequest("http://localhost/api/harness-accounts/account-a/login?company_id=company-a", { method: "POST" });
const params = { params: Promise.resolve({ id: "account-a" }) };

describe("/api/harness-accounts/[id]/login", () => {
  afterEach(() => {
    vi.clearAllMocks();
    vi.unstubAllGlobals();
  });

  it("starts a login on the account-scoped gateway route with the session bearer token", async () => {
    vi.mocked(requireAuth).mockResolvedValue(auth);
    vi.mocked(canAccessHarnessAccountCompany).mockResolvedValue(true);
    vi.mocked(getHarnessAccount).mockResolvedValue({ id: "account-a", runtimeHostId: null } as never);
    const login = { id: "login-1", account_id: "account-a", state: "authorizing" };
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify(login), { status: 201, headers: { "content-type": "application/json" } }));
    vi.stubGlobal("fetch", fetchMock);

    const response = await POST(request(), params);

    expect(response.status).toBe(201);
    await expect(response.json()).resolves.toEqual(login);
    expect(fetchMock).toHaveBeenCalledWith(
      "http://gateway.test/v1/companies/company-a/harness-accounts/account-a/logins",
      { method: "POST", headers: { authorization: "Bearer session-token" }, cache: "no-store" },
    );
  });

  it("returns the gateway's 409 unchanged when a login is already in progress", async () => {
    vi.mocked(requireAuth).mockResolvedValue(auth);
    vi.mocked(canAccessHarnessAccountCompany).mockResolvedValue(true);
    vi.mocked(getHarnessAccount).mockResolvedValue({ id: "account-a", runtimeHostId: "host-a" } as never);
    const conflict = { error: "A login is already in progress" };
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(JSON.stringify(conflict), { status: 409, headers: { "content-type": "application/json" } })));

    const response = await POST(request(), params);

    expect(response.status).toBe(409);
    await expect(response.json()).resolves.toEqual(conflict);
  });

  it("does not contact the gateway for an unauthorized company", async () => {
    vi.mocked(requireAuth).mockResolvedValue(auth);
    vi.mocked(canAccessHarnessAccountCompany).mockResolvedValue(false);
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);

    const response = await POST(request(), params);

    expect(response.status).toBe(403);
    expect(fetchMock).not.toHaveBeenCalled();
    expect(getHarnessAccount).not.toHaveBeenCalled();
  });
});
