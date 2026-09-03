import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { GroupProvisioningIssueList } from "./create-group-modal";
import type { GroupProvisioningIssue } from "../../lib/groups/group-provisioning-contract";

describe("CreateGroupModal provisioning issue list", () => {
  it("renders warning issues with the warning banner class and errors with the error banner class", () => {
    const issues: GroupProvisioningIssue[] = [
      issue("warning", "optional_slot_skipped", "QA Tester will be skipped."),
      issue("error", "company_mismatch", "Existing agent belongs to a different company."),
    ];

    const html = renderToStaticMarkup(React.createElement(GroupProvisioningIssueList, { issues }));

    expect(html).toContain('class="modal-form-warning"');
    expect(html).toContain("QA Tester will be skipped.");
    expect(html).toContain('class="modal-form-error"');
    expect(html).toContain("Existing agent belongs to a different company.");
  });
});

function issue(
  severity: GroupProvisioningIssue["severity"],
  code: string,
  message: string,
): GroupProvisioningIssue {
  return {
    severity,
    code,
    message,
    recoverable: severity === "error",
  };
}
