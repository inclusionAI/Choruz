import { afterEach, describe, expect, it, vi } from "vitest";

import {
  applyGroupDefaultDriver,
  blockingGroupTemplateIssues,
  createGroupProvisioningJob,
  buildGroupLaunchPlan,
  buildProvisioningJobCreationRequest,
  buildGroupReviewItems,
  createGroupTemplateDraft,
  groupTemplateIssues,
} from "./create-group-template-flow";
import type { Principal, RuntimeBindingInfo } from "../api/choruz-types";
import type { GroupProvisioningJobContract } from "./group-provisioning-contract";
import { getGroupTemplate } from "./team-templates";
import type { ClientDriverAvailabilityItem } from "../agents/create-agent-template-flow";

describe("create group template flow", () => {
  const originalFetch = globalThis.fetch;

  afterEach(() => {
    globalThis.fetch = originalFetch;
    vi.restoreAllMocks();
  });

  it("creates a launch draft with required roles and passive kickoff", () => {
    const template = getGroupTemplate("software-development-team")!;

    const draft = createGroupTemplateDraft({ groupTemplate: template, mission: "Ship onboarding MVP" });
    const plan = buildGroupLaunchPlan(draft);

    expect(draft.groupName).toBe("software-development-team-ship-onboarding-mvp");
    expect(plan.rolePlans.filter((role) => role.action === "create").map((role) => role.slotId)).toEqual([
      "project-operator",
      "backend-engineer",
      "code-reviewer",
    ]);
    expect(plan.rolePlans.filter((role) => role.action === "skip").map((role) => role.slotId)).toContain("frontend-engineer");
    expect(plan.workflow).toEqual({
      coordinatorRoleSlotId: "project-operator",
      participantRoleDefaults: {
        "project-operator": ["coordinator"],
        "backend-engineer": ["owner"],
        "code-reviewer": ["quality_check"],
      },
    });
    expect(plan.workflow?.participantRoleDefaults).not.toHaveProperty("frontend-engineer");
    expect(plan.workflow?.participantRoleDefaults).not.toHaveProperty("qa-tester");
    expect(plan.workflow?.participantRoleDefaults).not.toHaveProperty("devops-engineer");
    expect(plan.startWorkMode).toBe("manual");
    expect(plan.kickoffText).not.toMatch(/@[a-z0-9]/i);
    expect(plan.kickoffText).toContain("Roles: Project Operator, Backend Engineer, Code Reviewer");
    expect(plan.kickoffText).toContain("Please wait for the user");
  });

  it("blocks missing mission, required skips, and duplicate reused agents", () => {
    const template = getGroupTemplate("software-development-team")!;
    const draft = createGroupTemplateDraft({ groupTemplate: template });
    const nextDraft = {
      ...draft,
      roleDrafts: draft.roleDrafts.map((role) => {
        if (role.slotId === "project-operator") return { ...role, action: "skip" as const };
        if (role.slotId === "backend-engineer" || role.slotId === "code-reviewer") {
          return { ...role, action: "reuse" as const, existingAgentId: "agent-a" };
        }
        return role;
      }),
    };

    const issues = blockingGroupTemplateIssues({
      draft: nextDraft,
      existingAgents: [agent("agent-a", "Reusable")],
      runtimeBindings: [binding("agent-a")],
    });

    expect(issues.map((issue) => issue.code)).toEqual(expect.arrayContaining([
      "missing_group_mission",
      "required_slot_skipped",
      "duplicate_existing_agent_assignment",
    ]));
  });

  it("keeps reused-agent incompatibilities as warnings instead of mutating the agent", () => {
    const template = getGroupTemplate("software-development-team")!;
    const draft = createGroupTemplateDraft({ groupTemplate: template, mission: "Review UI" });
    const nextDraft = {
      ...draft,
      roleDrafts: draft.roleDrafts.map((role) =>
        role.slotId === "backend-engineer"
          ? { ...role, action: "reuse" as const, existingAgentId: "agent-a" }
          : role
      ),
    };

    const issues = groupTemplateIssues({
      draft: nextDraft,
      existingAgents: [agent("agent-a", "Reusable")],
      runtimeBindings: [binding("agent-a", "webhook_agent")],
    });

    expect(issues).toEqual(expect.arrayContaining([
      expect.objectContaining({ severity: "warning", code: "reuse_driver_mismatch", roleSlotId: "backend-engineer" }),
    ]));
    expect(buildGroupLaunchPlan(nextDraft).rolePlans).toEqual(expect.arrayContaining([
      expect.objectContaining({ action: "reuse", existingAgentId: "agent-a" }),
    ]));
  });

  it("keeps created-role driver incompatibilities as warnings instead of blocking review", () => {
    const template = getGroupTemplate("software-development-team")!;
    const draft = createGroupTemplateDraft({ groupTemplate: template, mission: "Ship API" });
    const nextDraft = {
      ...draft,
      roleDrafts: draft.roleDrafts.map((role) =>
        role.slotId === "backend-engineer"
          ? {
              ...role,
              driver: "webhook_agent" as const,
              webhookUrl: "https://example.com/hook",
            }
          : role
      ),
    };

    const issues = groupTemplateIssues({
      draft: nextDraft,
      existingAgents: [],
      runtimeBindings: [],
    });

    expect(issues).toEqual(expect.arrayContaining([
      expect.objectContaining({ severity: "warning", code: "incompatible_driver", roleSlotId: "backend-engineer" }),
    ]));
    expect(blockingGroupTemplateIssues({
      draft: nextDraft,
      existingAgents: [],
      runtimeBindings: [],
    }).map((issue) => issue.code)).not.toContain("incompatible_driver");
  });

  it("blocks review when a created role selects an unavailable driver", () => {
    const template = getGroupTemplate("software-development-team")!;
    const draft = createGroupTemplateDraft({ groupTemplate: template, mission: "Ship API" });
    const issues = blockingGroupTemplateIssues({
      draft,
      existingAgents: [],
      runtimeBindings: [],
      availability: unavailableDrivers(["codex_terminal"]),
    });

    expect(issues).toEqual(expect.arrayContaining([
      expect.objectContaining({ severity: "error", code: "driver_unavailable", roleSlotId: "project-operator" }),
    ]));
  });

  it("validates webhook role settings and carries them into the launch plan", () => {
    const template = getGroupTemplate("software-development-team")!;
    const draft = createGroupTemplateDraft({ groupTemplate: template, mission: "Ship API" });
    const invalidDraft = {
      ...draft,
      roleDrafts: draft.roleDrafts.map((role) =>
        role.slotId === "backend-engineer"
          ? {
              ...role,
              driver: "webhook_agent" as const,
              webhookUrl: "ftp://example.com/hook",
              webhookSecret: "short",
            }
          : role
      ),
    };

    expect(blockingGroupTemplateIssues({
      draft: invalidDraft,
      existingAgents: [],
      runtimeBindings: [],
    }).map((issue) => issue.code)).toEqual(expect.arrayContaining([
      "invalid_webhook_url",
      "webhook_secret_too_short",
      "webhook_secret_not_supported",
    ]));

    const validDraft = {
      ...draft,
      roleDrafts: draft.roleDrafts.map((role) =>
        role.slotId === "backend-engineer"
          ? {
              ...role,
              driver: "webhook_agent" as const,
              webhookUrl: "https://example.com/hook",
              webhookSecret: "",
            }
          : role
      ),
    };

    expect(blockingGroupTemplateIssues({
      draft: validDraft,
      existingAgents: [],
      runtimeBindings: [],
    }).map((issue) => issue.code)).not.toEqual(expect.arrayContaining([
      "invalid_webhook_url",
      "webhook_secret_too_short",
      "missing_webhook_url",
    ]));
    expect(buildGroupLaunchPlan(validDraft).rolePlans).toEqual(expect.arrayContaining([
      expect.objectContaining({
        slotId: "backend-engineer",
        action: "create",
        driver: "webhook_agent",
        webhookConfigured: true,
        webhookUrl: "https://example.com/hook",
      }),
    ]));
  });

  it("labels reused-agent warnings for drivers outside the template picker set", () => {
    const template = getGroupTemplate("software-development-team")!;
    const draft = createGroupTemplateDraft({ groupTemplate: template, mission: "Review UI" });
    const nextDraft = {
      ...draft,
      roleDrafts: draft.roleDrafts.map((role) =>
        role.slotId === "backend-engineer"
          ? { ...role, action: "reuse" as const, existingAgentId: "agent-a" }
          : role
      ),
    };

    const issues = groupTemplateIssues({
      draft: nextDraft,
      existingAgents: [agent("agent-a", "Reusable")],
      runtimeBindings: [binding("agent-a", "acp")],
    });

    expect(issues).toEqual(expect.arrayContaining([
      expect.objectContaining({
        code: "reuse_driver_mismatch",
        message: expect.stringContaining("acp"),
      }),
    ]));
  });

  it("applies a compatible group default driver to created roles", () => {
    const template = getGroupTemplate("software-development-team")!;
    const draft = createGroupTemplateDraft({ groupTemplate: template, mission: "Ship UI" });

    const updated = applyGroupDefaultDriver(draft, "claude_terminal");

    expect(updated.roleDrafts.filter((role) => role.action === "create").every((role) => role.driver === "claude_terminal")).toBe(true);
  });

  it("builds review rows with side effects and waiting behavior", () => {
    const template = getGroupTemplate("research-team")!;
    const draft = createGroupTemplateDraft({ groupTemplate: template, mission: "Compare two papers" });

    const items = buildGroupReviewItems(draft);

    expect(items).toEqual(expect.arrayContaining([
      { label: "Template", value: "Research Team" },
      { label: "Coordinator", value: "Synthesizer" },
      expect.objectContaining({ label: "Kickoff behavior", value: expect.stringContaining("work waits") }),
    ]));
  });

  it("creates provisioning jobs with same-origin credentials for cookie auth", async () => {
    const template = getGroupTemplate("research-team")!;
    const draft = createGroupTemplateDraft({ groupTemplate: template, mission: "Compare two papers" });
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify({
      job: jobContract("job-1", buildGroupLaunchPlan(draft)),
    }), {
      status: 201,
      headers: { "content-type": "application/json" },
    }));
    globalThis.fetch = fetchMock as typeof fetch;

    const job = await createGroupProvisioningJob({
      idempotencyKey: "idem-1",
      companyId: "company-2",
      plan: buildGroupLaunchPlan(draft),
    });

    expect(job.id).toBe("job-1");
    expect(fetchMock).toHaveBeenCalledWith("/api/group-provisioning-jobs", expect.objectContaining({
      method: "POST",
      credentials: "same-origin",
    }));
    const headers = new Headers(fetchMock.mock.calls[0]?.[1]?.headers);
    expect(headers.get("content-type")).toBe("application/json");
    expect(JSON.parse(fetchMock.mock.calls[0]?.[1]?.body as string)).toMatchObject({
      idempotencyKey: "idem-1",
      companyId: "company-2",
    });
  });

  it("builds provisioning job creation requests for the selected company", () => {
    const template = getGroupTemplate("research-team")!;
    const draft = createGroupTemplateDraft({ groupTemplate: template, mission: "Compare two papers" });
    const createdRole = draft.roleDrafts.find((role) => role.action === "create")!;
    createdRole.harnessAccountId = "12345678-1234-1234-1234-123456789abc";
    createdRole.harnessAccountName = "Claude work";
    createdRole.model = "claude-opus-5";

    const request = buildProvisioningJobCreationRequest(draft, "company-2");

    expect(request.companyId).toBe("company-2");
    expect(request.plan.groupTemplateId).toBe("research-team");
    expect(request.plan.rolePlans.find((role) => role.action === "create")).toMatchObject({
      harnessAccountId: "12345678-1234-1234-1234-123456789abc",
      model: "claude-opus-5",
    });
  });

  it("launches a new Claude or Codex role without an account choice", () => {
    const template = getGroupTemplate("research-team")!;
    const draft = createGroupTemplateDraft({ groupTemplate: template, mission: "Compare two papers" });

    expect(blockingGroupTemplateIssues({
      draft,
      existingAgents: [],
      runtimeBindings: [],
    }).map((item) => item.code)).not.toContain("missing_harness_account");
  });
});

