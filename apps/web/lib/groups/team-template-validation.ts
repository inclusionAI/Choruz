import path from "node:path";

import type { DriverAvailabilityItem } from "../drivers/driver-availability";
import type { Principal, RuntimeBindingInfo } from "../api/choruz-types";
import {
  getGroupTemplate,
  getRoleTemplate,
  LOCAL_CODING_DRIVER_IDS,
  type DriverId,
  type GroupTemplate,
  type RoleSlot,
  type RoleTemplate,
  type SetupInputValues,
  type TemplateVersion,
} from "./team-templates";

export type ValidationSeverity = "error" | "warning";

export type ValidationIssue = {
  severity: ValidationSeverity;
  code: string;
  message: string;
  field?: string;
  slotId?: string;
};

export type ValidationResult = {
  valid: boolean;
  errors: ValidationIssue[];
  warnings: ValidationIssue[];
};

export type DriverResolutionSource =
  | "system_default"
  | "group_template_default"
  | "role_template_recommendation"
  | "user_override";

export type DriverResolution = {
  driver: DriverId;
  source: DriverResolutionSource;
  compatible: boolean;
  suggestedDriver?: DriverId;
  issues: ValidationIssue[];
};

export type ExistingAgentCandidate = {
  principal: Principal;
  runtimeBinding?: RuntimeBindingInfo | null;
  companyId: string;
  workspaceId?: string;
  activeJobIds?: string[];
};

export type RoleAgentCreationInput = {
  roleTemplateId: string;
  roleTemplateVersion?: TemplateVersion;
  agentName: string;
  setupInputs?: SetupInputValues;
  systemDefaultDriver?: DriverId;
  groupDefaultDriver?: DriverId;
  userOverrideDriver?: DriverId;
  driverAvailability?: DriverAvailabilityItem[];
  instructions?: string;
  workspacePath?: string;
  skillPaths?: string[];
  webhookUrl?: string;
  webhookSecret?: string;
  homeDir?: string;
};

export type RoleSlotPlan =
  | {
      slotId: string;
      action: "create";
      agentName: string;
      setupInputs?: SetupInputValues;
      userOverrideDriver?: DriverId;
      instructions?: string;
      workspacePath?: string;
      skillPaths?: string[];
      webhookUrl?: string;
      webhookSecret?: string;
    }
  | {
      slotId: string;
      action: "reuse";
      existingAgentId: string;
      activeConflictingJobIds?: string[];
    }
  | {
      slotId: string;
      action: "skip";
    };

export type GroupLaunchPlanInput = {
  groupTemplateId: string;
  groupTemplateVersion?: TemplateVersion;
  groupName: string;
  mission: string;
  rolePlans: RoleSlotPlan[];
  existingAgents?: ExistingAgentCandidate[];
  existingAgentNames?: string[];
  existingGroupNames?: string[];
  companyId: string;
  workspaceId: string;
  systemDefaultDriver?: DriverId;
  driverAvailability?: DriverAvailabilityItem[];
  homeDir?: string;
};

const LOCAL_CLI_DRIVERS = new Set<DriverId>(LOCAL_CODING_DRIVER_IDS);
const REUSABLE_RUNTIME_BINDING_STATES = new Set(["idle", "running"]);

export function validationResult(issues: ValidationIssue[]): ValidationResult {
  const errors = issues.filter((issue) => issue.severity === "error");
  const warnings = issues.filter((issue) => issue.severity === "warning");
  return { valid: errors.length === 0, errors, warnings };
}

export function resolveDriverForRole(options: {
  roleTemplate: RoleTemplate;
  systemDefaultDriver?: DriverId;
  groupDefaultDriver?: DriverId;
  userOverrideDriver?: DriverId;
  availability?: DriverAvailabilityItem[];
}): DriverResolution {
  const { roleTemplate } = options;
  const candidates: Array<{ driver?: DriverId; source: DriverResolutionSource }> = [
    { driver: options.systemDefaultDriver, source: "system_default" },
    { driver: options.groupDefaultDriver, source: "group_template_default" },
    { driver: roleTemplate.recommendedDriver, source: "role_template_recommendation" },
    { driver: options.userOverrideDriver, source: "user_override" },
  ];
  const selected = candidates.filter((candidate) => candidate.driver).at(-1);
  const driver = selected?.driver ?? roleTemplate.recommendedDriver;
  const source = selected?.source ?? "role_template_recommendation";
  const issues: ValidationIssue[] = [];
  const compatible = roleTemplate.compatibleDrivers.includes(driver);

  if (!compatible) {
    issues.push({
      severity: "warning",
      code: "incompatible_driver",
      field: "driver",
      message: `${driver} is not compatible with ${roleTemplate.name}.`,
    });
  }

  const suggestedDriver = compatible
    ? suggestCompatibleAvailableDriver(roleTemplate, driver, options.availability)
    : undefined;

  return { driver, source, compatible, suggestedDriver, issues };
}

