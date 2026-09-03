import { describe, expect, it } from "vitest";
import { markdownToFields } from "../agents/agent-instructions";
import {
  BOARD_TASKS_CREATED_SECTION,
  getGroupTemplate,
  getRoleTemplate,
  type RoleTemplate,
} from "./team-templates";
import {
  renderGroupKickoff,
  renderRoleInstructions,
  resetRoleInstructionsToTemplate,
} from "./team-template-renderer";

describe("renderRoleInstructions", () => {
  it("renders structured fields and markdown from a role template", () => {
    const roleTemplate = getRoleTemplate("backend-engineer");
    expect(roleTemplate).toBeTruthy();

    const rendered = renderRoleInstructions({
      roleTemplate: roleTemplate!,
      setupInputs: {
        repository_path: "/repo/echat",
        project_context: "Next.js frontend and Rust services.",
      },
      workspaceConstraints: "Use the provided repository workspace only.",
    });

    expect(rendered.status).toBe("template_default");
    expect(rendered.fields.role).toContain("Backend Engineer");
    expect(rendered.fields.role).toContain(roleTemplate!.instructionTemplate);
    expect(rendered.fields.role).toContain("Repository path");
    expect(rendered.fields.projectContext).toContain("/repo/echat");
    expect(rendered.fields.projectContext).toContain(roleTemplate!.suggestedAccess.workspace);
    expect(rendered.fields.projectContext).toContain(roleTemplate!.suggestedAccess.data);
    expect(rendered.fields.projectContext).toContain("Use the provided repository workspace only.");
    expect(rendered.fields.workflow).toContain("Role-specific instruction template");
    expect(rendered.markdown).toContain("## Role");
    expect(rendered.markdown).toContain("## Boundaries");
    expect(markdownToFields(rendered.markdown)).toEqual(rendered.fields);
  });

  it("adds group context and marks the status", () => {
    const groupTemplate = getGroupTemplate("software-development-team");
    const roleTemplate = getRoleTemplate("code-reviewer");
    const roleSlot = groupTemplate?.roleSlots.find((slot) => slot.roleTemplateId === "code-reviewer");
    expect(groupTemplate).toBeTruthy();
    expect(roleTemplate).toBeTruthy();
    expect(roleSlot).toBeTruthy();

    const rendered = renderRoleInstructions({
      roleTemplate: roleTemplate!,
      roleSlot,
      groupMission: "Ship the onboarding template registry.",
    });

    expect(rendered.status).toBe("group_context_added");
    expect(rendered.fields.role).toContain("role slot");
    expect(rendered.fields.workflow).toContain("Ship the onboarding template registry.");
    expect(rendered.fields.collaboration).toContain("Review changes");
  });

  it("marks customized markdown without losing the template-derived fields", () => {
    const roleTemplate = getRoleTemplate("research-analyst");
    expect(roleTemplate).toBeTruthy();

    const rendered = renderRoleInstructions({
      roleTemplate: roleTemplate!,
      customizedMarkdown: "## Custom\n\nUse my edited instructions.",
    });

    expect(rendered.status).toBe("customized");
    expect(rendered.markdown).toBe("## Custom\n\nUse my edited instructions.");
    expect(rendered.fields.role).toContain("Research Analyst");
  });

  it("resets customized output back to the current template-generated instructions", () => {
    const roleTemplate = getRoleTemplate("source-checker");
    expect(roleTemplate).toBeTruthy();

    const customized = renderRoleInstructions({
      roleTemplate: roleTemplate!,
      groupMission: "Compare vendor claims.",
      customizedMarkdown: "edited",
    });
    const reset = resetRoleInstructionsToTemplate({
      roleTemplate: roleTemplate!,
      groupMission: "Compare vendor claims.",
    });

    expect(customized.status).toBe("customized");
    expect(reset.status).toBe("group_context_added");
    expect(reset.markdown).not.toBe("edited");
    expect(reset.markdown).toContain("Compare vendor claims.");
  });

  it("teaches the project-operator that Board tasks created is a task_create receipt, not a prose plan (ADR-005)", () => {
    const roleTemplate = getRoleTemplate("project-operator");
    expect(roleTemplate).toBeTruthy();

    const rendered = renderRoleInstructions({
      roleTemplate: roleTemplate!,
      groupMission: "Ship the onboarding MVP.",
    });

    expect(rendered.fields.boundaries).toContain("- Board tasks created");
    expect(rendered.fields.boundaries).not.toContain("- Assignments");
    expect(rendered.fields.boundaries).toContain("receipt of the silent task_create commands");
    expect(rendered.fields.boundaries).toContain("`task_id — title — assignee`");
    expect(rendered.fields.boundaries).toContain("`Board tasks created: none — <reason>`");
    expect(rendered.fields.boundaries).toContain("Prose assignments do not satisfy this section");
    expect(rendered.markdown).toContain("Board tasks created");
    expect(rendered.markdown).toContain("Prose assignments do not satisfy this section");
  });

  it("does not inject the Board tasks created receipt guidance into role templates that do not require it", () => {
    const backend = getRoleTemplate("backend-engineer");
    expect(backend).toBeTruthy();
    const rendered = renderRoleInstructions({ roleTemplate: backend! });
    expect(rendered.fields.boundaries).not.toContain("Board tasks created");
    expect(rendered.fields.boundaries).not.toContain("receipt of the silent task_create commands");
  });

  it("keys Board tasks receipt guidance on BOARD_TASKS_CREATED_SECTION so a typo in the section name cannot silently disable it", () => {
    // Regression for #11: `requiredSections.includes("Board tasks created")` was an
    // exact-match string in two files. If either drifted (e.g. a future contributor
    // renamed to "Board tasks created:" or "Board Tasks Created"), the guidance would
    // vanish without any test catching it. We now share a single const between the
    // template and the renderer, and pin the canonical value here so a deliberate
    // rename has to update both files plus this test in lockstep.
    expect(BOARD_TASKS_CREATED_SECTION).toBe("Board tasks created");

    // Any custom role declaring the canonical section in its required sections gets
    // the guidance — proving the gate is keyed off the exported const, not a literal.
    const baseline = getRoleTemplate("project-operator");
    expect(baseline).toBeTruthy();
    const synthetic: RoleTemplate = {
      ...baseline!,
      id: "synthetic-custom-coordinator",
      outputContract: {
        ...baseline!.outputContract,
        requiredSections: [BOARD_TASKS_CREATED_SECTION, "Blockers"],
      },
    };
    const rendered = renderRoleInstructions({ roleTemplate: synthetic });
    expect(rendered.fields.boundaries).toContain(
      "receipt of the silent task_create commands",
    );
    expect(rendered.fields.boundaries).toContain(BOARD_TASKS_CREATED_SECTION);
  });
});

