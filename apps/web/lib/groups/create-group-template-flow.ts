import type {
  ContractRoleSlotPlan,
  GroupLaunchPlanContract,
  GroupLaunchPlanWorkflow,
  GroupProvisioningJobContract,
  ProvisioningJobCancelRequest,
  ProvisioningJobCreationRequest,
  ProvisioningJobRetryRequest,
} from "./group-provisioning-contract";
import type { Principal, RuntimeBindingInfo } from "../api/choruz-types";
import {
  generateGroupName,
  getRoleTemplate,
  DRIVER_IDS,
  GROUP_TEMPLATES,
  type DriverId,
  type GroupTemplate,
  type RoleSlot,
  type RoleTemplate,
  type SetupInputValues,
} from "./team-templates";
import { renderGroupKickoff, renderRoleInstructions } from "./team-template-renderer";
import type { ClientDriverAvailabilityItem } from "../agents/create-agent-template-flow";
import { driverDisplayName } from "../drivers/driver-registry";
import type { DriverModel } from "../drivers/driver-models";

export type GroupTemplateRoleAction = "create" | "reuse" | "skip";
export type GroupTemplateWorkspaceMode = "generated" | "custom" | "none";

export type GroupTemplateRoleDraft = {
  slotId: string;
  action: GroupTemplateRoleAction;
  roleTemplateId: string;
  roleTemplateVersion: string;
  required: boolean;
  agentName: string;
  driver: DriverId;
  harnessAccountId: string;
  harnessAccountName: string;
  harnessAccountModels: DriverModel[];
  model: string;
  setupInputs: SetupInputValues;
  selectedSkills: string[];
  workspaceMode: GroupTemplateWorkspaceMode;
  workspacePath: string;
  existingAgentId: string;
  instructionStatus: "template_default" | "customized" | "group_context_added";
  webhookUrl: string;
  webhookSecret: string;
};

export type GroupTemplateDraft = {
  groupTemplateId: string;
  groupTemplateVersion: string;
  groupName: string;
  mission: string;
  groupDefaultDriver: DriverId;
  kickoffText: string;
  roleDrafts: GroupTemplateRoleDraft[];
};

export type GroupTemplateIssue = {
  severity: "error" | "warning";
  code: string;
  message: string;
  field?: string;
  roleSlotId?: string;
};

export type GroupReviewItem = {
  label: string;
  value: string;
};

export type GroupProvisioningJobResponse = {
  job: GroupProvisioningJobContract;
};

export function groupTemplateOptions(): GroupTemplate[] {
  return GROUP_TEMPLATES;
}

export function createGroupTemplateDraft(options: {
  groupTemplate: GroupTemplate;
  mission?: string;
  groupDefaultDriver?: DriverId;
}): GroupTemplateDraft {
  const mission = options.mission ?? "";
  const groupDefaultDriver = options.groupDefaultDriver ?? options.groupTemplate.defaultDriverPolicy.driver;
  return {
    groupTemplateId: options.groupTemplate.id,
    groupTemplateVersion: options.groupTemplate.version,
    groupName: generateGroupName(options.groupTemplate, mission),
    mission,
    groupDefaultDriver,
    kickoffText: renderGroupKickoff(options.groupTemplate, mission),
    roleDrafts: options.groupTemplate.roleSlots.map((slot) =>
      createRoleDraftForSlot({
        groupTemplate: options.groupTemplate,
        roleSlot: slot,
        mission,
        groupDefaultDriver,
      })
    ),
  };
}

export function createRoleDraftForSlot(options: {
  groupTemplate: GroupTemplate;
  roleSlot: RoleSlot;
  mission: string;
  groupDefaultDriver: DriverId;
}): GroupTemplateRoleDraft {
  const roleTemplate = requireRoleTemplate(options.roleSlot.roleTemplateId);
  const setupInputs = defaultSetupInputValues(roleTemplate);
  const rendered = renderRoleInstructions({
    roleTemplate,
    roleSlot: options.roleSlot,
    setupInputs,
    groupMission: options.mission,
    outputExpectations: roleTemplate.outputContract.summary,
  });
  const driver = resolveGroupRoleDriver(options.groupTemplate, roleTemplate, options.groupDefaultDriver);

  return {
    slotId: options.roleSlot.id,
    action: options.roleSlot.required ? "create" : "skip",
    roleTemplateId: roleTemplate.id,
    roleTemplateVersion: roleTemplate.version,
    required: options.roleSlot.required,
    agentName: options.roleSlot.defaultAgentName,
    driver,
    harnessAccountId: "",
    harnessAccountName: "",
    harnessAccountModels: [],
    model: "",
    setupInputs,
    selectedSkills: [],
    workspaceMode: "generated",
    workspacePath: "",
    existingAgentId: "",
    instructionStatus: rendered.status,
    webhookUrl: "",
    webhookSecret: "",
  };
}

