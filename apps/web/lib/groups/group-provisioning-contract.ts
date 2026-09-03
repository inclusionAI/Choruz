import {
  GROUP_PROVISIONING_JOB_STATUSES,
  GROUP_PROVISIONING_STATUS_TRANSITIONS,
  TERMINAL_GROUP_PROVISIONING_JOB_STATUSES,
  sanitizeProvisioningJson,
  type GroupProvisioningJobStatus,
  type JsonValue,
} from "./group-provisioning-store";
import { validateModelId } from "../drivers/driver-model-validation";
import {
  DRIVER_IDS,
  type DriverId,
  type InstructionStatus,
  type SetupInputValues,
  type TemplateVersion,
} from "./team-templates";

export type { GroupProvisioningJobStatus } from "./group-provisioning-store";

export type ContractIssueSeverity = "error" | "warning";
export type ContractIssueAnchor = {
  field?: string;
  roleSlotId?: string;
  roleTemplateId?: string;
  agentId?: string;
};

export type GroupProvisioningIssue = ContractIssueAnchor & {
  severity: ContractIssueSeverity;
  code: string;
  message: string;
  recoverable: boolean;
};

export type RoleTemplateAgentCreationMetadata = {
  mode: "role_template";
  roleTemplateId: string;
  roleTemplateVersion: TemplateVersion;
  instructionStatus: InstructionStatus;
  setupSummary: SetupInputValues;
  selectedSkills: string[];
  workspaceMode: "generated" | "custom" | "existing" | "none";
  originatingGroupProvisioningJobId?: string;
  originatingRoleSlotId?: string;
};

export type ContractRoleSlotPlan =
  | {
      slotId: string;
      action: "create";
      agentName: string;
      roleTemplateId: string;
      roleTemplateVersion: TemplateVersion;
      driver: DriverId;
      harnessAccountId?: string;
      model?: string;
      instructionStatus: InstructionStatus;
      setupInputs: SetupInputValues;
      selectedSkills: string[];
      workspaceMode: "generated" | "custom" | "none";
      workspacePath?: string;
      webhookConfigured?: boolean;
      webhookUrl?: string;
    }
  | {
      slotId: string;
      action: "reuse";
      existingAgentId: string;
      displayName?: string;
      roleTemplateId: string;
      roleTemplateVersion: TemplateVersion;
    }
  | {
      slotId: string;
      action: "skip";
      roleTemplateId: string;
      roleTemplateVersion: TemplateVersion;
      reason: "optional" | "user_choice" | "recovery_choice";
    };

export type GroupLaunchStartWorkMode = "manual";

export type GroupLaunchPlanWorkflow = {
  coordinatorRoleSlotId?: string;
  participantRoleDefaults: Record<string, string[]>;
};

export type GroupLaunchPlanContract = {
  groupTemplateId: string;
  groupTemplateVersion: TemplateVersion;
  groupName: string;
  mission: string;
  workflow?: GroupLaunchPlanWorkflow;
  rolePlans: ContractRoleSlotPlan[];
  kickoffText: string;
  startWorkMode?: GroupLaunchStartWorkMode;
};

export type GroupLaunchPlanValidationRequest = {
  plan: GroupLaunchPlanContract;
};

export type GroupLaunchPlanValidationResponse = {
  valid: boolean;
  issues: GroupProvisioningIssue[];
  plan: GroupLaunchPlanContract;
  allowedActions: readonly GroupProvisioningUiAction[];
};

export type ProvisioningJobCreationRequest = {
  idempotencyKey: string;
  companyId: string;
  plan: GroupLaunchPlanContract;
};

export type ProvisioningJobCreationResponse = {
  job: GroupProvisioningJobContract;
};

export type ProvisioningJobReadResponse = {
  job: GroupProvisioningJobContract;
};

export type ProvisioningJobRunRequest = {
  maxSteps?: number;
};

export type ProvisioningJobRunResponse = {
  job: GroupProvisioningJobContract;
};

export type ProvisioningJobRetryRequest = {
  choice: RecoveryChoiceId;
  roleSlotId?: string;
  nextStatus?: GroupProvisioningJobStatus;
};

export type ProvisioningJobRetryResponse = {
  job: GroupProvisioningJobContract;
};

