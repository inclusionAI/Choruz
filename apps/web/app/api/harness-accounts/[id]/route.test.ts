import { NextRequest, NextResponse } from "next/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import { requireAuth } from "../../../../lib/api/api-auth";
import { canAccessHarnessAccountCompany } from "../../../../lib/agents/harness-account-access";
import { disableHarnessAccount, getHarnessAccount } from "../../../../lib/agents/harness-accounts";
import { DELETE } from "./route";

vi.mock("../../../../lib/api/api-auth", () => ({ requireAuth: vi.fn() }));
vi.mock("../../../../lib/agents/harness-account-access", () => ({ canAccessHarnessAccountCompany: vi.fn() }));
vi.mock("../../../../lib/agents/harness-accounts", () => ({
  disableHarnessAccount: vi.fn(),
  getHarnessAccount: vi.fn(),
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

describe("DELETE /api/harness-accounts/[id]", () => {
  afterEach(() => vi.clearAllMocks());

  it("reports how many dependent Agent bindings were stopped", async () => {
    vi.mocked(requireAuth).mockResolvedValue(auth);
    vi.mocked(canAccessHarnessAccountCompany).mockResolvedValue(true);
    vi.mocked(getHarnessAccount).mockResolvedValue({ id: "account-a" } as never);
    vi.mocked(disableHarnessAccount).mockResolvedValue(2);

    const response = await DELETE(
      new NextRequest("http://localhost/api/harness-accounts/account-a?company_id=company-a", { method: "DELETE" }),
      { params: Promise.resolve({ id: "account-a" }) },
    );

    expect(response.status).toBe(200);
    await expect(response.json()).resolves.toEqual({ disabled_bindings: 2 });
    expect(disableHarnessAccount).toHaveBeenCalledWith("account-a", "company-a");
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
    expect(disableHarnessAccount).not.toHaveBeenCalled();
  });
});
