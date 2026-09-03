import type { GroupProvisioningIssue } from "./group-provisioning-contract";

export function groupProvisioningIssueClassName(issue: Pick<GroupProvisioningIssue, "severity">): string {
  return issue.severity === "warning" ? "modal-form-warning" : "modal-form-error";
}
