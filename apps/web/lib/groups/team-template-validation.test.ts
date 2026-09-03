import { describe, expect, it } from "vitest";

import type { DriverAvailabilityItem } from "../drivers/driver-availability";
import type { Principal, RuntimeBindingInfo } from "../api/choruz-types";
import { getGroupTemplate, getRoleTemplate, type DriverId } from "./team-templates";
import {
  type ExistingAgentCandidate,
  resolveDriverForRole,
  validateGroupLaunchPlan,
  validateRoleAgentCreation,
} from "./team-template-validation";

const availability = (availableDrivers: DriverId[]): DriverAvailabilityItem[] =>
  ([
    "claude_terminal",
    "codex_terminal",
    "codex_exec",
    "pi_terminal",
    "grok_terminal",
    "opencode_terminal",
    "webhook_agent",
  ] as DriverId[]).map((driverId) => ({
    label: driverId,
    driverId,
    available: availableDrivers.includes(driverId),
    status: availableDrivers.includes(driverId) ? "available" : "unavailable",
    reason: availableDrivers.includes(driverId) ? `${driverId} ok` : `${driverId} missing`,
    setupHint: `set up ${driverId}`,
  }));

const principal = (overrides: Partial<Principal> = {}): Principal => ({
  id: "agent-a",
  workspace_id: "ws-a",
  principal_type: "agent",
  name: "agent-a",
  avatar_url: null,
  scopes: [],
  disabled: false,
  created_at: "2026-05-19T00:00:00Z",
  updated_at: "2026-05-19T00:00:00Z",
  ...overrides,
});

const binding = (overrides: Partial<RuntimeBindingInfo> = {}): RuntimeBindingInfo => ({
  id: "binding-a",
  workspace_id: "ws-a",
  conversation_id: "conv-a",
  conversation_type: "direct",
  agent_principal_id: "agent-a",
  driver_type: "codex_terminal",
  workspace_path: "/Users/alice/workspace",
  state: "idle",
  ...overrides,
});