export function suggestCompatibleAvailableDriver(
  roleTemplate: RoleTemplate,
  currentDriver: DriverId,
  availability: DriverAvailabilityItem[] = [],
): DriverId | undefined {
  if (isDriverAvailable(currentDriver, availability)) return undefined;
  return roleTemplate.compatibleDrivers.find((driver) => driver !== currentDriver && isDriverAvailable(driver, availability));
}

export function validateRoleAgentCreation(input: RoleAgentCreationInput): ValidationResult {
  const issues: ValidationIssue[] = [];
  const roleTemplate = getRoleTemplate(input.roleTemplateId);
  if (!roleTemplate) {
    return validationResult([
      issue("error", "unknown_role_template", `Unknown role template ${input.roleTemplateId}.`, "roleTemplateId"),
    ]);
  }
  if (input.roleTemplateVersion && input.roleTemplateVersion !== roleTemplate.version) {
    issues.push(issue("error", "role_template_version_mismatch", `Role template ${roleTemplate.id} version ${input.roleTemplateVersion} is not available.`, "roleTemplateVersion"));
  }

  const resolution = resolveDriverForRole({
    roleTemplate,
    systemDefaultDriver: input.systemDefaultDriver,
    groupDefaultDriver: input.groupDefaultDriver,
    userOverrideDriver: input.userOverrideDriver,
    availability: input.driverAvailability,
  });
  issues.push(...resolution.issues);

  addDriverAvailabilityIssue(issues, resolution.driver, input.driverAvailability);
  if (resolution.compatible) {
    if (resolution.suggestedDriver) {
      issues.push(issue("warning", "driver_fallback_available", `${resolution.driver} is unavailable. ${resolution.suggestedDriver} is the next compatible available driver.`, "driver"));
    }
  }

  validateAgentName(issues, input.agentName);
  validateSetupInputs(issues, roleTemplate, input.setupInputs ?? {});
  validateWorkspacePath(issues, input.workspacePath, input.homeDir);
  validateSkillPaths(issues, input.skillPaths ?? [], input.homeDir);
  validateWebhook(issues, resolution.driver, input.webhookUrl, input.webhookSecret);
  validateInstructionPresence(issues, resolution.driver, input.instructions);

  return validationResult(issues);
}

