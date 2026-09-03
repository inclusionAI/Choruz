import {
  GROUP_PROVISIONING_STATUS_CONTRACT,
  type GroupLaunchPlanContract,
  type GroupProvisioningIssue,
  type GroupProvisioningJobContract,
  type GroupProvisioningJobStatus,
  type GroupProvisioningStepResult,
  type ProgressStep,
  type RecoveryChoice,
} from "./group-provisioning-contract";

const NOW = "2026-05-19T00:00:00.000Z";
const DONE = "2026-05-19T00:02:00.000Z";
const provisionedKickoffText = "Mission: Ship the onboarding experience MVP contract gate safely.\n\nWorkflow: Plan -> implement -> review -> verify -> summarize.\n\nPlease wait for the user to provide the first concrete work item before starting execution.\n\nCurrent members: project-operator (Project Operator), backend-engineer (Backend Engineer), code-reviewer (Code Reviewer).\n\nNext user action: send the first concrete work item or question when ready. Until then, work waits for the user kickoff.";

const basePlan: GroupLaunchPlanContract = {
  groupTemplateId: "software-development-team",
  groupTemplateVersion: "1.0.0",
  groupName: "software-development-team-onboarding-mvp",
  mission: "Ship the onboarding experience MVP contract gate safely.",
  kickoffText: "Mission: Ship the onboarding experience MVP contract gate safely.\n\nWorkflow: Plan -> implement -> review -> verify -> summarize.\n\nPlease wait for the user to provide the first concrete work item before starting execution.\n\nRoles: Project Operator, Backend Engineer, Code Reviewer.\n\nNext user action: send the first concrete work item or question when ready. Until then, work waits for the user kickoff.",
  startWorkMode: "manual",
  workflow: {
    coordinatorRoleSlotId: "project-operator",
    participantRoleDefaults: {
      "project-operator": ["coordinator"],
      "backend-engineer": ["owner"],
      "code-reviewer": ["quality_check"],
      "frontend-engineer": ["owner"],
      "qa-tester": ["quality_check"],
      "devops-engineer": ["operations"],
    },
  },
  rolePlans: [
    {
      slotId: "project-operator",
      action: "create",
      agentName: "project-operator",
      roleTemplateId: "project-operator",
      roleTemplateVersion: "1.0.0",
      driver: "codex_terminal",
      instructionStatus: "group_context_added",
      setupInputs: { mission: "Coordinate the onboarding MVP." },
      selectedSkills: ["planning", "status-reporting"],
      workspaceMode: "generated",
    },
    {
      slotId: "backend-engineer",
      action: "create",
      agentName: "backend-engineer",
      roleTemplateId: "backend-engineer",
      roleTemplateVersion: "1.0.0",
      driver: "codex_terminal",
      instructionStatus: "group_context_added",
      setupInputs: { repository_path: "/Users/alice/projects/choruz", project_context: "Onboarding MVP" },
      selectedSkills: ["repo-navigation", "test-running"],
      workspaceMode: "custom",
      workspacePath: "/Users/alice/projects/choruz",
    },
    {
      slotId: "code-reviewer",
      action: "reuse",
      existingAgentId: "agent-reviewer-existing",
      roleTemplateId: "code-reviewer",
      roleTemplateVersion: "1.0.0",
    },
    {
      slotId: "qa-tester",
      action: "skip",
      roleTemplateId: "qa-tester",
      roleTemplateVersion: "1.0.0",
      reason: "optional",
    },
  ],
};

const happyResults: GroupProvisioningStepResult[] = [
  {
    kind: "created_agent",
    stepId: "agent-project-operator",
    roleSlotId: "project-operator",
    agentId: "agent-project-operator",
    agentName: "project-operator",
    driver: "codex_terminal",
    roleTemplateId: "project-operator",
    roleTemplateVersion: "1.0.0",
    instructionStatus: "group_context_added",
    workspaceMode: "generated",
  },
  {
    kind: "created_agent",
    stepId: "agent-backend-engineer",
    roleSlotId: "backend-engineer",
    agentId: "agent-backend-engineer",
    agentName: "backend-engineer",
    driver: "codex_terminal",
    roleTemplateId: "backend-engineer",
    roleTemplateVersion: "1.0.0",
    instructionStatus: "group_context_added",
    workspaceMode: "custom",
  },
  {
    kind: "reused_agent",
    stepId: "reuse-code-reviewer",
    roleSlotId: "code-reviewer",
    agentId: "agent-reviewer-existing",
    agentName: "code-reviewer",
    roleTemplateId: "code-reviewer",
    roleTemplateVersion: "1.0.0",
  },
  {
    kind: "skipped_optional_role",
    stepId: "skip-qa-tester",
    roleSlotId: "qa-tester",
    roleTemplateId: "qa-tester",
    reason: "user_choice",
  },
  {
    kind: "created_group",
    stepId: "group-create",
    groupConversationId: "conv-software-team",
    groupName: "software-development-team-onboarding-mvp",
  },
  member("project-operator", "agent-project-operator", "added"),
  member("backend-engineer", "agent-backend-engineer", "added"),
  member("code-reviewer", "agent-reviewer-existing", "added"),
  {
    kind: "routing_policy_update",
    stepId: "policy-enable",
    groupConversationId: "conv-software-team",
    result: "enabled",
  },
  {
    kind: "kickoff_post",
    stepId: "kickoff-post",
    groupConversationId: "conv-software-team",
    messageId: "msg-kickoff",
    kickoffText: provisionedKickoffText,
    result: "posted",
  },
];