describe("onboarding template validation", () => {
  it("resolves driver precedence from system default through user override", () => {
    const roleTemplate = getRoleTemplate("backend-engineer");
    expect(roleTemplate).toBeTruthy();

    const systemOnly = resolveDriverForRole({
      roleTemplate: roleTemplate!,
      systemDefaultDriver: "claude_terminal",
    });
    expect(systemOnly.driver).toBe("codex_terminal");
    expect(systemOnly.source).toBe("role_template_recommendation");

    const userOverride = resolveDriverForRole({
      roleTemplate: roleTemplate!,
      systemDefaultDriver: "claude_terminal",
      groupDefaultDriver: "codex_exec",
      userOverrideDriver: "claude_terminal",
    });
    expect(userOverride.driver).toBe("claude_terminal");
    expect(userOverride.source).toBe("user_override");
  });

  it("warns but allows incompatible user driver overrides", () => {
    const result = validateRoleAgentCreation({
      roleTemplateId: "source-checker",
      agentName: "source-checker",
      userOverrideDriver: "codex_exec",
      setupInputs: { research_question: "What changed?" },
      instructions: "Check sources.",
      driverAvailability: availability(["codex_exec"]),
    });

    expect(result.valid).toBe(true);
    expect(result.errors.map((error) => error.code)).not.toContain("incompatible_driver");
    expect(result.warnings.map((warning) => warning.code)).toContain("incompatible_driver");
  });

  it("still blocks unavailable drivers even when the driver is only advisory-incompatible", () => {
    const result = validateRoleAgentCreation({
      roleTemplateId: "source-checker",
      agentName: "source-checker",
      userOverrideDriver: "codex_exec",
      setupInputs: { research_question: "What changed?" },
      instructions: "Check sources.",
      driverAvailability: availability([]),
    });

    expect(result.valid).toBe(false);
    expect(result.errors.map((error) => error.code)).toContain("driver_unavailable");
    expect(result.warnings.map((warning) => warning.code)).toContain("incompatible_driver");
  });

  it("suggests the next compatible available driver when the selected driver is unavailable", () => {
    const roleTemplate = getRoleTemplate("frontend-engineer");
    expect(roleTemplate).toBeTruthy();

    const resolution = resolveDriverForRole({
      roleTemplate: roleTemplate!,
      availability: availability(["claude_terminal"]),
    });

    expect(resolution.driver).toBe("codex_terminal");
    expect(resolution.suggestedDriver).toBe("claude_terminal");
  });

  it("blocks required roles when selected drivers are unavailable", () => {
    const group = getGroupTemplate("research-team");
    expect(group).toBeTruthy();

    const result = validateGroupLaunchPlan({
      groupTemplateId: "research-team",
      groupTemplateVersion: group!.version,
      groupName: "research-team",
      mission: "Research the market",
      companyId: "company-a",
      workspaceId: "ws-a",
      driverAvailability: availability([]),
      rolePlans: group!.roleSlots
        .filter((slot) => slot.required)
        .map((slot) => ({
          slotId: slot.id,
          action: "create" as const,
          agentName: slot.defaultAgentName,
          setupInputs: { research_question: "Research the market" },
          instructions: "Follow the research plan.",
        })),
    });

    expect(result.valid).toBe(false);
    expect(result.errors.map((error) => error.code)).toContain("driver_unavailable");
  });

  it("allows optional roles to be skipped as warnings", () => {
    const result = validateGroupLaunchPlan({
      groupTemplateId: "software-development-team",
      groupName: "software-team",
      mission: "Ship task 2",
      companyId: "company-a",
      workspaceId: "ws-a",
      driverAvailability: availability(["codex_terminal"]),
      rolePlans: [
        { slotId: "project-operator", action: "create", agentName: "project-operator", instructions: "Coordinate." },
        { slotId: "backend-engineer", action: "create", agentName: "backend-engineer", instructions: "Build." },
        { slotId: "code-reviewer", action: "create", agentName: "code-reviewer", instructions: "Review." },
        { slotId: "frontend-engineer", action: "skip" },
        { slotId: "qa-tester", action: "skip" },
        { slotId: "devops-engineer", action: "skip" },
      ],
    });

    expect(result.valid).toBe(true);
    expect(result.warnings.map((warning) => warning.code)).toContain("optional_slot_skipped");
  });

  it("prevents duplicate existing-agent reuse in the same launch", () => {
    const result = validateGroupLaunchPlan({
      groupTemplateId: "software-development-team",
      groupName: "software-team",
      mission: "Ship task 2",
      companyId: "company-a",
      workspaceId: "ws-a",
      existingAgents: [{
        principal: principal({ id: "agent-a" }),
        runtimeBinding: binding({ agent_principal_id: "agent-a" }),
        companyId: "company-a",
      }],
      rolePlans: [
        { slotId: "project-operator", action: "reuse", existingAgentId: "agent-a" },
        { slotId: "backend-engineer", action: "reuse", existingAgentId: "agent-a" },
        { slotId: "code-reviewer", action: "create", agentName: "code-reviewer", instructions: "Review." },
      ],
    });

    expect(result.valid).toBe(false);
    expect(result.errors.map((error) => error.code)).toContain("duplicate_existing_agent_assignment");
  });

  it("prevents duplicate created agent names in the same launch", () => {
    const result = validateGroupLaunchPlan({
      groupTemplateId: "software-development-team",
      groupName: "software-team",
      mission: "Ship task 2",
      companyId: "company-a",
      workspaceId: "ws-a",
      driverAvailability: availability(["codex_terminal"]),
      rolePlans: [
        { slotId: "project-operator", action: "create", agentName: "same-agent", instructions: "Coordinate." },
        { slotId: "backend-engineer", action: "create", agentName: "same-agent", instructions: "Build." },
        { slotId: "code-reviewer", action: "create", agentName: "code-reviewer", instructions: "Review." },
      ],
    });

    expect(result.valid).toBe(false);
    expect(result.errors.map((error) => error.code)).toContain("duplicate_created_agent_name");
  });

  it("reports duplicate role slot plan submissions", () => {
    const result = validateGroupLaunchPlan({
      groupTemplateId: "software-development-team",
      groupName: "software-team",
      mission: "Ship task 2",
      companyId: "company-a",
      workspaceId: "ws-a",
      driverAvailability: availability(["codex_terminal"]),
      rolePlans: [
        { slotId: "project-operator", action: "create", agentName: "project-operator", instructions: "Coordinate." },
        { slotId: "project-operator", action: "create", agentName: "project-operator-2", instructions: "Also coordinate." },
        { slotId: "backend-engineer", action: "create", agentName: "backend-engineer", instructions: "Build." },
        { slotId: "code-reviewer", action: "create", agentName: "code-reviewer", instructions: "Review." },
      ],
    });

    expect(result.valid).toBe(false);
    expect(result.errors.map((error) => error.code)).toContain("duplicate_role_slot_plan");
  });

  it("blocks existing-agent reuse across workspaces", () => {
    const result = validateGroupLaunchPlan({
      groupTemplateId: "software-development-team",
      groupName: "software-team",
      mission: "Ship task 2",
      companyId: "company-a",
      workspaceId: "ws-a",
      existingAgents: [{
        principal: principal({ id: "agent-b", workspace_id: "ws-b" }),
        runtimeBinding: binding({ agent_principal_id: "agent-b", workspace_id: "ws-b" }),
        companyId: "company-a",
      }],
      rolePlans: [
        { slotId: "project-operator", action: "reuse", existingAgentId: "agent-b" },
        { slotId: "backend-engineer", action: "create", agentName: "backend-engineer", instructions: "Build." },
        { slotId: "code-reviewer", action: "create", agentName: "code-reviewer", instructions: "Review." },
      ],
    });

    expect(result.valid).toBe(false);
    expect(result.errors.map((error) => error.code)).toContain("workspace_mismatch");
  });

  it("blocks existing-agent reuse when company membership is missing", () => {
    const agentWithoutCompany = {
      principal: principal({ id: "agent-a" }),
      runtimeBinding: binding({ agent_principal_id: "agent-a" }),
    } as unknown as ExistingAgentCandidate;

    const result = validateGroupLaunchPlan({
      groupTemplateId: "software-development-team",
      groupName: "software-team",
      mission: "Ship task 2",
      companyId: "company-a",
      workspaceId: "ws-a",
      existingAgents: [agentWithoutCompany],
      rolePlans: [
        { slotId: "project-operator", action: "reuse", existingAgentId: "agent-a" },
        { slotId: "backend-engineer", action: "create", agentName: "backend-engineer", instructions: "Build." },
        { slotId: "code-reviewer", action: "create", agentName: "code-reviewer", instructions: "Review." },
      ],
    });

    expect(result.valid).toBe(false);
    expect(result.errors.map((error) => error.code)).toContain("company_membership_unverified");
  });

  it("blocks existing-agent reuse across companies", () => {
    const result = validateGroupLaunchPlan({
      groupTemplateId: "software-development-team",
      groupName: "software-team",
      mission: "Ship task 2",
      companyId: "company-a",
      workspaceId: "ws-a",
      existingAgents: [{
        principal: principal({ id: "agent-a" }),
        runtimeBinding: binding({ agent_principal_id: "agent-a" }),
        companyId: "company-b",
      }],
      rolePlans: [
        { slotId: "project-operator", action: "reuse", existingAgentId: "agent-a" },
        { slotId: "backend-engineer", action: "create", agentName: "backend-engineer", instructions: "Build." },
        { slotId: "code-reviewer", action: "create", agentName: "code-reviewer", instructions: "Review." },
      ],
    });

    expect(result.valid).toBe(false);
    expect(result.errors.map((error) => error.code)).toContain("company_mismatch");
  });

  it("allows existing-agent reuse with matching company and workspace membership", () => {
    const result = validateGroupLaunchPlan({
      groupTemplateId: "software-development-team",
      groupName: "software-team",
      mission: "Ship task 2",
      companyId: "company-a",
      workspaceId: "ws-a",
      existingAgents: [{
        principal: principal({ id: "agent-a" }),
        runtimeBinding: binding({ agent_principal_id: "agent-a" }),
        companyId: "company-a",
      }],
      rolePlans: [
        { slotId: "project-operator", action: "reuse", existingAgentId: "agent-a" },
        { slotId: "backend-engineer", action: "create", agentName: "backend-engineer", instructions: "Build." },
        { slotId: "code-reviewer", action: "create", agentName: "code-reviewer", instructions: "Review." },
      ],
    });

    expect(result.valid).toBe(true);
    expect(result.errors).toEqual([]);
  });

  it("blocks reused humans and internal agents from generated task-owner plans", () => {
    const result = validateGroupLaunchPlan({
      groupTemplateId: "software-development-team",
      groupName: "software-team",
      mission: "Ship task 2",
      companyId: "company-a",
      workspaceId: "ws-a",
      existingAgents: [
        {
          principal: principal({ id: "human-a", principal_type: "human", name: "Human A" }),
          runtimeBinding: binding({ agent_principal_id: "human-a" }),
          companyId: "company-a",
        },
        {
          principal: principal({ id: "agent-internal", channel_visibility: "internal", name: "Internal Agent" }),
          runtimeBinding: binding({ agent_principal_id: "agent-internal" }),
          companyId: "company-a",
        },
      ],
      rolePlans: [
        { slotId: "project-operator", action: "reuse", existingAgentId: "human-a" },
        { slotId: "backend-engineer", action: "reuse", existingAgentId: "agent-internal" },
        { slotId: "code-reviewer", action: "create", agentName: "code-reviewer", instructions: "Review." },
      ],
    });

    expect(result.valid).toBe(false);
    expect(result.errors.map((error) => error.code)).toEqual(expect.arrayContaining([
      "existing_principal_not_agent",
      "existing_agent_internal",
    ]));
  });

  it("surfaces disabled and offline reused agents as warnings", () => {
    const result = validateGroupLaunchPlan({
      groupTemplateId: "software-development-team",
      groupName: "software-team",
      mission: "Ship task 2",
      companyId: "company-a",
      workspaceId: "ws-a",
      existingAgents: [{
        principal: principal({ disabled: true }),
        runtimeBinding: binding({ state: "offline" }),
        companyId: "company-a",
      }],
      rolePlans: [
        { slotId: "project-operator", action: "reuse", existingAgentId: "agent-a" },
        { slotId: "backend-engineer", action: "create", agentName: "backend-engineer", instructions: "Build." },
        { slotId: "code-reviewer", action: "create", agentName: "code-reviewer", instructions: "Review." },
      ],
    });

    expect(result.valid).toBe(true);
    expect(result.warnings.map((warning) => warning.code)).toEqual(
      expect.arrayContaining(["existing_agent_disabled", "runtime_binding_not_reusable"]),
    );
  });

  it("validates webhook URL and secret rules", () => {
    const invalid = validateRoleAgentCreation({
      roleTemplateId: "backend-engineer",
      agentName: "backend-webhook",
      userOverrideDriver: "webhook_agent",
      webhookUrl: "ftp://example.com/hook",
      webhookSecret: "short",
      driverAvailability: availability(["webhook_agent"]),
    });
    expect(invalid.errors.map((error) => error.code)).toEqual(
      expect.arrayContaining(["invalid_webhook_url", "webhook_secret_too_short"]),
    );
    expect(invalid.warnings.map((warning) => warning.code)).toContain("incompatible_driver");
  });

  it("validates workspace and skill path shape rules", () => {
    const result = validateRoleAgentCreation({
      roleTemplateId: "backend-engineer",
      agentName: "backend-engineer",
      setupInputs: {},
      instructions: "Build backend.",
      workspacePath: "/tmp/not-home",
      skillPaths: ["relative-skill"],
      homeDir: "/Users/alice",
      driverAvailability: availability(["codex_terminal"]),
    });

    expect(result.errors.map((error) => error.code)).toEqual(
      expect.arrayContaining(["path_outside_home", "path_must_be_absolute"]),
    );
  });

  it("blocks reuse when the caller reports active conflicting job ids", () => {
    const result = validateGroupLaunchPlan({
      groupTemplateId: "software-development-team",
      groupName: "software-team",
      mission: "Ship task 2",
      companyId: "company-a",
      workspaceId: "ws-a",
      existingAgents: [{
        principal: principal(),
        runtimeBinding: binding(),
        companyId: "company-a",
      }],
      rolePlans: [
        { slotId: "project-operator", action: "reuse", existingAgentId: "agent-a", activeConflictingJobIds: ["job-a"] },
        { slotId: "backend-engineer", action: "create", agentName: "backend-engineer", instructions: "Build." },
        { slotId: "code-reviewer", action: "create", agentName: "code-reviewer", instructions: "Review." },
      ],
    });

    expect(result.valid).toBe(false);
    expect(result.errors.map((error) => error.code)).toContain("active_conflicting_job");
  });
});
