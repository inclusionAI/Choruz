import { describe, expect, it } from "vitest";

import { emptyFields } from "./agent-instructions";
import {
  buildCreateAgentProvisioningPayload,
  buildCreateAgentReviewItems,
  type ClientDriverAvailabilityItem,
  createTemplateDraft,
  driverWarnings,
  groupedRoleTemplates,
  instructionStatusLabel,
  regenerateTemplateInstructions,
  templateBlockingIssues,
} from "./create-agent-template-flow";
import { driverDisplayName } from "../drivers/driver-registry";
import { getRoleTemplate } from "../groups/team-templates";

describe("create agent template flow", () => {
  const frontendTemplate = getRoleTemplate("frontend-engineer");
  const financeTemplate = getRoleTemplate("lead-financial-analyst");

  it("groups role templates by category and includes Blank Agent separately in the UI", () => {
    const groups = groupedRoleTemplates();
    expect(groups.map((group) => group.category)).toContain("software");
    expect(groups.flatMap((group) => group.templates.map((template) => template.id))).toContain("frontend-engineer");
  });

  it("selecting a template populates name, recommended driver, setup inputs, and generated instructions", () => {
    expect(frontendTemplate).toBeTruthy();
    const draft = createTemplateDraft({ roleTemplate: frontendTemplate! });

    expect(draft.agentName).toBe("frontend-engineer");
    expect(draft.driver).toBe(frontendTemplate!.recommendedDriver);
    expect(draft.setupInputs).toEqual({
      repository_path: "",
      project_context: "",
    });
    expect(draft.instructionStatus).toBe("template_default");
    expect(draft.instructionFields.role).toContain("Frontend Engineer");
    expect(draft.instructionFields.boundaries).toContain("browser-verification");
  });

  it("regenerates setup-sensitive instructions and supports reset-to-template after customization", () => {
    expect(frontendTemplate).toBeTruthy();
    const customized = createTemplateDraft({ roleTemplate: frontendTemplate! });
    customized.instructionFields.role = "custom role";

    const reset = regenerateTemplateInstructions({
      roleTemplate: frontendTemplate!,
      setupInputs: {
        repository_path: "/Users/alice/projects/example-frontend",
        project_context: "Next.js frontend with modal tests",
      },
    });

    expect(reset.instructionStatus).toBe("template_default");
    expect(reset.instructionFields.role).toContain("Frontend Engineer");
    expect(reset.instructionFields.role).not.toBe("custom role");
    expect(reset.instructionFields.projectContext).toContain("Next.js frontend with modal tests");
    expect(reset.instructionFields.projectContext).toContain("/Users/alice/projects/example-frontend");
  });

  it("labels instruction states for the modal and review step", () => {
    expect(instructionStatusLabel("template_default")).toBe("Template default");
    expect(instructionStatusLabel("customized")).toBe("Customized");
    expect(instructionStatusLabel("group_context_added")).toBe("Group context added");
  });

  it("presents legacy Codex exec bindings as Codex", () => {
    expect(driverDisplayName("codex_terminal")).toBe("Codex");
    expect(driverDisplayName("codex_exec")).toBe("Codex");
  });

  it("does not place external webhook agents on a runtime host", () => {
    const payload = buildCreateAgentProvisioningPayload({
      name: "External Agent",
      driver: "webhook_agent",
      webhookUrl: "https://example.com/hook",
      runtimeHostId: "host-west",
      instructionFields: emptyFields(),
      workspaceMode: "generated",
      selectedSkillPaths: [],
    });

    expect(payload.runtime_host_id).toBeUndefined();
    expect(buildCreateAgentReviewItems({
      agentName: "External Agent",
      driver: "webhook_agent",
      runtimeHostName: "Build Server West",
      workspaceMode: "generated",
    })).not.toContainEqual(expect.objectContaining({ label: "Runtime server" }));
  });

  it("warns without blocking incompatible driver overrides", () => {
    expect(frontendTemplate).toBeTruthy();
    const warnings = driverWarnings({
      roleTemplate: frontendTemplate!,
      driver: "webhook_agent",
    });

    expect(warnings.some((warning) => warning.code === "incompatible_driver")).toBe(true);
    expect(templateBlockingIssues({
      roleTemplate: frontendTemplate!,
      driver: "webhook_agent",
      webhookUrl: "https://example.com/hook",
    })).toEqual([]);
  });

  it("warns when the selected driver is unavailable and suggests an available compatible driver", () => {
    expect(frontendTemplate).toBeTruthy();
    const availability: ClientDriverAvailabilityItem[] = [
      {
        driverId: "codex_terminal" as const,
        label: "Codex",
        status: "unavailable",
        reason: "Codex CLI was not found.",
        setupHint: "Install Codex.",
      },
      {
        driverId: "claude_terminal" as const,
        label: "Claude",
        status: "available",
        reason: "Claude CLI is available.",
        setupHint: "Install Claude.",
      },
    ];
    const warnings = driverWarnings({
      roleTemplate: frontendTemplate!,
      driver: "codex_terminal",
      availability,
    });

    expect(warnings.map((warning) => warning.code)).toEqual([
      "driver_unavailable",
      "driver_fallback_available",
    ]);
    expect(templateBlockingIssues({
      roleTemplate: frontendTemplate!,
      driver: "codex_terminal",
      availability,
    })).toEqual([
      {
        code: "driver_unavailable",
        message: "Codex CLI was not found.",
        field: "driver",
      },
    ]);
  });

  it("builds review rows with driver, workspace, skills, instruction status, webhook, and mentionability", () => {
    expect(frontendTemplate).toBeTruthy();
    const rows = buildCreateAgentReviewItems({
      agentName: "frontend-engineer",
      driver: "webhook_agent",
      workspaceMode: "custom",
      workspacePath: "/Users/alice/workspaces/frontend",
      roleTemplate: frontendTemplate,
      selectedSkillNames: ["browser-verification"],
      instructionStatus: "customized",
      webhookUrl: "https://example.com/hook",
      webhookSecretProvided: true,
    });

    expect(Object.fromEntries(rows.map((row) => [row.label, row.value]))).toMatchObject({
      Driver: "External agent",
      "Workspace behavior": "Custom path: /Users/alice/workspaces/frontend",
      "Selected skills": "browser-verification",
      Instructions: "Customized",
      Webhook: "https://example.com/hook with provided signing secret",
      Mentionability: "@frontend-engineer in direct or group chat",
    });
  });

  it("does not present suggested skills as selected skills in review", () => {
    expect(frontendTemplate).toBeTruthy();
    const rows = buildCreateAgentReviewItems({
      agentName: "frontend-engineer",
      driver: "codex_terminal",
      workspaceMode: "generated",
      roleTemplate: frontendTemplate,
      selectedSkillNames: [],
      instructionStatus: "template_default",
    });

    expect(Object.fromEntries(rows.map((row) => [row.label, row.value]))).toMatchObject({
      "Selected skills": "None",
    });
  });

  it("adds template metadata to create payloads only for template mode", () => {
    expect(frontendTemplate).toBeTruthy();
    const draft = createTemplateDraft({ roleTemplate: frontendTemplate! });
    const payload = buildCreateAgentProvisioningPayload({
      name: draft.agentName,
      driver: draft.driver,
      instructionFields: draft.instructionFields,
      activeCompanyId: "company-1",
      workspaceMode: "generated",
      selectedSkillPaths: ["/Users/alice/.codex/skills/browser"],
      roleTemplate: frontendTemplate,
      setupInputs: draft.setupInputs,
      instructionStatus: draft.instructionStatus,
    });

    expect(payload.template_metadata).toMatchObject({
      mode: "role_template",
      roleTemplateId: "frontend-engineer",
      roleTemplateVersion: frontendTemplate!.version,
      instructionStatus: "template_default",
      workspaceMode: "generated",
      driverSource: "role_template_recommendation",
    });
    expect(payload.instructions).toContain("Frontend Engineer");
  });

  it("preserves blank-agent payload shape with no template metadata", () => {
    const payload = buildCreateAgentProvisioningPayload({
      name: "Manual Agent",
      driver: "claude_terminal",
      instructionFields: emptyFields(),
      workspaceMode: "generated",
      selectedSkillPaths: [],
    });

    expect(payload).toEqual({
      name: "Manual Agent",
      driver_type: "claude_terminal",
      instructions: "",
    });
  });

  it("includes the selected model in provisioning and review", () => {
    const payload = buildCreateAgentProvisioningPayload({
      name: "Model Agent",
      driver: "claude_terminal",
      model: "sonnet",
      instructionFields: emptyFields(),
      workspaceMode: "generated",
      selectedSkillPaths: [],
    });
    const review = buildCreateAgentReviewItems({
      agentName: "Model Agent",
      driver: "claude_terminal",
      model: "sonnet",
      workspaceMode: "generated",
    });

    expect(payload.model).toBe("sonnet");
    expect(review).toContainEqual({ label: "Model", value: "sonnet" });
  });

  it("keeps runtime-host placement in the idempotent provisioning payload", () => {
    const payload = buildCreateAgentProvisioningPayload({
      name: "Remote Agent",
      driver: "codex_terminal",
      runtimeHostId: "host-west",
      harnessAccountId: "12345678-1234-1234-1234-123456789abc",
      instructionFields: emptyFields(),
      workspaceMode: "generated",
      selectedSkillPaths: [],
    });
    const review = buildCreateAgentReviewItems({
      agentName: "Remote Agent",
      driver: "codex_terminal",
      runtimeHostName: "Build Server West",
      harnessAccountName: "Codex Team",
      workspaceMode: "generated",
    });

    expect(payload.runtime_host_id).toBe("host-west");
    expect(payload.harness_account_id).toBe("12345678-1234-1234-1234-123456789abc");
    expect(review).toContainEqual({
      label: "Runtime server",
      value: "Build Server West",
    });
    expect(review).toContainEqual({ label: "Harness account", value: "Codex Team" });
  });

  it("supports required setup templates in payload provenance", () => {
    expect(financeTemplate).toBeTruthy();
    const draft = createTemplateDraft({
      roleTemplate: financeTemplate!,
      setupInputs: {
        analysis_question: "Should we invest?",
        data_sources: "model.xlsx",
      },
    });

    const payload = buildCreateAgentProvisioningPayload({
      name: draft.agentName,
      driver: "claude_terminal",
      instructionFields: draft.instructionFields,
      workspaceMode: "generated",
      selectedSkillPaths: [],
      roleTemplate: financeTemplate,
      setupInputs: draft.setupInputs,
      instructionStatus: draft.instructionStatus,
    });

    expect(payload.template_metadata?.setupSummary).toMatchObject({
      analysis_question: "Should we invest?",
      data_sources: "model.xlsx",
    });
    expect(payload.instructions).toContain("Should we invest?");
  });

  it("blocks missing required setup inputs before review/create", () => {
    expect(financeTemplate).toBeTruthy();

    expect(templateBlockingIssues({
      roleTemplate: financeTemplate!,
      driver: financeTemplate!.recommendedDriver,
      setupInputs: {
        analysis_question: "",
      },
    })).toEqual([
      {
        code: "missing_required_setup_input",
        message: "Analysis question is required for Lead Financial Analyst.",
        field: "analysis_question",
      },
    ]);
  });

  it("blocks invalid webhook URL and short signing secret before review/create", () => {
    expect(templateBlockingIssues({
      driver: "webhook_agent",
      setupInputs: {},
      webhookUrl: "http://",
      webhookSecret: "short",
    })).toEqual([
      {
        code: "invalid_webhook_url",
        message: "Webhook URL must be a valid URL.",
        field: "webhookUrl",
      },
      {
        code: "webhook_secret_too_short",
        message: "Webhook secret must be at least 16 characters when provided.",
        field: "webhookSecret",
      },
    ]);
  });
});