export function updateDraftMission(draft: GroupTemplateDraft, mission: string): GroupTemplateDraft {
  const template = requireGroupTemplate(draft.groupTemplateId);
  return {
    ...draft,
    mission,
    groupName: generateGroupName(template, mission),
    kickoffText: renderGroupKickoff(template, mission),
    roleDrafts: draft.roleDrafts.map((roleDraft) => ({
      ...roleDraft,
      setupInputs: {
        ...roleDraft.setupInputs,
        ...(roleDraft.setupInputs.mission !== undefined ? { mission } : {}),
      },
      instructionStatus: "group_context_added",
    })),
  };
}

export function applyGroupDefaultDriver(draft: GroupTemplateDraft, driver: DriverId): GroupTemplateDraft {
  return {
    ...draft,
    groupDefaultDriver: driver,
    roleDrafts: draft.roleDrafts.map((roleDraft) => {
      const roleTemplate = getRoleTemplate(roleDraft.roleTemplateId);
      if (!roleTemplate || !roleTemplate.compatibleDrivers.includes(driver)) return roleDraft;
      if (roleDraft.action !== "create") return roleDraft;
      return {
        ...roleDraft,
        driver,
        harnessAccountId: "",
        harnessAccountName: "",
        harnessAccountModels: [],
        model: "",
      };
    }),
  };
}

export function groupTemplateIssues(options: {
  draft: GroupTemplateDraft;
  existingAgents: Principal[];
  runtimeBindings: RuntimeBindingInfo[];
  availability?: ClientDriverAvailabilityItem[];
}): GroupTemplateIssue[] {
  const issues: GroupTemplateIssue[] = [];
  const template = requireGroupTemplate(options.draft.groupTemplateId);
  if (!options.draft.groupName.trim()) {
    issues.push(issue("error", "missing_group_name", "Group name is required.", "groupName"));
  }
  if (!options.draft.mission.trim()) {
    issues.push(issue("error", "missing_group_mission", "Mission is required.", "mission"));
  }

  const reused = new Map<string, string>();
  for (const slot of template.roleSlots) {
    const draft = options.draft.roleDrafts.find((candidate) => candidate.slotId === slot.id);
    if (!draft) {
      issues.push(issue("error", "missing_slot_plan", `${slot.label} needs a plan.`, "roleDrafts", slot.id));
      continue;
    }
    const roleTemplate = getRoleTemplate(draft.roleTemplateId);
    if (!roleTemplate) {
      issues.push(issue("error", "unknown_role_template", `Unknown role template ${draft.roleTemplateId}.`, "roleTemplateId", slot.id));
      continue;
    }
    if (draft.action === "skip") {
      issues.push(issue(slot.required ? "error" : "warning", slot.required ? "required_slot_skipped" : "optional_slot_skipped", `${slot.label} will be skipped.`, "action", slot.id));
      continue;
    }
    if (draft.action === "reuse") {
      if (!draft.existingAgentId) {
        issues.push(issue("error", "missing_existing_agent", `Choose an agent for ${slot.label}.`, "existingAgentId", slot.id));
        continue;
      }
      const firstSlot = reused.get(draft.existingAgentId);
      if (firstSlot && firstSlot !== slot.id) {
        issues.push(issue("error", "duplicate_existing_agent_assignment", "An existing agent cannot fill multiple role slots.", "existingAgentId", slot.id));
      } else {
        reused.set(draft.existingAgentId, slot.id);
      }
      const agent = options.existingAgents.find((candidate) => candidate.id === draft.existingAgentId);
      const binding = options.runtimeBindings.find((candidate) => candidate.agent_principal_id === draft.existingAgentId);
      if (!agent) issues.push(issue("error", "unknown_existing_agent", "Existing agent was not found.", "existingAgentId", slot.id));
      else if (agent.disabled) issues.push(issue("warning", "existing_agent_disabled", `${agent.name} is disabled.`, "existingAgentId", slot.id));
      if (binding && !roleTemplate.compatibleDrivers.includes(binding.driver_type as DriverId)) {
        issues.push(issue("warning", "reuse_driver_mismatch", `${driverDisplayName(binding.driver_type)} differs from the role recommendation.`, "existingAgentId", slot.id));
      }
      continue;
    }

    if (!draft.agentName.trim()) {
      issues.push(issue("error", "missing_agent_name", `${slot.label} needs an agent name.`, "agentName", slot.id));
    }
    const selectedDriver = options.availability?.find((item) => item.driverId === draft.driver);
    if (selectedDriver && !isDriverAvailable(selectedDriver)) {
      issues.push(issue("error", "driver_unavailable", selectedDriver.reason || `${driverDisplayName(draft.driver)} is unavailable on this machine.`, "driver", slot.id));
    }
    if (!roleTemplate.compatibleDrivers.includes(draft.driver)) {
      issues.push(issue("warning", "incompatible_driver", `${driverDisplayName(draft.driver)} is not compatible with ${roleTemplate.name}.`, "driver", slot.id));
    }
    for (const input of roleTemplate.setupInputs) {
      if (input.required && !(draft.setupInputs[input.id] ?? input.defaultValue ?? "").trim()) {
        issues.push(issue("error", "missing_required_setup_input", `${input.label} is required for ${roleTemplate.name}.`, input.id, slot.id));
      }
    }
    if (draft.workspaceMode === "custom" && !draft.workspacePath.trim()) {
      issues.push(issue("error", "missing_workspace_path", "Custom workspace path is required.", "workspacePath", slot.id));
    }
    if (draft.driver === "webhook_agent" && !draft.webhookUrl.trim()) {
      issues.push(issue("error", "missing_webhook_url", "Webhook URL is required for webhook agents.", "webhookUrl", slot.id));
    } else if (draft.driver === "webhook_agent") {
      try {
        const url = new URL(draft.webhookUrl.trim());
        if (url.protocol !== "http:" && url.protocol !== "https:") {
          issues.push(issue("error", "invalid_webhook_url", "Webhook URL must use http or https.", "webhookUrl", slot.id));
        }
      } catch {
        issues.push(issue("error", "invalid_webhook_url", "Webhook URL must be a valid URL.", "webhookUrl", slot.id));
      }
    }
    if (draft.driver === "webhook_agent" && draft.webhookSecret.trim().length > 0 && draft.webhookSecret.trim().length < 16) {
      issues.push(issue("error", "webhook_secret_too_short", "Webhook secret must be at least 16 characters when provided.", "webhookSecret", slot.id));
    }
    if (draft.driver === "webhook_agent" && draft.webhookSecret.trim().length > 0) {
      issues.push(issue("error", "webhook_secret_not_supported", "Group webhook secrets cannot be stored in launch plans. Leave blank to generate one.", "webhookSecret", slot.id));
    }
  }

  return issues;
}