export function validateGroupLaunchPlan(input: GroupLaunchPlanInput): ValidationResult {
  const issues: ValidationIssue[] = [];
  const groupTemplate = getGroupTemplate(input.groupTemplateId);
  if (!groupTemplate) {
    return validationResult([
      issue("error", "unknown_group_template", `Unknown group template ${input.groupTemplateId}.`, "groupTemplateId"),
    ]);
  }
  if (input.groupTemplateVersion && input.groupTemplateVersion !== groupTemplate.version) {
    issues.push(issue("error", "group_template_version_mismatch", `Group template ${groupTemplate.id} version ${input.groupTemplateVersion} is not available.`, "groupTemplateVersion"));
  }

  validateGroupName(issues, input.groupName, input.existingGroupNames ?? []);
  if (!input.mission.trim()) {
    issues.push(issue("error", "missing_group_mission", "Group mission is required.", "mission"));
  }

  const plansBySlot = new Map(input.rolePlans.map((plan) => [plan.slotId, plan]));
  const seenReuse = new Set<string>();
  const existingAgentNames = new Set((input.existingAgentNames ?? []).map(normalizeName));
  const seenCreateNames = new Map<string, string>();

  for (const plan of input.rolePlans) {
    if (input.rolePlans.filter((candidate) => candidate.slotId === plan.slotId).length > 1) {
      issues.push(slotIssue("error", "duplicate_role_slot_plan", `Role slot ${plan.slotId} was submitted more than once.`, plan.slotId));
    }
    if (plan.action === "create") {
      const normalizedName = normalizeName(plan.agentName);
      const firstSlotId = seenCreateNames.get(normalizedName);
      if (normalizedName && firstSlotId && firstSlotId !== plan.slotId) {
        issues.push(slotIssue("error", "duplicate_created_agent_name", `Agent name ${plan.agentName} is used by multiple new agents in this launch.`, plan.slotId, "agentName"));
      } else if (normalizedName) {
        seenCreateNames.set(normalizedName, plan.slotId);
      }
    }
  }

  for (const slot of groupTemplate.roleSlots) {
    const roleTemplate = getRoleTemplate(slot.roleTemplateId);
    if (!roleTemplate) {
      issues.push(slotIssue("error", "unknown_slot_role_template", `Role slot ${slot.id} references missing role template ${slot.roleTemplateId}.`, slot.id));
      continue;
    }

    const plan = plansBySlot.get(slot.id);
    if (!plan) {
      if (slot.required) {
        issues.push(slotIssue("error", "missing_required_slot_plan", `${slot.label} is required.`, slot.id));
      } else {
        issues.push(slotIssue("warning", "optional_slot_skipped", `${slot.label} will be skipped.`, slot.id));
      }
      continue;
    }

    if (plan.action === "skip") {
      issues.push(slotIssue(slot.required ? "error" : "warning", slot.required ? "required_slot_skipped" : "optional_slot_skipped", `${slot.label} will be skipped.`, slot.id));
      continue;
    }

    if (plan.action === "reuse") {
      if (seenReuse.has(plan.existingAgentId)) {
        issues.push(slotIssue("error", "duplicate_existing_agent_assignment", "An existing agent cannot fill multiple slots in the same launch.", slot.id, "existingAgentId"));
      }
      seenReuse.add(plan.existingAgentId);
      issues.push(...validateExistingAgentReuse({
        slot,
        roleTemplate,
        existingAgentId: plan.existingAgentId,
        existingAgents: input.existingAgents ?? [],
        companyId: input.companyId,
        workspaceId: input.workspaceId,
        assignedAgentIds: seenReuse,
        activeConflictingJobIds: plan.activeConflictingJobIds,
      }));
      continue;
    }

    if (existingAgentNames.has(normalizeName(plan.agentName))) {
      issues.push(slotIssue("error", "agent_name_conflict", `An agent named ${plan.agentName} already exists.`, slot.id, "agentName"));
    }
    const roleValidation = validateRoleAgentCreation({
      roleTemplateId: roleTemplate.id,
      agentName: plan.agentName,
      setupInputs: plan.setupInputs,
      systemDefaultDriver: input.systemDefaultDriver,
      groupDefaultDriver: groupDefaultForRole(groupTemplate, roleTemplate),
      userOverrideDriver: plan.userOverrideDriver,
      driverAvailability: input.driverAvailability,
      instructions: plan.instructions,
      workspacePath: plan.workspacePath,
      skillPaths: plan.skillPaths,
      webhookUrl: plan.webhookUrl,
      webhookSecret: plan.webhookSecret,
      homeDir: input.homeDir,
    });
    issues.push(...roleValidation.errors.map((item) => ({ ...item, slotId: slot.id })));
    issues.push(...roleValidation.warnings.map((item) => ({ ...item, slotId: slot.id })));
  }

  for (const plan of input.rolePlans) {
    if (!groupTemplate.roleSlots.some((slot) => slot.id === plan.slotId)) {
      issues.push(slotIssue("error", "unknown_role_slot", `Unknown role slot ${plan.slotId}.`, plan.slotId));
    }
  }

  return validationResult(issues);
}