export const groupProvisioningContractFixtures = {
  happyPath: job({
    id: "job-happy-path",
    status: "completed",
    stepResults: happyResults,
    createdAgentIds: ["agent-project-operator", "agent-backend-engineer"],
    reusedAgentIds: ["agent-reviewer-existing"],
    createdGroupId: "conv-software-team",
    completedAt: DONE,
  }),
  validationFailure: job({
    id: "job-validation-failure",
    status: "failed_validation",
    plan: {
      ...basePlan,
      groupName: "software-development-team",
      mission: " ",
    },
    issues: [
      issue("error", "missing_group_mission", "Group mission is required.", true, { field: "mission" }),
      issue("error", "agent_name_conflict", "An agent named backend-engineer already exists.", true, {
        field: "agentName",
        roleSlotId: "backend-engineer",
        roleTemplateId: "backend-engineer",
      }),
    ],
    recoveryChoices: [
      choice("edit_plan", "Edit setup", "Return to the launch plan and fix blocking fields.", "validating"),
      choice("cancel", "Cancel", "Cancel without creating agents or a group.", "canceled"),
    ],
    errorSummary: "Launch plan has blocking validation errors.",
  }),
  optionalAgentFailure: job({
    id: "job-optional-agent-failure",
    status: "partial_failure",
    stepResults: [
      happyResults[0],
      happyResults[1],
      {
        kind: "skipped_optional_role",
        stepId: "skip-frontend-engineer",
        roleSlotId: "frontend-engineer",
        roleTemplateId: "frontend-engineer",
        reason: "unavailable",
      },
    ],
    issues: [
      issue("warning", "optional_agent_create_failed", "Frontend Engineer could not be created and may be skipped.", true, {
        field: "rolePlans.frontend-engineer",
        roleSlotId: "frontend-engineer",
        roleTemplateId: "frontend-engineer",
      }),
    ],
    recoveryChoices: [
      choice("retry_agent_creation", "Retry agent", "Try creating the optional agent again.", "creating_agents", "frontend-engineer"),
      choice("skip_optional_role", "Skip role", "Continue launch without this optional role.", "creating_group", "frontend-engineer"),
      choice("cancel", "Cancel", "Cancel the launch and choose cleanup behavior.", "canceled"),
    ],
    createdAgentIds: ["agent-project-operator", "agent-backend-engineer"],
    errorSummary: "Optional role creation failed.",
  }),
  requiredAgentFailure: job({
    id: "job-required-agent-failure",
    status: "failed",
    stepResults: [happyResults[0]],
    issues: [
      issue("error", "required_agent_create_failed", "Backend Engineer could not be created.", true, {
        field: "rolePlans.backend-engineer",
        roleSlotId: "backend-engineer",
        roleTemplateId: "backend-engineer",
      }),
    ],
    recoveryChoices: [
      choice("retry_agent_creation", "Retry agent", "Try creating the required agent again.", "creating_agents", "backend-engineer"),
      choice("edit_plan", "Edit setup", "Update the required role setup and validate again.", "validating", "backend-engineer"),
      choice("soft_delete_generated_agents", "Disable generated agents", "Soft-delete generated agents created only for this job.", "rolled_back"),
      choice("cancel", "Cancel", "Cancel and leave generated assets disabled or preserved according to the selected cleanup.", "canceled"),
    ],
    createdAgentIds: ["agent-project-operator"],
    errorSummary: "Required role creation failed; group creation has not started.",
  }),
  groupCreationFailure: job({
    id: "job-group-creation-failure",
    status: "failed",
    stepResults: [happyResults[0], happyResults[1], happyResults[2], happyResults[3]],
    issues: [
      issue("error", "group_create_failed", "The group conversation could not be created.", true, {
        field: "groupName",
      }),
    ],
    recoveryChoices: [
      choice("retry_group_creation", "Retry group", "Try creating the group conversation again.", "creating_group"),
      choice("soft_delete_generated_agents", "Disable generated agents", "Soft-delete generated agents created only for this job.", "rolled_back"),
      choice("cancel", "Cancel", "Cancel without hard-deleting generated agents.", "canceled"),
    ],
    createdAgentIds: ["agent-project-operator", "agent-backend-engineer"],
    reusedAgentIds: ["agent-reviewer-existing"],
    errorSummary: "Group conversation creation failed.",
  }),
  memberAddPartialFailure: job({
    id: "job-member-add-partial-failure",
    status: "partial_failure",
    stepResults: [
      happyResults[0],
      happyResults[1],
      happyResults[2],
      happyResults[3],
      happyResults[4],
      member("project-operator", "agent-project-operator", "added"),
      member("backend-engineer", "agent-backend-engineer", "failed", issue("error", "member_add_failed", "Backend Engineer could not be added to the group.", true, {
        roleSlotId: "backend-engineer",
        agentId: "agent-backend-engineer",
      })),
      member("code-reviewer", "agent-reviewer-existing", "added"),
    ],
    issues: [
      issue("error", "member_add_failed", "One or more members could not be added.", true, {
        field: "members",
        roleSlotId: "backend-engineer",
      }),
    ],
    recoveryChoices: [
      choice("retry_member_add", "Retry add", "Try adding missing members again.", "adding_members", "backend-engineer"),
      choice("replace_agent", "Replace agent", "Select another agent for this role slot.", "validating", "backend-engineer"),
      choice("manual_invite", "Manual invite", "Keep the group and invite the missing member manually.", "posting_kickoff", "backend-engineer"),
      choice("cancel", "Cancel", "Cancel with the partially created group preserved unless cleanup is selected.", "canceled"),
    ],
    createdAgentIds: ["agent-project-operator", "agent-backend-engineer"],
    reusedAgentIds: ["agent-reviewer-existing"],
    createdGroupId: "conv-software-team",
    errorSummary: "Member add partially failed.",
  }),
  kickoffWarning: job({
    id: "job-kickoff-warning",
    status: "completed_with_warning",
    stepResults: [
      ...happyResults.slice(0, -1),
      {
        kind: "kickoff_post",
        stepId: "kickoff-post",
        groupConversationId: "conv-software-team",
        kickoffText: provisionedKickoffText,
        result: "warning",
        issue: issue("warning", "kickoff_post_delayed", "Group was created, but kickoff posting needs a retry.", true, {
          field: "kickoffText",
        }),
      },
    ],
    issues: [
      issue("warning", "kickoff_post_delayed", "Group is ready, but the kickoff message was not confirmed.", true, {
        field: "kickoffText",
      }),
    ],
    recoveryChoices: [
      choice("retry_kickoff", "Retry kickoff", "Try posting the kickoff message again.", "posting_kickoff"),
      choice("enter_group", "Enter group", "Enter the group and post manually if needed."),
    ],
    createdAgentIds: ["agent-project-operator", "agent-backend-engineer"],
    reusedAgentIds: ["agent-reviewer-existing"],
    createdGroupId: "conv-software-team",
    completedAt: DONE,
  }),
  cancellationBeforeSideEffects: job({
    id: "job-cancel-before-side-effects",
    status: "canceled",
    progressSteps: [
      step("validate", "validation", "Validate launch plan", "canceled", []),
      step("create-agents", "agent_creation", "Create or reuse agents", "pending", []),
      step("create-group", "group_creation", "Create group", "pending", []),
      step("add-members", "member_add", "Add members", "pending", []),
      step("post-kickoff", "kickoff_post", "Post kickoff", "pending", []),
      step("cleanup", "cleanup", "Cleanup generated assets", "succeeded", []),
    ],
    stepResults: [
      {
        kind: "cleanup",
        stepId: "cleanup",
        result: "none_needed",
        softDeletedAgentIds: [],
        preservedAgentIds: [],
      },
    ],
    recoveryChoices: [],
    errorSummary: "User canceled before validation created side effects.",
    canceledAt: DONE,
  }),
  cancellationAfterSideEffects: job({
    id: "job-cancel-after-side-effects",
    status: "canceled",
    progressSteps: [
      step("validate", "validation", "Validate launch plan", "succeeded", []),
      step("create-agents", "agent_creation", "Create or reuse agents", "succeeded", []),
      step("create-group", "group_creation", "Create group", "pending", []),
      step("add-members", "member_add", "Add members", "pending", []),
      step("post-kickoff", "kickoff_post", "Post kickoff", "pending", []),
      step("cleanup", "cleanup", "Cleanup generated assets", "succeeded", []),
    ],
    stepResults: [
      happyResults[0],
      happyResults[1],
      {
        kind: "cleanup",
        stepId: "cleanup",
        result: "soft_deleted_generated_agents",
        softDeletedAgentIds: ["agent-project-operator", "agent-backend-engineer"],
        preservedAgentIds: ["agent-reviewer-existing"],
      },
      {
        kind: "residual_assets",
        stepId: "residual-assets",
        generatedAgentIds: ["agent-project-operator", "agent-backend-engineer"],
        reusedAgentIds: ["agent-reviewer-existing"],
        customWorkspacePathsPreserved: ["/Users/alice/projects/choruz"],
        note: "Generated agents were disabled. Reused agents and user-provided workspace paths were preserved.",
      },
    ],
    createdAgentIds: ["agent-project-operator", "agent-backend-engineer"],
    reusedAgentIds: ["agent-reviewer-existing"],
    recoveryChoices: [],
    errorSummary: "User canceled after generated agents were created.",
    canceledAt: DONE,
  }),
} satisfies Record<string, GroupProvisioningJobContract>;