export type RecoveryChoiceRequest =
  | {
      choice: "edit_plan" | "retry_validation";
      editedPlan?: GroupLaunchPlanContract;
    }
  | {
      choice: "retry_agent_creation" | "skip_optional_role" | "retry_member_add" | "replace_agent";
      roleSlotId: string;
      replacementAgentId?: string;
      editedRolePlan?: ContractRoleSlotPlan;
    }
  | {
      choice: "retry_group_creation" | "retry_kickoff" | "enter_group" | "manual_invite";
    }
  | {
      choice: "soft_delete_generated_agents";
      generatedAgentIds: string[];
      preserveReusedAgents: true;
    };

export type RecoveryChoiceResponse = {
  job: GroupProvisioningJobContract;
};

export type ProvisioningJobCancelRequest = {
  choice: "cancel_only" | "soft_delete_generated_agents";
  reason?: string;
};

export type ProvisioningJobCancelResponse = {
  job: GroupProvisioningJobContract;
};

export type RecoveryChoiceId =
  | "edit_plan"
  | "retry_validation"
  | "retry_agent_creation"
  | "skip_optional_role"
  | "retry_group_creation"
  | "soft_delete_generated_agents"
  | "retry_member_add"
  | "replace_agent"
  | "manual_invite"
  | "retry_kickoff"
  | "enter_group"
  | "cancel";

export type RecoveryChoice = {
  id: RecoveryChoiceId;
  label: string;
  description: string;
  roleSlotId?: string;
  destructive?: boolean;
  nextStatus?: GroupProvisioningJobStatus;
};

export type GroupProvisioningUiAction =
  | "edit_plan"
  | "create_job"
  | "read_job"
  | "run"
  | "retry"
  | "cancel"
  | "choose_recovery"
  | "enter_group";

export type GroupProvisioningBackendAction =
  | "validate_plan"
  | "create_job"
  | "read_job"
  | "advance_job"
  | "prepare_retry"
  | "cancel_job"
  | "record_recovery_choice"
  | "none";

export type GroupProvisioningStatusContract = {
  status: GroupProvisioningJobStatus;
  terminal: boolean;
  nextStatuses: readonly GroupProvisioningJobStatus[];
  uiActions: readonly GroupProvisioningUiAction[];
  backendActions: readonly GroupProvisioningBackendAction[];
};

export const GROUP_PROVISIONING_STATUS_CONTRACT: Record<GroupProvisioningJobStatus, GroupProvisioningStatusContract> = {
  validating: statusContract("validating", ["read_job", "run", "cancel"], ["read_job", "advance_job", "cancel_job"]),
  creating_agents: statusContract("creating_agents", ["read_job", "run", "cancel"], ["read_job", "advance_job", "cancel_job"]),
  creating_group: statusContract("creating_group", ["read_job", "run", "cancel"], ["read_job", "advance_job", "cancel_job"]),
  adding_members: statusContract("adding_members", ["read_job", "run", "cancel"], ["read_job", "advance_job", "cancel_job"]),
  posting_kickoff: statusContract("posting_kickoff", ["read_job", "run", "cancel"], ["read_job", "advance_job", "cancel_job"]),
  completed: statusContract("completed", ["read_job", "enter_group"], ["read_job", "none"]),
  completed_with_warning: statusContract("completed_with_warning", ["read_job", "retry", "enter_group"], ["read_job", "prepare_retry"]),
  partial_failure: statusContract("partial_failure", ["read_job", "choose_recovery", "retry", "cancel"], ["read_job", "record_recovery_choice", "prepare_retry", "cancel_job"]),
  failed_validation: statusContract("failed_validation", ["edit_plan", "read_job", "retry", "cancel"], ["validate_plan", "read_job", "prepare_retry", "cancel_job"]),
  failed: statusContract("failed", ["read_job", "choose_recovery", "retry", "cancel"], ["read_job", "record_recovery_choice", "prepare_retry", "cancel_job"]),
  rolled_back: statusContract("rolled_back", ["read_job"], ["read_job", "none"]),
  canceled: statusContract("canceled", ["read_job"], ["read_job", "none"]),
};

export type ProgressStepKind =
  | "validation"
  | "agent_creation"
  | "group_creation"
  | "member_add"
  | "routing_policy_update"
  | "kickoff_post"
  | "cleanup";

export type ProgressStep = {
  id: string;
  kind: ProgressStepKind;
  label: string;
  status: "pending" | "running" | "succeeded" | "warning" | "failed" | "skipped" | "canceled";
  roleSlotId?: string;
  startedAt?: string;
  completedAt?: string;
  issues?: GroupProvisioningIssue[];
};

