import { describe, expect, it } from "vitest";

import {
  GROUP_PROVISIONING_JOB_STATUSES,
  GROUP_PROVISIONING_STATUS_TRANSITIONS,
} from "./group-provisioning-store";
import {
  GROUP_PROVISIONING_STATUS_CONTRACT,
  assertEveryStatusHasExplicitActions,
  isRecoveryChoiceRequest,
  validateGroupProvisioningContractFixture,
} from "./group-provisioning-contract";
import {
  groupProvisioningContractFixtureList,
  groupProvisioningContractFixtures,
} from "./group-provisioning-contract-fixtures";

describe("group provisioning contract fixtures", () => {
  it("validates every fixture with runtime guards and sensitive-key checks", () => {
    for (const fixture of groupProvisioningContractFixtureList) {
      expect(validateGroupProvisioningContractFixture(fixture)).toBe(fixture);
    }
  });

  it("covers all required contract scenarios", () => {
    expect(Object.keys(groupProvisioningContractFixtures).sort()).toEqual([
      "cancellationAfterSideEffects",
      "cancellationBeforeSideEffects",
      "groupCreationFailure",
      "happyPath",
      "kickoffWarning",
      "memberAddPartialFailure",
      "optionalAgentFailure",
      "requiredAgentFailure",
      "validationFailure",
    ]);
  });

  it("defines explicit UI and backend actions for every canonical status", () => {
    assertEveryStatusHasExplicitActions();

    for (const status of GROUP_PROVISIONING_JOB_STATUSES) {
      const contract = GROUP_PROVISIONING_STATUS_CONTRACT[status];
      expect(contract.status).toBe(status);
      expect(contract.nextStatuses).toEqual(GROUP_PROVISIONING_STATUS_TRANSITIONS[status]);
      expect(contract.uiActions.length).toBeGreaterThan(0);
      expect(contract.backendActions.length).toBeGreaterThan(0);
    }
  });

  it("keeps kickoff messages passive and free of agent mentions", () => {
    for (const fixture of groupProvisioningContractFixtureList) {
      expect(fixture.plan.startWorkMode).toBe("manual");
      expect(fixture.plan.kickoffText).toContain("Roles:");
      expect(fixture.plan.kickoffText).toContain("Next user action:");
      expect(fixture.plan.kickoffText).not.toMatch(/@[a-z0-9][\w.-]*/i);
      expect(fixture.plan.kickoffText).not.toMatch(/\b(Frontend Engineer|QA Tester|DevOps Engineer)\b/);
      expect(fixture.plan.kickoffText).not.toMatch(/\b(agent_commands|app_mention|auto[- ]?start|start work automatically)\b/i);

      for (const result of fixture.stepResults) {
        if (result.kind !== "kickoff_post") continue;
        expect(result.kickoffText).toContain("Current members:");
        expect(result.kickoffText).not.toMatch(/\b(Frontend Engineer|QA Tester|DevOps Engineer)\b/);
        expect(result.kickoffText).not.toMatch(/@[a-z0-9][\w.-]*/i);
        expect(result.kickoffText).not.toMatch(/\b(agent_commands|app_mention|auto[- ]?start|start work automatically)\b/i);
      }
    }
  });

  it("anchors validation issues to fields and role slots for editor linking", () => {
    const validation = groupProvisioningContractFixtures.validationFailure;

    expect(validation.issues.some((issue) => issue.field === "mission")).toBe(true);
    expect(validation.issues.some((issue) => issue.roleSlotId === "backend-engineer")).toBe(true);
    expect(validation.issues.every((issue) => issue.recoverable)).toBe(true);
  });

  it("keeps recovery choice next statuses aligned with canonical transitions", () => {
    for (const fixture of groupProvisioningContractFixtureList) {
      for (const choice of fixture.recoveryChoices) {
        if (!choice.nextStatus) continue;
        expect(GROUP_PROVISIONING_STATUS_TRANSITIONS[fixture.status]).toContain(choice.nextStatus);
      }
    }
  });

  it("defines payloads for non-retry recovery choices", () => {
    expect(isRecoveryChoiceRequest({
      choice: "skip_optional_role",
      roleSlotId: "qa-tester",
    })).toBe(true);
    expect(isRecoveryChoiceRequest({
      choice: "replace_agent",
      roleSlotId: "backend-engineer",
      replacementAgentId: "agent-replacement",
    })).toBe(true);
    expect(isRecoveryChoiceRequest({
      choice: "manual_invite",
    })).toBe(true);
    expect(isRecoveryChoiceRequest({
      choice: "soft_delete_generated_agents",
      generatedAgentIds: ["agent-project-operator"],
      preserveReusedAgents: true,
    })).toBe(true);
    expect(isRecoveryChoiceRequest({ choice: "skip_optional_role" })).toBe(false);
    expect(isRecoveryChoiceRequest({ choice: "soft_delete_generated_agents", generatedAgentIds: ["agent-a"] })).toBe(false);
  });

  it("rejects enum drift and sensitive keys anywhere in contract fixtures", () => {
    const invalidDriver = structuredClone(groupProvisioningContractFixtures.happyPath);
    const createPlan = invalidDriver.plan.rolePlans.find((plan) => plan.action === "create");
    if (!createPlan || createPlan.action !== "create") throw new Error("fixture missing create role plan");
    createPlan.driver = "made_up_driver" as never;
    expect(() => validateGroupProvisioningContractFixture(invalidDriver)).toThrow(/invalid group provisioning job contract fixture/);

    const webhookSecretPlan = structuredClone(groupProvisioningContractFixtures.happyPath);
    const secretCreatePlan = webhookSecretPlan.plan.rolePlans.find((plan) => plan.action === "create");
    if (!secretCreatePlan || secretCreatePlan.action !== "create") throw new Error("fixture missing create role plan");
    secretCreatePlan.driver = "webhook_agent";
    secretCreatePlan.webhookUrl = "https://example.com/hook";
    (secretCreatePlan as typeof secretCreatePlan & { webhookSecret: string }).webhookSecret = "do-not-store";
    expect(() => validateGroupProvisioningContractFixture(webhookSecretPlan)).toThrow(/invalid group provisioning job contract fixture/);

    const webhookModelPlan = structuredClone(groupProvisioningContractFixtures.happyPath);
    const modeledCreatePlan = webhookModelPlan.plan.rolePlans.find((plan) => plan.action === "create");
    if (!modeledCreatePlan || modeledCreatePlan.action !== "create") throw new Error("fixture missing create role plan");
    modeledCreatePlan.driver = "webhook_agent";
    modeledCreatePlan.model = "external-model";
    expect(() => validateGroupProvisioningContractFixture(webhookModelPlan)).toThrow(/invalid group provisioning job contract fixture/);

    const invalidWebhookConfigured = structuredClone(groupProvisioningContractFixtures.happyPath);
    const webhookConfiguredPlan = invalidWebhookConfigured.plan.rolePlans.find((plan) => plan.action === "create");
    if (!webhookConfiguredPlan || webhookConfiguredPlan.action !== "create") throw new Error("fixture missing create role plan");
    webhookConfiguredPlan.webhookConfigured = "yes" as never;
    expect(() => validateGroupProvisioningContractFixture(invalidWebhookConfigured)).toThrow(/invalid group provisioning job contract fixture/);

    const invalidStepResult = structuredClone(groupProvisioningContractFixtures.happyPath);
    const memberResult = invalidStepResult.stepResults.find((result) => result.kind === "member_add");
    if (!memberResult || memberResult.kind !== "member_add") throw new Error("fixture missing member add result");
    memberResult.result = "sort_of_added" as never;
    expect(() => validateGroupProvisioningContractFixture(invalidStepResult)).toThrow(/invalid group provisioning job contract fixture/);

    const secretInIssue = structuredClone(groupProvisioningContractFixtures.validationFailure);
    secretInIssue.issues[0] = {
      ...secretInIssue.issues[0],
      apiToken: "nope",
    } as never;
    expect(() => validateGroupProvisioningContractFixture(secretInIssue)).toThrow(/sensitive provisioning key/);
  });

  it("represents required result kinds for provisioning progress", () => {
    const kinds = new Set(groupProvisioningContractFixtureList.flatMap((fixture) => fixture.stepResults.map((result) => result.kind)));

    expect(kinds).toEqual(new Set([
      "cleanup",
      "created_agent",
      "created_group",
      "kickoff_post",
      "member_add",
      "residual_assets",
      "reused_agent",
      "routing_policy_update",
      "skipped_optional_role",
    ]));
  });

  it("separates cancellation before side effects from soft-delete cleanup after side effects", () => {
    const before = groupProvisioningContractFixtures.cancellationBeforeSideEffects;
    const after = groupProvisioningContractFixtures.cancellationAfterSideEffects;

    expect(before.createdAgentIds).toEqual([]);
    expect(before.createdGroupId).toBeUndefined();
    expect(before.stepResults).toContainEqual({
      kind: "cleanup",
      stepId: "cleanup",
      result: "none_needed",
      softDeletedAgentIds: [],
      preservedAgentIds: [],
    });

    const cleanup = after.stepResults.find((result) => result.kind === "cleanup");
    expect(cleanup).toMatchObject({
      result: "soft_deleted_generated_agents",
      softDeletedAgentIds: ["agent-project-operator", "agent-backend-engineer"],
      preservedAgentIds: ["agent-reviewer-existing"],
    });
    expect(after.stepResults.some((result) => result.kind === "residual_assets")).toBe(true);
  });
});