export function validateExistingAgentReuse(options: {
  slot: RoleSlot;
  roleTemplate: RoleTemplate;
  existingAgentId: string;
  existingAgents: ExistingAgentCandidate[];
  companyId: string;
  workspaceId: string;
  assignedAgentIds?: Set<string>;
  activeConflictingJobIds?: string[];
}): ValidationIssue[] {
  const issues: ValidationIssue[] = [];
  const candidate = options.existingAgents.find((agent) => agent.principal.id === options.existingAgentId);
  if (!candidate) {
    return [
      slotIssue("error", "unknown_existing_agent", `Existing agent ${options.existingAgentId} was not found.`, options.slot.id, "existingAgentId"),
    ];
  }

  if (candidate.principal.principal_type !== "agent") {
    issues.push(slotIssue("error", "existing_principal_not_agent", "Existing task-owner slots can only reuse agent principals.", options.slot.id, "existingAgentId"));
  }
  if (candidate.principal.channel_visibility === "internal") {
    issues.push(slotIssue("error", "existing_agent_internal", "Internal agents cannot be reused as visible group task owners.", options.slot.id, "existingAgentId"));
  }

  if (!candidate.companyId) {
    issues.push(slotIssue("error", "company_membership_unverified", "Existing agent company membership could not be verified.", options.slot.id, "existingAgentId"));
  } else if (candidate.companyId !== options.companyId) {
    issues.push(slotIssue("error", "company_mismatch", "Existing agent belongs to a different company.", options.slot.id, "existingAgentId"));
  }
  const agentWorkspaceId = candidate.workspaceId ?? candidate.principal.workspace_id ?? candidate.runtimeBinding?.workspace_id;
  if (agentWorkspaceId && agentWorkspaceId !== options.workspaceId) {
    issues.push(slotIssue("error", "workspace_mismatch", "Existing agent belongs to a different workspace.", options.slot.id, "existingAgentId"));
  }
  if (candidate.principal.disabled) {
    issues.push(slotIssue("warning", "existing_agent_disabled", "Existing agent is disabled and may need to be re-enabled before launch.", options.slot.id, "existingAgentId"));
  }
  if (!candidate.runtimeBinding) {
    issues.push(slotIssue("warning", "missing_runtime_binding", "Existing agent has no runtime binding.", options.slot.id, "existingAgentId"));
  } else {
    const driver = candidate.runtimeBinding.driver_type as DriverId;
    if (!options.roleTemplate.compatibleDrivers.includes(driver)) {
      issues.push(slotIssue("warning", "reuse_driver_mismatch", `${driver} is not a recommended driver for ${options.roleTemplate.name}.`, options.slot.id, "existingAgentId"));
    }
    if (!REUSABLE_RUNTIME_BINDING_STATES.has(candidate.runtimeBinding.state)) {
      issues.push(slotIssue("warning", "runtime_binding_not_reusable", "Existing agent runtime binding is not ready for reuse.", options.slot.id, "existingAgentId"));
    }
  }

  const activeJobIds = options.activeConflictingJobIds ?? candidate.activeJobIds ?? [];
  if (activeJobIds.length > 0) {
    issues.push(slotIssue("error", "active_conflicting_job", "Existing agent is already part of an active provisioning job.", options.slot.id, "existingAgentId"));
  }

  return issues;
}

function groupDefaultForRole(groupTemplate: GroupTemplate, roleTemplate: RoleTemplate): DriverId | undefined {
  const policy = groupTemplate.defaultDriverPolicy;
  if (policy.applyToCompatibleRoles && roleTemplate.compatibleDrivers.includes(policy.driver)) {
    return policy.driver;
  }
  if (policy.fallback === "role_recommendation") return undefined;
  return policy.driver;
}

function addDriverAvailabilityIssue(issues: ValidationIssue[], driver: DriverId, availability: DriverAvailabilityItem[] = []): void {
  const item = availability.find((candidate) => candidate.driverId === driver);
  if (item && !item.available) {
    issues.push(issue("error", "driver_unavailable", item.reason, "driver"));
  }
}

function isDriverAvailable(driver: DriverId, availability: DriverAvailabilityItem[]): boolean {
  const item = availability.find((candidate) => candidate.driverId === driver);
  return item ? item.available : true;
}

function validateAgentName(issues: ValidationIssue[], agentName: string): void {
  const trimmed = agentName.trim();
  if (!trimmed) {
    issues.push(issue("error", "missing_agent_name", "Agent name is required.", "agentName"));
  } else if (trimmed.length > 80) {
    issues.push(issue("error", "agent_name_too_long", "Agent name must be 80 characters or fewer.", "agentName"));
  }
}