export const groupProvisioningContractFixtureList = Object.values(groupProvisioningContractFixtures);

function job(input: {
  id: string;
  status: GroupProvisioningJobStatus;
  plan?: GroupLaunchPlanContract;
  stepResults?: GroupProvisioningStepResult[];
  issues?: GroupProvisioningIssue[];
  recoveryChoices?: RecoveryChoice[];
  progressSteps?: ProgressStep[];
  createdAgentIds?: string[];
  reusedAgentIds?: string[];
  createdGroupId?: string;
  errorSummary?: string;
  completedAt?: string;
  canceledAt?: string;
}): GroupProvisioningJobContract {
  const statusContract = GROUP_PROVISIONING_STATUS_CONTRACT[input.status];
  return {
    id: input.id,
    status: input.status,
    companyId: "company-1",
    requestedBy: "human-1",
    idempotencyKey: `idem-${input.id}`,
    plan: input.plan ?? basePlan,
    progressSteps: input.progressSteps ?? progressFor(input.status, input.issues ?? []),
    stepResults: input.stepResults ?? [],
    issues: input.issues ?? [],
    recoveryChoices: input.recoveryChoices ?? [],
    allowedUiActions: statusContract.uiActions,
    allowedBackendActions: statusContract.backendActions,
    createdAgentIds: input.createdAgentIds ?? [],
    reusedAgentIds: input.reusedAgentIds ?? [],
    createdGroupId: input.createdGroupId,
    errorSummary: input.errorSummary,
    createdAt: NOW,
    updatedAt: input.completedAt ?? input.canceledAt ?? DONE,
    completedAt: input.completedAt,
    canceledAt: input.canceledAt,
  };
}