export function blockingGroupTemplateIssues(options: {
  draft: GroupTemplateDraft;
  existingAgents: Principal[];
  runtimeBindings: RuntimeBindingInfo[];
  availability?: ClientDriverAvailabilityItem[];
}): GroupTemplateIssue[] {
  return groupTemplateIssues(options).filter((item) => item.severity === "error");
}

export function buildGroupLaunchPlan(draft: GroupTemplateDraft): GroupLaunchPlanContract {
  return {
    groupTemplateId: draft.groupTemplateId,
    groupTemplateVersion: draft.groupTemplateVersion as `${number}.${number}.${number}`,
    groupName: draft.groupName.trim(),
    mission: draft.mission.trim(),
    workflow: workflowForGroupTemplate(requireGroupTemplate(draft.groupTemplateId), draft.roleDrafts),
    kickoffText: draft.kickoffText,
    startWorkMode: "manual",
    rolePlans: draft.roleDrafts.map(roleDraftToPlan),
  };
}

export function buildProvisioningJobCreationRequest(
  draft: GroupTemplateDraft,
  companyId: string,
): ProvisioningJobCreationRequest {
  return {
    idempotencyKey: `group-launch:${draft.groupTemplateId}:${cryptoRandomId()}`,
    companyId,
    plan: buildGroupLaunchPlan(draft),
  };
}