function validateGroupName(issues: ValidationIssue[], groupName: string, existingGroupNames: string[]): void {
  const trimmed = groupName.trim();
  if (!trimmed) {
    issues.push(issue("error", "missing_group_name", "Group name is required.", "groupName"));
  } else if (existingGroupNames.map(normalizeName).includes(normalizeName(trimmed))) {
    issues.push(issue("error", "group_name_conflict", `A group named ${trimmed} already exists.`, "groupName"));
  }
}

function validateSetupInputs(issues: ValidationIssue[], roleTemplate: RoleTemplate, values: SetupInputValues): void {
  for (const input of roleTemplate.setupInputs) {
    if (input.required && !values[input.id]?.trim() && !input.defaultValue?.trim()) {
      issues.push(issue("error", "missing_required_setup_input", `${input.label} is required.`, `setupInputs.${input.id}`));
    }
  }
}

function validateWorkspacePath(issues: ValidationIssue[], workspacePath: string | undefined, homeDir: string | undefined): void {
  if (!workspacePath?.trim()) return;
  validateAbsoluteHomePath(issues, workspacePath, homeDir, "workspacePath", "Workspace path");
}

function validateSkillPaths(issues: ValidationIssue[], skillPaths: string[], homeDir: string | undefined): void {
  for (const [index, skillPath] of skillPaths.entries()) {
    validateAbsoluteHomePath(issues, skillPath, homeDir, `skillPaths.${index}`, "Skill path");
  }
}

function validateAbsoluteHomePath(issues: ValidationIssue[], value: string, homeDir: string | undefined, field: string, label: string): void {
  if (value.includes("\0")) {
    issues.push(issue("error", "invalid_path", `${label} cannot contain null bytes.`, field));
    return;
  }
  const resolved = path.resolve(value);
  if (!path.isAbsolute(value)) {
    issues.push(issue("error", "path_must_be_absolute", `${label} must be an absolute path.`, field));
  }
  if (homeDir && !isPathInside(resolved, homeDir)) {
    issues.push(issue("error", "path_outside_home", `${label} must be under ${homeDir}.`, field));
  }
}

function validateWebhook(issues: ValidationIssue[], driver: DriverId, webhookUrl: string | undefined, webhookSecret: string | undefined): void {
  if (driver !== "webhook_agent") {
    if (webhookUrl?.trim() || webhookSecret?.trim()) {
      issues.push(issue("warning", "webhook_fields_ignored", "Webhook settings are ignored for local CLI drivers.", "webhookUrl"));
    }
    return;
  }
  if (!webhookUrl?.trim()) {
    issues.push(issue("error", "missing_webhook_url", "Webhook URL is required for webhook agents.", "webhookUrl"));
    return;
  }
  try {
    const url = new URL(webhookUrl);
    if (url.protocol !== "http:" && url.protocol !== "https:") {
      issues.push(issue("error", "invalid_webhook_url", "Webhook URL must use http or https.", "webhookUrl"));
    }
  } catch {
    issues.push(issue("error", "invalid_webhook_url", "Webhook URL must be a valid URL.", "webhookUrl"));
  }
  if (webhookSecret !== undefined && webhookSecret.trim().length > 0 && webhookSecret.trim().length < 16) {
    issues.push(issue("error", "webhook_secret_too_short", "Webhook secret must be at least 16 characters when provided.", "webhookSecret"));
  }
}

function validateInstructionPresence(issues: ValidationIssue[], driver: DriverId, instructions: string | undefined): void {
  if (LOCAL_CLI_DRIVERS.has(driver) && !instructions?.trim()) {
    issues.push(issue("error", "missing_instructions", "Instructions are required for local CLI drivers.", "instructions"));
  }
}

function isPathInside(candidatePath: string, rootPath: string): boolean {
  const relative = path.relative(path.resolve(rootPath), path.resolve(candidatePath));
  return relative === "" || (!!relative && !relative.startsWith("..") && !path.isAbsolute(relative));
}

function normalizeName(value: string): string {
  return value.trim().toLowerCase();
}

function issue(severity: ValidationSeverity, code: string, message: string, field?: string): ValidationIssue {
  return { severity, code, message, field };
}

function slotIssue(severity: ValidationSeverity, code: string, message: string, slotId: string, field?: string): ValidationIssue {
  return { severity, code, message, slotId, field };
}
