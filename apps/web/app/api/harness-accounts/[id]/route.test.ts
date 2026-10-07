import { NextRequest, NextResponse } from "next/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import { requireAuth } from "../../../../lib/api/api-auth";
import { canAccessHarnessAccountCompany } from "../../../../lib/agents/harness-account-access";
import { getHarnessAccount } from "../../../../lib/agents/harness-accounts";
import { apiFetch } from "../../../../lib/api/choruz-api";
import { DELETE } from "./route";

vi.mock("../../../../lib/api/api-auth", () => ({ requireAuth: vi.fn() }));
vi.mock("../../../../lib/agents/harness-account-access", () => ({ canAccessHarnessAccountCompany: vi.fn() }));
vi.mock("../../../../lib/agents/harness-accounts", () => ({
  getHarnessAccount: vi.fn(),
}));
vi.mock("../../../../lib/api/choruz-api",async(importOriginal)=>({...await importOriginal<typeof import("../../../../lib/api/choruz-api")>(),apiFetch:vi.fn()}));

const auth = {
  token: "session-token",
  claims: {
    principal_id: "user-a",
    workspace_id: "company-a",
    display_name: "Alice",
    expires_at_epoch_s: 1,
  },
};

describe("DELETE /api/harness-accounts/[id]", () => {
  afterEach(() => vi.clearAllMocks());

  it("reports how many dependent Agent bindings were stopped", async () => {
    vi.mocked(requireAuth).mockResolvedValue(auth);
    vi.mocked(canAccessHarnessAccountCompany).mockResolvedValue(true);
    vi.mocked(getHarnessAccount).mockResolvedValue({ id: "account-a" } as never);
    vi.mocked(apiFetch).mockResolvedValue({ disabled_bindings:2,removal_pending:false });

    const response = await DELETE(
      new NextRequest("http://localhost/api/harness-accounts/account-a?company_id=company-a", { method: "DELETE" }),
      { params: Promise.resolve({ id: "account-a" }) },
    );

    expect(response.status).toBe(200);
    await expect(response.json()).resolves.toEqual({ disabled_bindings: 2,removal_pending:false });
    expect(apiFetch).toHaveBeenCalledWith("/v1/companies/company-a/harness-accounts/account-a","session-token",{method:"DELETE"});
  });

  it("does not inspect an account outside the authorized company", async () => {
    vi.mocked(requireAuth).mockResolvedValue(auth);
    vi.mocked(canAccessHarnessAccountCompany).mockResolvedValue(false);

    const response = await DELETE(
      new NextRequest("http://localhost/api/harness-accounts/account-a?company_id=company-b", { method: "DELETE" }),
      { params: Promise.resolve({ id: "account-a" }) },
    );

    expect(response).toBeInstanceOf(NextResponse);
    expect(response.status).toBe(403);
    expect(apiFetch).not.toHaveBeenCalled();
  });
  it("keeps unconfirmed device cleanup pending",async()=>{
    vi.mocked(requireAuth).mockResolvedValue(auth);
    vi.mocked(canAccessHarnessAccountCompany).mockResolvedValue(true);
    vi.mocked(getHarnessAccount).mockResolvedValue({id:"account-a"} as never);
    vi.mocked(apiFetch).mockResolvedValue({disabled_bindings:null,removal_pending:true});
    const response=await DELETE(new NextRequest("http://localhost/api/harness-accounts/account-a?company_id=company-a",{method:"DELETE"}),{params:Promise.resolve({id:"account-a"})});
    expect(response.status).toBe(202);
    expect((await response.json()).removal_pending).toBe(true);
  });
});