export function buildGroupReviewItems(draft: GroupTemplateDraft): GroupReviewItem[] {
  const created = draft.roleDrafts.filter((role) => role.action === "create");
  const reused = draft.roleDrafts.filter((role) => role.action === "reuse");
  const skipped = draft.roleDrafts.filter((role) => role.action === "skip");
  const template = requireGroupTemplate(draft.groupTemplateId);
  return [
    { label: "Template", value: template.name },
    { label: "Group", value: draft.groupName.trim() || "Unnamed group" },
    { label: "Mission", value: draft.mission.trim() || "Not set" },
    { label: "Workflow", value: template.workflow.description },
    { label: "Coordinator", value: coordinatorLabel(template) },
    { label: "New agents", value: created.length ? created.map((role) => role.agentName).join(", ") : "None" },
    { label: "Reused agents", value: reused.length ? reused.map((role) => role.existingAgentId).join(", ") : "None" },
    { label: "Skipped roles", value: skipped.length ? skipped.map((role) => role.slotId).join(", ") : "None" },
    { label: "Drivers", value: summarizeDrivers(created) },
    {
      label: "Harness accounts",
      value: created.length
        ? created.map((role) => `${role.agentName}: ${role.harnessAccountName || "Not required"}`).join(", ")
        : "None",
    },
    { label: "Workspace behavior", value: summarizeWorkspace(created) },
    { label: "Kickoff behavior", value: "Posts a passive kickoff only; work waits for the user's first task." },
  ];
}

export function isGroupProvisioningTerminal(job: GroupProvisioningJobContract): boolean {
  return ["completed", "completed_with_warning", "failed", "failed_validation", "partial_failure", "rolled_back", "canceled"].includes(job.status);
}

export async function createGroupProvisioningJob(request: ProvisioningJobCreationRequest): Promise<GroupProvisioningJobContract> {
  const response = await groupProvisioningFetch<GroupProvisioningJobResponse>("/api/group-provisioning-jobs", {
    method: "POST",
    body: JSON.stringify(request),
  });
  return response.job;
}

export async function readGroupProvisioningJob(jobId: string): Promise<GroupProvisioningJobContract> {
  const response = await groupProvisioningFetch<GroupProvisioningJobResponse>(`/api/group-provisioning-jobs/${encodeURIComponent(jobId)}`);
  return response.job;
}

export async function runGroupProvisioningJob(jobId: string, maxSteps = 3): Promise<GroupProvisioningJobContract> {
  const response = await groupProvisioningFetch<GroupProvisioningJobResponse>(`/api/group-provisioning-jobs/${encodeURIComponent(jobId)}/run`, {
    method: "POST",
    body: JSON.stringify({ maxSteps }),
  });
  return response.job;
}

export async function retryGroupProvisioningJob(jobId: string, request: ProvisioningJobRetryRequest): Promise<GroupProvisioningJobContract> {
  const response = await groupProvisioningFetch<GroupProvisioningJobResponse>(`/api/group-provisioning-jobs/${encodeURIComponent(jobId)}/retry`, {
    method: "POST",
    body: JSON.stringify(request),
  });
  return response.job;
}

export async function cancelGroupProvisioningJob(jobId: string, request: ProvisioningJobCancelRequest): Promise<GroupProvisioningJobContract> {
  const response = await groupProvisioningFetch<GroupProvisioningJobResponse>(`/api/group-provisioning-jobs/${encodeURIComponent(jobId)}/cancel`, {
    method: "POST",
    body: JSON.stringify(request),
  });
  return response.job;
}

function roleDraftToPlan(role: GroupTemplateRoleDraft): ContractRoleSlotPlan {
  if (role.action === "reuse") {
    return {
      slotId: role.slotId,
      action: "reuse",
      existingAgentId: role.existingAgentId,
      roleTemplateId: role.roleTemplateId,
      roleTemplateVersion: role.roleTemplateVersion as `${number}.${number}.${number}`,
    };
  }
  if (role.action === "skip") {
    return {
      slotId: role.slotId,
      action: "skip",
      roleTemplateId: role.roleTemplateId,
      roleTemplateVersion: role.roleTemplateVersion as `${number}.${number}.${number}`,
      reason: role.required ? "user_choice" : "optional",
    };
  }
  return {
    slotId: role.slotId,
    action: "create",
    agentName: role.agentName,
    roleTemplateId: role.roleTemplateId,
    roleTemplateVersion: role.roleTemplateVersion as `${number}.${number}.${number}`,
    driver: role.driver,
    ...(role.harnessAccountId ? { harnessAccountId: role.harnessAccountId } : {}),
    ...(role.model.trim() ? { model: role.model.trim() } : {}),
    instructionStatus: role.instructionStatus,
    setupInputs: role.setupInputs,
    selectedSkills: role.selectedSkills,
    workspaceMode: role.workspaceMode,
    ...(role.workspaceMode === "custom" && role.workspacePath.trim()
      ? { workspacePath: role.workspacePath.trim() }
      : {}),
    ...(role.driver === "webhook_agent" && role.webhookUrl.trim()
      ? {
          webhookConfigured: true,
          webhookUrl: role.webhookUrl.trim(),
        }
      : {}),
  };
}