export type CreatedAgentStepResult = {
  kind: "created_agent";
  stepId: string;
  roleSlotId: string;
  agentId: string;
  agentName: string;
  driver: DriverId;
  roleTemplateId: string;
  roleTemplateVersion: TemplateVersion;
  instructionStatus: InstructionStatus;
  workspaceMode: "generated" | "custom" | "none";
};

export type ReusedAgentStepResult = {
  kind: "reused_agent";
  stepId: string;
  roleSlotId: string;
  agentId: string;
  agentName: string;
  roleTemplateId: string;
  roleTemplateVersion: TemplateVersion;
};

export type SkippedOptionalRoleStepResult = {
  kind: "skipped_optional_role";
  stepId: string;
  roleSlotId: string;
  roleTemplateId: string;
  reason: "user_choice" | "unavailable" | "recovery_choice";
};

export type CreatedGroupStepResult = {
  kind: "created_group";
  stepId: string;
  groupConversationId: string;
  groupName: string;
};

export type MemberAddStepResult = {
  kind: "member_add";
  stepId: string;
  roleSlotId: string;
  agentId: string;
  result: "added" | "failed" | "skipped";
  issue?: GroupProvisioningIssue;
};

export type RoutingPolicyUpdateStepResult = {
  kind: "routing_policy_update";
  stepId: string;
  groupConversationId: string;
  result: "enabled" | "failed" | "skipped";
  issue?: GroupProvisioningIssue;
};

export type KickoffPostStepResult = {
  kind: "kickoff_post";
  stepId: string;
  groupConversationId: string;
  messageId?: string;
  kickoffText: string;
  result: "posted" | "warning" | "failed" | "skipped";
  issue?: GroupProvisioningIssue;
};

export type CleanupStepResult = {
  kind: "cleanup";
  stepId: string;
  result: "none_needed" | "soft_deleted_generated_agents" | "failed";
  softDeletedAgentIds: string[];
  preservedAgentIds: string[];
  issue?: GroupProvisioningIssue;
};

export type ResidualAssetsStepResult = {
  kind: "residual_assets";
  stepId: string;
  groupConversationId?: string;
  generatedAgentIds: string[];
  reusedAgentIds: string[];
  customWorkspacePathsPreserved: string[];
  note: string;
};

export type GroupProvisioningStepResult =
  | CreatedAgentStepResult
  | ReusedAgentStepResult
  | SkippedOptionalRoleStepResult
  | CreatedGroupStepResult
  | MemberAddStepResult
  | RoutingPolicyUpdateStepResult
  | KickoffPostStepResult
  | CleanupStepResult
  | ResidualAssetsStepResult;

export type GroupProvisioningJobContract = {
  id: string;
  status: GroupProvisioningJobStatus;
  companyId: string;
  requestedBy: string;
  idempotencyKey: string;
  plan: GroupLaunchPlanContract;
  progressSteps: ProgressStep[];
  stepResults: GroupProvisioningStepResult[];
  issues: GroupProvisioningIssue[];
  recoveryChoices: RecoveryChoice[];
  allowedUiActions: readonly GroupProvisioningUiAction[];
  allowedBackendActions: readonly GroupProvisioningBackendAction[];
  createdAgentIds: string[];
  reusedAgentIds: string[];
  createdGroupId?: string;
  errorSummary?: string;
  createdAt: string;
  updatedAt: string;
  completedAt?: string;
  canceledAt?: string;
};

export function isGroupProvisioningJobStatus(value: unknown): value is GroupProvisioningJobStatus {
  return typeof value === "string" && GROUP_PROVISIONING_JOB_STATUSES.includes(value as GroupProvisioningJobStatus);
}