describe("renderGroupKickoff", () => {
  it("renders the mission without agent mentions or routing metadata", () => {
    const groupTemplate = getGroupTemplate("financial-analysis-team");
    expect(groupTemplate).toBeTruthy();

    const kickoff = renderGroupKickoff(groupTemplate!, "Evaluate Q2 margin risk.");

    expect(kickoff).toContain("Evaluate Q2 margin risk.");
    expect(kickoff).toContain("Roles: Lead Financial Analyst, Data Analyst, Risk Reviewer");
    expect(kickoff).toContain("Next user action:");
    expect(kickoff).not.toContain("@");
    expect(kickoff).not.toContain("agent_commands");
    expect(kickoff).not.toContain("app_mention");
  });

  it("can render actual current members instead of template role slots", () => {
    const groupTemplate = getGroupTemplate("software-development-team");
    expect(groupTemplate).toBeTruthy();

    const kickoff = renderGroupKickoff(groupTemplate!, "Ship onboarding MVP.", {
      members: [
        { name: "project-operator", roleLabel: "Project Operator" },
        { name: "backend-engineer", roleLabel: "Backend Engineer" },
        { name: "code-reviewer", roleLabel: "Code Reviewer" },
      ],
    });

    expect(kickoff).toContain("Current members: project-operator (Project Operator), backend-engineer (Backend Engineer), code-reviewer (Code Reviewer).");
    expect(kickoff).not.toContain("Frontend Engineer");
    expect(kickoff).not.toContain("QA Tester");
    expect(kickoff).not.toContain("DevOps Engineer");
  });
});
