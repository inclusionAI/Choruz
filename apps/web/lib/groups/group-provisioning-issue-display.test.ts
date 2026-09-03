import { describe, expect, it } from "vitest";

import { groupProvisioningIssueClassName } from "./group-provisioning-issue-display";

describe("group provisioning issue display", () => {
  it("renders warnings with the warning banner class", () => {
    expect(groupProvisioningIssueClassName({ severity: "warning" })).toBe("modal-form-warning");
  });

  it("renders errors with the error banner class", () => {
    expect(groupProvisioningIssueClassName({ severity: "error" })).toBe("modal-form-error");
  });
});