function agent(id: string, name: string): Principal {
  return {
    id,
    workspace_id: "company-1",
    principal_type: "agent",
    name,
    avatar_url: null,
    scopes: [],
    disabled: false,
    created_at: "2026-05-19T00:00:00.000Z",
    updated_at: "2026-05-19T00:00:00.000Z",
  };
}

function binding(agentPrincipalId: string, driverType = "codex_terminal"): RuntimeBindingInfo {
  return {
    id: `binding-${agentPrincipalId}`,
    workspace_id: "company-1",
    conversation_id: `direct-${agentPrincipalId}`,
    conversation_type: "direct",
    agent_principal_id: agentPrincipalId,
    driver_type: driverType,
    workspace_path: "/tmp/workspace",
    state: "idle",
  };
}

function unavailableDrivers(driverIds: string[]): ClientDriverAvailabilityItem[] {
  return driverIds.map((driverId) => ({
    driverId: driverId as ClientDriverAvailabilityItem["driverId"],
    label: driverId,
    status: "unavailable",
    available: false,
    reason: `${driverId} missing`,
    setupHint: `Install ${driverId}`,
  }));
}

function jobContract(
  id: string,
  plan: GroupProvisioningJobContract["plan"],
): GroupProvisioningJobContract {
  return {
    id,
    companyId: "company-1",
    requestedBy: "human-1",
    idempotencyKey: "idem-1",
    status: "validating",
    plan,
    stepResults: [],
    progressSteps: [],
    issues: [],
    recoveryChoices: [],
    allowedUiActions: ["cancel"],
    allowedBackendActions: ["advance_job", "cancel_job"],
    createdAgentIds: [],
    reusedAgentIds: [],
    createdAt: "2026-05-19T00:00:00.000Z",
    updatedAt: "2026-05-19T00:00:00.000Z",
  };
}
