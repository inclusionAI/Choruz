import { afterEach, describe, expect, it, vi } from "vitest";

import { fetchCompany } from "../api/choruz-api";
import { hasGroupProvisioningCompanyAccess } from "./group-provisioning-company-access";

vi.mock("../api/choruz-api", () => ({
  fetchCompany: vi.fn(),
}));

describe("group provisioning company access", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("allows access when the company can be fetched", async () => {
    vi.mocked(fetchCompany).mockResolvedValue({ id: "company-1" } as never);

    await expect(hasGroupProvisioningCompanyAccess("session-token", "company-1")).resolves.toBe(true);
    expect(fetchCompany).toHaveBeenCalledWith("session-token", "company-1");
  });

  it("denies access when the company fetch fails", async () => {
    vi.mocked(fetchCompany).mockRejectedValue(new Error("not found"));

    await expect(hasGroupProvisioningCompanyAccess("session-token", "company-2")).resolves.toBe(false);
  });

  it("denies blank company ids without calling the gateway", async () => {
    await expect(hasGroupProvisioningCompanyAccess("session-token", "   ")).resolves.toBe(false);
    expect(fetchCompany).not.toHaveBeenCalled();
  });
});