export function isGroupProvisioningJobContract(value: unknown): value is GroupProvisioningJobContract {
  if (!isRecord(value)) return false;
  if (!isGroupProvisioningJobStatus(value.status)) return false;
  if (!isNonEmptyString(value.id) || !isNonEmptyString(value.companyId) || !isNonEmptyString(value.requestedBy)) return false;
  if (!isNonEmptyString(value.idempotencyKey) || !isGroupLaunchPlanContract(value.plan)) return false;
  if (!isStringArray(value.createdAgentIds) || !isStringArray(value.reusedAgentIds)) return false;
  if (!Array.isArray(value.progressSteps) || !value.progressSteps.every(isProgressStep)) return false;
  if (!Array.isArray(value.stepResults) || !value.stepResults.every(isGroupProvisioningStepResult)) return false;
  if (!Array.isArray(value.issues) || !value.issues.every(isGroupProvisioningIssue)) return false;
  if (!Array.isArray(value.recoveryChoices) || !value.recoveryChoices.every(isRecoveryChoice)) return false;
  if (!isKnownActions(value.allowedUiActions, ALL_UI_ACTIONS)) return false;
  if (!isKnownActions(value.allowedBackendActions, ALL_BACKEND_ACTIONS)) return false;
  if (!isIsoLikeString(value.createdAt) || !isIsoLikeString(value.updatedAt)) return false;
  return statusActionsMatch(value.status, value.allowedUiActions, value.allowedBackendActions);
}

export function validateGroupProvisioningContractFixture(value: unknown): GroupProvisioningJobContract {
  if (!isGroupProvisioningJobContract(value)) {
    throw new Error("invalid group provisioning job contract fixture");
  }
  sanitizeProvisioningJson(value as unknown as JsonValue, "fixture");
  assertKickoffIsPassive(value.plan.kickoffText);
  for (const result of value.stepResults) {
    if (result.kind === "kickoff_post") assertKickoffIsPassive(result.kickoffText);
    if (result.kind === "cleanup" && result.result === "soft_deleted_generated_agents" && result.softDeletedAgentIds.length === 0) {
      throw new Error("cleanup fixture must name soft-deleted generated agents");
    }
  }
  return value;
}

export function assertEveryStatusHasExplicitActions(): void {
  for (const status of GROUP_PROVISIONING_JOB_STATUSES) {
    const contract = GROUP_PROVISIONING_STATUS_CONTRACT[status];
    if (!contract?.uiActions.length) throw new Error(`missing UI actions for ${status}`);
    if (!contract.backendActions.length) throw new Error(`missing backend actions for ${status}`);
  }
}

function statusContract(
  status: GroupProvisioningJobStatus,
  uiActions: readonly GroupProvisioningUiAction[],
  backendActions: readonly GroupProvisioningBackendAction[],
): GroupProvisioningStatusContract {
  return {
    status,
    terminal: TERMINAL_GROUP_PROVISIONING_JOB_STATUSES.includes(status as never),
    nextStatuses: GROUP_PROVISIONING_STATUS_TRANSITIONS[status],
    uiActions,
    backendActions,
  };
}

function assertKickoffIsPassive(kickoffText: string): void {
  if (/@[a-z0-9][\w.-]*/i.test(kickoffText)) throw new Error("kickoff text must not mention agents");
  if (/\b(route|routing|agent_commands|app_mention|auto[- ]?start|start work automatically)\b/i.test(kickoffText)) {
    throw new Error("kickoff text must not include routing or auto-start metadata");
  }
}

const INSTRUCTION_STATUSES = ["template_default", "customized", "group_context_added"] as const satisfies readonly InstructionStatus[];
const CREATE_WORKSPACE_MODES = ["generated", "custom", "none"] as const;
const ROLE_METADATA_WORKSPACE_MODES = ["generated", "custom", "existing", "none"] as const;
const ROLE_PLAN_ACTIONS = ["create", "reuse", "skip"] as const;
const SKIP_REASONS = ["optional", "user_choice", "recovery_choice"] as const;
const RECOVERY_CHOICE_IDS = [
  "edit_plan",
  "retry_validation",
  "retry_agent_creation",
  "skip_optional_role",
  "retry_group_creation",
  "soft_delete_generated_agents",
  "retry_member_add",
  "replace_agent",
  "manual_invite",
  "retry_kickoff",
  "enter_group",
  "cancel",
] as const satisfies readonly RecoveryChoiceId[];
const PROGRESS_STEP_KINDS = [
  "validation",
  "agent_creation",
  "group_creation",
  "member_add",
  "routing_policy_update",
  "kickoff_post",
  "cleanup",
] as const satisfies readonly ProgressStepKind[];
const PROGRESS_STEP_STATUSES = ["pending", "running", "succeeded", "warning", "failed", "skipped", "canceled"] as const;
const MEMBER_ADD_RESULTS = ["added", "failed", "skipped"] as const;
const ROUTING_POLICY_RESULTS = ["enabled", "failed", "skipped"] as const;
const KICKOFF_POST_RESULTS = ["posted", "warning", "failed", "skipped"] as const;
const CLEANUP_RESULTS = ["none_needed", "soft_deleted_generated_agents", "failed"] as const;
const OPTIONAL_SKIP_REASONS = ["user_choice", "unavailable", "recovery_choice"] as const;