function progressFor(status: GroupProvisioningJobStatus, issues: GroupProvisioningIssue[]): ProgressStep[] {
  return [
    step("validate", "validation", "Validate launch plan", status === "failed_validation" ? "failed" : "succeeded", issues),
    step("create-agents", "agent_creation", "Create or reuse agents", ["validating", "failed_validation"].includes(status) ? "pending" : status === "failed" || status === "partial_failure" ? "failed" : "succeeded", issues),
    step("create-group", "group_creation", "Create group", ["completed", "completed_with_warning", "partial_failure", "canceled"].includes(status) ? "succeeded" : "pending", issues),
    step("add-members", "member_add", "Add members", status === "partial_failure" ? "failed" : ["completed", "completed_with_warning"].includes(status) ? "succeeded" : "pending", issues),
    step("post-kickoff", "kickoff_post", "Post kickoff", status === "completed_with_warning" ? "warning" : status === "completed" ? "succeeded" : "pending", issues),
  ];
}

function step(
  id: string,
  kind: ProgressStep["kind"],
  label: string,
  status: ProgressStep["status"],
  issues: GroupProvisioningIssue[],
): ProgressStep {
  return {
    id,
    kind,
    label,
    status,
    issues: status === "failed" || status === "warning" ? issues : undefined,
  };
}

function member(
  roleSlotId: string,
  agentId: string,
  result: "added" | "failed" | "skipped",
  issueValue?: GroupProvisioningIssue,
): GroupProvisioningStepResult {
  return {
    kind: "member_add",
    stepId: `member-${roleSlotId}`,
    roleSlotId,
    agentId,
    result,
    issue: issueValue,
  };
}

function issue(
  severity: GroupProvisioningIssue["severity"],
  code: string,
  message: string,
  recoverable: boolean,
  anchor: Omit<GroupProvisioningIssue, "severity" | "code" | "message" | "recoverable"> = {},
): GroupProvisioningIssue {
  return { severity, code, message, recoverable, ...anchor };
}

function choice(
  id: RecoveryChoice["id"],
  label: string,
  description: string,
  nextStatus?: GroupProvisioningJobStatus,
  roleSlotId?: string,
): RecoveryChoice {
  return { id, label, description, nextStatus, roleSlotId };
}
