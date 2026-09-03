import { describe, expect, it } from "vitest";
import {
  GROUP_TEMPLATES,
  ROLE_TEMPLATES,
  generateAgentName,
  generateGroupName,
  getGroupTemplate,
  getRoleTemplate,
  type DriverId,
} from "./team-templates";

const expectedRoleIds = [
  "backend-engineer",
  "frontend-engineer",
  "code-reviewer",
  "qa-tester",
  "lead-financial-analyst",
  "data-analyst",
  "valuation-analyst",
  "risk-reviewer",
  "research-analyst",
  "source-checker",
  "synthesizer",
  "project-operator",
];

const expectedGroupIds = [
  "software-development-team",
  "financial-analysis-team",
  "research-team",
];

describe("onboarding template registry", () => {
  it("contains the MVP role templates with stable ids", () => {
    expect(ROLE_TEMPLATES.map((template) => template.id).sort()).toEqual(expectedRoleIds.sort());
  });

  it("contains the MVP group templates with stable ids", () => {
    expect(GROUP_TEMPLATES.map((template) => template.id).sort()).toEqual(expectedGroupIds.sort());
  });

  it("has complete role template metadata", () => {
    for (const template of ROLE_TEMPLATES) {
      expect(template.version).toMatch(/^\d+\.\d+\.\d+$/);
      expect(template.name).toBeTruthy();
      expect(template.description).toBeTruthy();
      expect(template.bestFor.length).toBeGreaterThan(0);
      expect(template.instructionTemplate.trim().length).toBeGreaterThan(20);
      expect(template.compatibleDrivers).toContain(template.recommendedDriver);
      expect(template.setupInputs.length).toBeGreaterThan(0);
      expect(template.suggestedAccess.workspace.trim().length).toBeGreaterThan(20);
      expect(template.suggestedAccess.data.trim().length).toBeGreaterThan(20);
      expect(template.suggestedSkills.length).toBeGreaterThan(0);
      expect(template.suggestedFirstTasks.length).toBeGreaterThan(0);
      expect(template.outputContract.requiredSections.length).toBeGreaterThan(0);
    }
  });

  it("has complete group metadata and valid role-slot references", () => {
    for (const template of GROUP_TEMPLATES) {
      expect(template.version).toMatch(/^\d+\.\d+\.\d+$/);
      expect(template.description).toBeTruthy();
      expect(template.defaultDriverPolicy.driver).toBeTruthy();
      expect(template.workflow.steps.length).toBeGreaterThan(0);
      expect(template.workflow.coordinatorRoleSlotId).toBeTruthy();
      expect(template.kickoffTemplate.body).toContain("{{mission}}");
      expect(template.outputContract.requiredSections.length).toBeGreaterThan(0);
      expect(template.roleSlots.some((slot) => slot.required)).toBe(true);
      expect(template.roleSlots.some((slot) => !slot.required)).toBe(true);
      const coordinatorSlot = template.roleSlots.find((slot) => slot.id === template.workflow.coordinatorRoleSlotId);
      expect(coordinatorSlot, `${template.id}:coordinator`).toBeTruthy();
      expect(coordinatorSlot?.workflowRoleKeys).toContain("coordinator");

      for (const slot of template.roleSlots) {
        expect(getRoleTemplate(slot.roleTemplateId), `${template.id}:${slot.id}`).toBeTruthy();
        expect(slot.defaultAgentName).toBe(generateAgentName(slot.label, slot.id));
        expect(slot.responsibilities.length).toBeGreaterThan(0);
        expect(slot.workflowRoleKeys?.length, `${template.id}:${slot.id}:workflowRoleKeys`).toBeGreaterThan(0);
      }
    }
  });

  it("declares group default drivers that are compatible with at least one required role", () => {
    for (const group of GROUP_TEMPLATES) {
      const requiredRoles = group.roleSlots
        .filter((slot) => slot.required)
        .map((slot) => getRoleTemplate(slot.roleTemplateId))
        .filter(Boolean);
      expect(requiredRoles.some((role) => role?.compatibleDrivers.includes(group.defaultDriverPolicy.driver))).toBe(true);
    }
  });

  it("does not declare unknown driver ids", () => {
    const validDriverIds = new Set<DriverId>([
      "claude_terminal",
      "codex_terminal",
      "codex_exec",
      "pi_terminal",
      "grok_terminal",
      "opencode_terminal",
      "webhook_agent",
    ]);

    for (const role of ROLE_TEMPLATES) {
      expect(validDriverIds.has(role.recommendedDriver)).toBe(true);
      expect(role.compatibleDrivers.every((driver) => validDriverIds.has(driver))).toBe(true);
    }
  });

  it("looks up templates by id", () => {
    expect(getRoleTemplate("backend-engineer")?.name).toBe("Backend Engineer");
    expect(getGroupTemplate("research-team")?.name).toBe("Research Team");
    expect(getRoleTemplate("missing")).toBeUndefined();
    expect(getGroupTemplate("missing")).toBeUndefined();
  });

  it("generates deterministic agent and group names", () => {
    expect(generateAgentName("Backend Engineer")).toBe("backend-engineer");
    expect(generateAgentName("QA Tester", "qa-tester")).toBe("qa-tester");
    expect(generateGroupName("Software Development Team")).toBe("software-development-team");
    expect(generateGroupName("Research Team", "Market map for LLM tooling")).toBe("research-team-market-map-for-llm");
  });

  it("requires the project-operator coordinator contract to ask for a Board tasks created receipt instead of prose Assignments (ADR-005)", () => {
    const operator = getRoleTemplate("project-operator");
    expect(operator).toBeTruthy();
    expect(operator!.outputContract.requiredSections).toContain("Board tasks created");
    expect(operator!.outputContract.requiredSections).not.toContain("Assignments");
  });

  it("does not let any role's output contract substitute prose Assignments for a Board tasks created receipt", () => {
    for (const role of ROLE_TEMPLATES) {
      const sections = role.outputContract.requiredSections;
      const declaresAssignments = sections.includes("Assignments");
      const declaresReceipt = sections.includes("Board tasks created");
      expect(declaresAssignments, `${role.id} still requires prose "Assignments"`).toBe(false);
      // If a role demands an assignment-shaped section at all, it must take the receipt form
      // so the agent cannot satisfy the contract without emitting task_create commands.
      const hasAssignmentShape = sections.some((section) =>
        /assignment|assignee|owner|task/i.test(section),
      );
      if (hasAssignmentShape) {
        expect(declaresReceipt, `${role.id} declares an assignment-shaped section without the receipt`).toBe(true);
      }
    }
  });
});