function isGroupLaunchPlanContract(value: unknown): value is GroupLaunchPlanContract {
  if (!isRecord(value)) return false;
  return isNonEmptyString(value.groupTemplateId)
    && isNonEmptyString(value.groupTemplateVersion)
    && typeof value.groupName === "string"
    && typeof value.mission === "string"
    && typeof value.kickoffText === "string"
    && (value.workflow === undefined || isGroupLaunchPlanWorkflow(value.workflow))
    && (value.startWorkMode === undefined || value.startWorkMode === "manual")
    && Array.isArray(value.rolePlans)
    && value.rolePlans.every(isContractRoleSlotPlan);
}

function isGroupLaunchPlanWorkflow(value: unknown): value is GroupLaunchPlanWorkflow {
  if (!isRecord(value) || !isRecord(value.participantRoleDefaults)) return false;
  if (value.coordinatorRoleSlotId !== undefined && !isNonEmptyString(value.coordinatorRoleSlotId)) return false;
  return Object.values(value.participantRoleDefaults).every(isStringArray);
}

function isContractRoleSlotPlan(value: unknown): value is ContractRoleSlotPlan {
  if (!isRecord(value) || !isNonEmptyString(value.slotId) || !isNonEmptyString(value.roleTemplateId)) return false;
  if (!isNonEmptyString(value.roleTemplateVersion) || !isOneOf(value.action, ROLE_PLAN_ACTIONS)) return false;
  if (value.action === "create") {
    if ("webhookSecret" in value || "webhook_secret" in value) return false;
    return isNonEmptyString(value.agentName)
      && isOneOf(value.driver, DRIVER_IDS)
      && (value.harnessAccountId === undefined || isNonEmptyString(value.harnessAccountId))
      && validateModelId(value.model) === null
      && !(value.driver === "webhook_agent" && typeof value.model === "string" && value.model.trim())
      && isOneOf(value.instructionStatus, INSTRUCTION_STATUSES)
      && isRecord(value.setupInputs)
      && isStringArray(value.selectedSkills)
      && isOneOf(value.workspaceMode, CREATE_WORKSPACE_MODES)
      && (value.webhookConfigured === undefined || typeof value.webhookConfigured === "boolean")
      && (value.webhookUrl === undefined || typeof value.webhookUrl === "string");
  }
  if (value.action === "reuse") {
    return isNonEmptyString(value.existingAgentId)
      && (value.displayName === undefined || typeof value.displayName === "string");
  }
  if (value.action === "skip") return isOneOf(value.reason, SKIP_REASONS);
  return false;
}

function isProgressStep(value: unknown): value is ProgressStep {
  if (!isRecord(value)) return false;
  if (!isNonEmptyString(value.id) || !isOneOf(value.kind, PROGRESS_STEP_KINDS) || !isNonEmptyString(value.label)) return false;
  if (!isOneOf(value.status, PROGRESS_STEP_STATUSES)) return false;
  return value.issues === undefined || (Array.isArray(value.issues) && value.issues.every(isGroupProvisioningIssue));
}

function isGroupProvisioningIssue(value: unknown): value is GroupProvisioningIssue {
  if (!isRecord(value)) return false;
  return (value.severity === "error" || value.severity === "warning")
    && isNonEmptyString(value.code)
    && isNonEmptyString(value.message)
    && typeof value.recoverable === "boolean";
}

function isRecoveryChoice(value: unknown): value is RecoveryChoice {
  if (!isRecord(value)) return false;
  return isOneOf(value.id, RECOVERY_CHOICE_IDS)
    && isNonEmptyString(value.label)
    && isNonEmptyString(value.description)
    && (value.nextStatus === undefined || isGroupProvisioningJobStatus(value.nextStatus));
}