function resolveGroupRoleDriver(groupTemplate: GroupTemplate, roleTemplate: RoleTemplate, groupDefaultDriver: DriverId): DriverId {
  if (groupTemplate.defaultDriverPolicy.applyToCompatibleRoles && roleTemplate.compatibleDrivers.includes(groupDefaultDriver)) {
    return groupDefaultDriver;
  }
  return roleTemplate.recommendedDriver;
}

function defaultSetupInputValues(roleTemplate: RoleTemplate): SetupInputValues {
  return Object.fromEntries(
    roleTemplate.setupInputs.map((input) => [input.id, input.defaultValue ?? ""]),
  );
}

function summarizeDrivers(roles: GroupTemplateRoleDraft[]): string {
  if (roles.length === 0) return "None";
  const labels = new Set(roles.map((role) => driverDisplayName(role.driver)));
  return [...labels].join(", ");
}

function isDriverAvailable(item: ClientDriverAvailabilityItem): boolean {
  return item.available ?? item.status === "available";
}

function summarizeWorkspace(roles: GroupTemplateRoleDraft[]): string {
  if (roles.length === 0) return "None";
  const custom = roles.filter((role) => role.workspaceMode === "custom").length;
  const generated = roles.filter((role) => role.workspaceMode === "generated").length;
  const none = roles.filter((role) => role.workspaceMode === "none").length;
  return [
    generated ? `${generated} generated` : "",
    custom ? `${custom} custom` : "",
    none ? `${none} none` : "",
  ].filter(Boolean).join(", ");
}

function workflowForGroupTemplate(
  template: GroupTemplate,
  roleDrafts: GroupTemplateRoleDraft[],
): GroupLaunchPlanWorkflow {
  const assignableSlotIds = new Set(
    roleDrafts
      .filter((roleDraft) => roleDraft.action !== "skip")
      .map((roleDraft) => roleDraft.slotId),
  );
  return {
    ...(template.workflow.coordinatorRoleSlotId
      && assignableSlotIds.has(template.workflow.coordinatorRoleSlotId)
      ? { coordinatorRoleSlotId: template.workflow.coordinatorRoleSlotId }
      : {}),
    participantRoleDefaults: Object.fromEntries(
      template.roleSlots
        .filter((slot) => assignableSlotIds.has(slot.id))
        .map((slot) => [slot.id, slot.workflowRoleKeys ?? []]),
    ),
  };
}

function coordinatorLabel(template: GroupTemplate): string {
  const coordinatorSlotId = template.workflow.coordinatorRoleSlotId;
  if (!coordinatorSlotId) return "Not configured";
  const slot = template.roleSlots.find((candidate) => candidate.id === coordinatorSlotId);
  return slot ? slot.label : coordinatorSlotId;
}

function requireGroupTemplate(id: string): GroupTemplate {
  const template = GROUP_TEMPLATES.find((candidate) => candidate.id === id);
  if (!template) throw new Error(`Unknown group template ${id}`);
  return template;
}

function requireRoleTemplate(id: string): RoleTemplate {
  const template = getRoleTemplate(id);
  if (!template) throw new Error(`Unknown role template ${id}`);
  return template;
}

function issue(
  severity: GroupTemplateIssue["severity"],
  code: string,
  message: string,
  field?: string,
  roleSlotId?: string,
): GroupTemplateIssue {
  return {
    severity,
    code,
    message,
    ...(field ? { field } : {}),
    ...(roleSlotId ? { roleSlotId } : {}),
  };
}

function cryptoRandomId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) return crypto.randomUUID();
  return `${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

async function groupProvisioningFetch<T>(path: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers);
  if (init.body && !headers.has("content-type")) headers.set("content-type", "application/json");
  const response = await fetch(path, {
    ...init,
    headers,
    credentials: init.credentials ?? "same-origin",
  });
  if (!response.ok) {
    let detail = `${response.status} ${response.statusText}`;
    try {
      const body = (await response.json()) as { error?: string | { detail?: string } };
      if (typeof body.error === "string") detail = body.error;
      else if (body.error?.detail) detail = body.error.detail;
    } catch {
      // Keep the status text fallback.
    }
    throw new Error(detail);
  }
  return await response.json() as T;
}

export { DRIVER_IDS as GROUP_TEMPLATE_DRIVER_IDS };