function isGroupProvisioningStepResult(value: unknown): value is GroupProvisioningStepResult {
  if (!isRecord(value) || !isNonEmptyString(value.kind) || !isNonEmptyString(value.stepId)) return false;
  switch (value.kind) {
    case "created_agent":
      return isNonEmptyString(value.roleSlotId)
        && isNonEmptyString(value.agentId)
        && isNonEmptyString(value.agentName)
        && isOneOf(value.driver, DRIVER_IDS)
        && isOneOf(value.instructionStatus, INSTRUCTION_STATUSES)
        && isOneOf(value.workspaceMode, CREATE_WORKSPACE_MODES);
    case "reused_agent":
      return isNonEmptyString(value.roleSlotId) && isNonEmptyString(value.agentId) && isNonEmptyString(value.agentName);
    case "skipped_optional_role":
      return isNonEmptyString(value.roleSlotId) && isNonEmptyString(value.roleTemplateId) && isOneOf(value.reason, OPTIONAL_SKIP_REASONS);
    case "created_group":
      return isNonEmptyString(value.groupConversationId) && isNonEmptyString(value.groupName);
    case "member_add":
      return isNonEmptyString(value.roleSlotId) && isNonEmptyString(value.agentId) && isOneOf(value.result, MEMBER_ADD_RESULTS);
    case "routing_policy_update":
      return isNonEmptyString(value.groupConversationId) && isOneOf(value.result, ROUTING_POLICY_RESULTS);
    case "kickoff_post":
      return isNonEmptyString(value.groupConversationId) && isNonEmptyString(value.kickoffText) && isOneOf(value.result, KICKOFF_POST_RESULTS);
    case "cleanup":
      return isOneOf(value.result, CLEANUP_RESULTS) && isStringArray(value.softDeletedAgentIds) && isStringArray(value.preservedAgentIds);
    case "residual_assets":
      return isStringArray(value.generatedAgentIds) && isStringArray(value.reusedAgentIds) && isStringArray(value.customWorkspacePathsPreserved);
    default:
      return false;
  }
}

export function isRecoveryChoiceRequest(value: unknown): value is RecoveryChoiceRequest {
  if (!isRecord(value) || !isOneOf(value.choice, RECOVERY_CHOICE_IDS)) return false;
  switch (value.choice) {
    case "edit_plan":
    case "retry_validation":
      return value.editedPlan === undefined || isGroupLaunchPlanContract(value.editedPlan);
    case "retry_agent_creation":
    case "skip_optional_role":
    case "retry_member_add":
    case "replace_agent":
      return isNonEmptyString(value.roleSlotId)
        && (value.replacementAgentId === undefined || isNonEmptyString(value.replacementAgentId))
        && (value.editedRolePlan === undefined || isContractRoleSlotPlan(value.editedRolePlan));
    case "retry_group_creation":
    case "retry_kickoff":
    case "enter_group":
    case "manual_invite":
      return true;
    case "soft_delete_generated_agents":
      return isStringArray(value.generatedAgentIds) && value.preserveReusedAgents === true;
    case "cancel":
      return false;
    default:
      return false;
  }
}

function statusActionsMatch(
  status: GroupProvisioningJobStatus,
  uiActions: readonly unknown[],
  backendActions: readonly unknown[],
): boolean {
  const expected = GROUP_PROVISIONING_STATUS_CONTRACT[status];
  return sameStringSet(uiActions, expected.uiActions) && sameStringSet(backendActions, expected.backendActions);
}

function sameStringSet(left: readonly unknown[], right: readonly string[]): boolean {
  return left.length === right.length && left.every((item) => typeof item === "string" && right.includes(item));
}

function isKnownActions(value: unknown, allowed: readonly string[]): value is readonly string[] {
  return Array.isArray(value) && value.every((item) => typeof item === "string" && allowed.includes(item));
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === "string");
}

function isOneOf<const T extends readonly string[]>(value: unknown, allowed: T): value is T[number] {
  return typeof value === "string" && allowed.includes(value);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

function isIsoLikeString(value: unknown): value is string {
  return typeof value === "string" && !Number.isNaN(Date.parse(value));
}

const ALL_UI_ACTIONS: readonly GroupProvisioningUiAction[] = [
  "edit_plan",
  "create_job",
  "read_job",
  "run",
  "retry",
  "cancel",
  "choose_recovery",
  "enter_group",
];

const ALL_BACKEND_ACTIONS: readonly GroupProvisioningBackendAction[] = [
  "validate_plan",
  "create_job",
  "read_job",
  "advance_job",
  "prepare_retry",
  "cancel_job",
  "record_recovery_choice",
  "none",
];
