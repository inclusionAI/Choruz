import { fieldsToMarkdown, type AgentInstructionFields } from "./agent-instructions";
import type { DriverAvailabilityItem } from "../drivers/driver-availability";
import { driverDisplayName } from "../drivers/driver-registry";
import type { RoleTemplateAgentCreationMetadata } from "../groups/group-provisioning-contract";
import {
  generateAgentName,
  ROLE_TEMPLATES,
  type DriverId,
  type InstructionStatus,
  type RoleTemplate,
  type RoleTemplateCategory,
  type SetupInputValues,
} from "../groups/team-templates";
import { renderRoleInstructions } from "../groups/team-template-renderer";

export type CreateAgentWorkspaceMode = "generated" | "custom" | "none";

export type ClientDriverAvailabilityItem = Omit<DriverAvailabilityItem, "available"> & {
  available?: boolean;
};

export type TemplateInstructionDraft = {
  agentName: string;
  driver: DriverId;
  setupInputs: SetupInputValues;
  instructionFields: AgentInstructionFields;
  instructionStatus: InstructionStatus;
};

export type TemplateDriverWarning = {
  code: "incompatible_driver" | "driver_unavailable" | "driver_fallback_available";
  message: string;
};

export type TemplateBlockingIssue = {
  code:
    | "driver_unavailable"
    | "missing_required_setup_input"
    | "missing_webhook_url"
    | "invalid_webhook_url"
    | "webhook_secret_too_short";
  message: string;
  field?: string;
};

export type ProvisioningPayloadInput = {
  name: string;
  driver: DriverId;
  instructionFields: AgentInstructionFields;
  activeCompanyId?: string | null;
  workspaceMode: CreateAgentWorkspaceMode;
  workspacePath?: string;
  selectedSkillPaths: string[];
  webhookUrl?: string;
  webhookSecret?: string;
  roleTemplate?: RoleTemplate | null;
  setupInputs?: SetupInputValues;
  instructionStatus?: InstructionStatus;
  model?: string;
  runtimeHostId?: string;
  harnessAccountId?: string;
};

export type ReviewItem = {
  label: string;
  value: string;
};

export type CreateAgentProvisioningPayload = {
  name: string;
  driver_type: DriverId;
  instructions?: string;
  workspace_id?: string;
  workspace_path?: string;
  skill_paths?: string[];
  webhook_url?: string;
  webhook_secret?: string;
  model?: string;
  runtime_host_id?: string;
  harness_account_id?: string;
  template_metadata?: RoleTemplateAgentCreationMetadata & {
    driverSource: "role_template_recommendation" | "user_override";
    recommendedDriver: DriverId;
  };
};

const CATEGORY_LABELS: Record<RoleTemplateCategory, string> = {
  software: "Software",
  finance: "Finance",
  research: "Research",
  operations: "Operations",
};

export function groupedRoleTemplates(): Array<{
  category: RoleTemplateCategory;
  label: string;
  templates: RoleTemplate[];
}> {
  return (Object.keys(CATEGORY_LABELS) as RoleTemplateCategory[])
    .map((category) => ({
      category,
      label: CATEGORY_LABELS[category],
      templates: ROLE_TEMPLATES.filter((template) => template.category === category),
    }))
    .filter((group) => group.templates.length > 0);
}

export function createTemplateDraft(options: {
  roleTemplate: RoleTemplate;
  setupInputs?: SetupInputValues;
  workspacePath?: string;
  workspaceMode?: CreateAgentWorkspaceMode;
}): TemplateInstructionDraft {
  const setupInputs = defaultSetupInputValues(options.roleTemplate, options.setupInputs);
  const rendered = renderRoleInstructions({
    roleTemplate: options.roleTemplate,
    setupInputs,
    workspaceConstraints: workspaceConstraint(options.workspaceMode, options.workspacePath),
  });

  return {
    agentName: generateAgentName(options.roleTemplate),
    driver: options.roleTemplate.recommendedDriver,
    setupInputs,
    instructionFields: rendered.fields,
    instructionStatus: rendered.status,
  };
}

export function regenerateTemplateInstructions(options: {
  roleTemplate: RoleTemplate;
  setupInputs: SetupInputValues;
  workspacePath?: string;
  workspaceMode?: CreateAgentWorkspaceMode;
}): Pick<TemplateInstructionDraft, "instructionFields" | "instructionStatus"> {
  const rendered = renderRoleInstructions({
    roleTemplate: options.roleTemplate,
    setupInputs: options.setupInputs,
    workspaceConstraints: workspaceConstraint(options.workspaceMode, options.workspacePath),
  });

  return {
    instructionFields: rendered.fields,
    instructionStatus: rendered.status,
  };
}

export function instructionStatusLabel(status: InstructionStatus): string {
  switch (status) {
    case "template_default":
      return "Template default";
    case "customized":
      return "Customized";
    case "group_context_added":
      return "Group context added";
  }
}

export function driverWarnings(options: {
  roleTemplate?: RoleTemplate | null;
  driver: DriverId;
  availability?: ClientDriverAvailabilityItem[];
}): TemplateDriverWarning[] {
  const warnings: TemplateDriverWarning[] = [];
  const { roleTemplate, driver } = options;

  if (roleTemplate && !roleTemplate.compatibleDrivers.includes(driver)) {
    warnings.push({
      code: "incompatible_driver",
      message: `${driverDisplayName(driver)} is not compatible with ${roleTemplate.name}.`,
    });
  }

  const availability = options.availability ?? [];
  const selected = availability.find((item) => item.driverId === driver);
  if (selected && !isAvailable(selected)) {
    warnings.push({
      code: "driver_unavailable",
      message: selected.reason || `${driverDisplayName(driver)} is unavailable on this machine.`,
    });
  }

  if (roleTemplate && selected && !isAvailable(selected)) {
    const fallback = roleTemplate.compatibleDrivers.find((candidate) => {
      if (candidate === driver) return false;
      const item = availability.find((availableItem) => availableItem.driverId === candidate);
      return item ? isAvailable(item) : true;
    });
    if (fallback) {
      warnings.push({
        code: "driver_fallback_available",
        message: `${driverDisplayName(fallback)} is the next compatible available driver.`,
      });
    }
  }

  return warnings;
}

export function templateBlockingIssues(options: {
  roleTemplate?: RoleTemplate | null;
  driver: DriverId;
  availability?: ClientDriverAvailabilityItem[];
  setupInputs?: SetupInputValues;
  webhookUrl?: string;
  webhookSecret?: string;
}): TemplateBlockingIssue[] {
  const { roleTemplate } = options;
  const issues: TemplateBlockingIssue[] = [];
  const selectedDriver = options.availability?.find((item) => item.driverId === options.driver);
  if (selectedDriver && !isAvailable(selectedDriver)) {
    issues.push({
      code: "driver_unavailable",
      message: selectedDriver.reason || `${driverDisplayName(options.driver)} is unavailable on this machine.`,
      field: "driver",
    });
  }

  if (roleTemplate) {
    for (const input of roleTemplate.setupInputs) {
      if (input.required && !(options.setupInputs?.[input.id] ?? "").trim()) {
        issues.push({
          code: "missing_required_setup_input",
          message: `${input.label} is required for ${roleTemplate.name}.`,
          field: input.id,
        });
      }
    }
  }

  if (options.driver === "webhook_agent") {
    const webhookUrl = options.webhookUrl?.trim() ?? "";
    if (!webhookUrl) {
      issues.push({
        code: "missing_webhook_url",
        message: "Webhook URL is required for webhook agents.",
        field: "webhookUrl",
      });
    } else {
      try {
        const url = new URL(webhookUrl);
        if (url.protocol !== "http:" && url.protocol !== "https:") {
          issues.push({
            code: "invalid_webhook_url",
            message: "Webhook URL must use http or https.",
            field: "webhookUrl",
          });
        }
      } catch {
        issues.push({
          code: "invalid_webhook_url",
          message: "Webhook URL must be a valid URL.",
          field: "webhookUrl",
        });
      }
    }

    if ((options.webhookSecret?.trim().length ?? 0) > 0 && options.webhookSecret!.trim().length < 16) {
      issues.push({
        code: "webhook_secret_too_short",
        message: "Webhook secret must be at least 16 characters when provided.",
        field: "webhookSecret",
      });
    }
  }

  return issues;
}

export function buildCreateAgentProvisioningPayload(
  input: ProvisioningPayloadInput,
): CreateAgentProvisioningPayload {
  const isWebhookMode = input.driver === "webhook_agent";
  const payload: CreateAgentProvisioningPayload = {
    name: input.name.trim(),
    driver_type: input.driver,
    ...(isWebhookMode
      ? {
          webhook_url: input.webhookUrl?.trim() ?? "",
          ...(input.webhookSecret?.trim()
            ? { webhook_secret: input.webhookSecret.trim() }
            : {}),
        }
      : {
          instructions: fieldsToMarkdown(input.instructionFields),
          ...(input.model?.trim() ? { model: input.model.trim() } : {}),
          ...(input.workspaceMode === "custom" && input.workspacePath?.trim()
            ? { workspace_path: input.workspacePath.trim() }
            : {}),
          ...(input.selectedSkillPaths.length > 0
            ? { skill_paths: input.selectedSkillPaths }
            : {}),
        }),
    ...(input.activeCompanyId ? { workspace_id: input.activeCompanyId } : {}),
    ...(!isWebhookMode && input.runtimeHostId?.trim()
      ? { runtime_host_id: input.runtimeHostId.trim() }
      : {}),
    ...(!isWebhookMode && input.harnessAccountId?.trim()
      ? { harness_account_id: input.harnessAccountId.trim() }
      : {}),
  };

  if (input.roleTemplate) {
    payload.template_metadata = {
      mode: "role_template",
      roleTemplateId: input.roleTemplate.id,
      roleTemplateVersion: input.roleTemplate.version,
      instructionStatus: input.instructionStatus ?? "template_default",
      setupSummary: input.setupInputs ?? {},
      selectedSkills: input.selectedSkillPaths,
      workspaceMode: input.workspaceMode,
      driverSource:
        input.driver === input.roleTemplate.recommendedDriver
          ? "role_template_recommendation"
          : "user_override",
      recommendedDriver: input.roleTemplate.recommendedDriver,
    };
  }

  return payload;
}

export function buildCreateAgentReviewItems(options: {
  agentName: string;
  driver: DriverId;
  workspaceMode: CreateAgentWorkspaceMode;
  workspacePath?: string;
  roleTemplate?: RoleTemplate | null;
  selectedSkillNames?: string[];
  instructionStatus?: InstructionStatus;
  webhookUrl?: string;
  webhookSecretProvided?: boolean;
  model?: string;
  runtimeHostName?: string;
  harnessAccountName?: string;
}): ReviewItem[] {
  const selectedSkillNames = options.selectedSkillNames ?? [];
  const items: ReviewItem[] = [
    {
      label: "Agent",
      value: options.agentName.trim() || "Unnamed agent",
    },
    {
      label: "Start with",
      value: options.roleTemplate?.name ?? "Blank Agent",
    },
    {
      label: "Driver",
      value: driverDisplayName(options.driver),
    },
    {
      label: "Model",
      value: options.model?.trim() || "Harness default",
    },
    ...(options.driver === "webhook_agent"
      ? []
      : [{
          label: "Runtime server",
          value: options.runtimeHostName || "This computer",
        }]),
    ...(options.driver !== "webhook_agent" && options.harnessAccountName
      ? [{ label: "Harness account", value: options.harnessAccountName }]
      : []),
    {
      label: "Workspace behavior",
      value:
        options.workspaceMode === "custom"
          ? `Custom path: ${options.workspacePath || "not selected"}`
          : "Generated workspace",
    },
    {
      label: "Selected skills",
      value:
        selectedSkillNames.length > 0
          ? selectedSkillNames.join(", ")
          : "None",
    },
    {
      label: "Instructions",
      value: options.roleTemplate
        ? instructionStatusLabel(options.instructionStatus ?? "template_default")
        : "Manual",
    },
  ];

  if (options.driver === "webhook_agent") {
    items.push({
      label: "Webhook",
      value: `${options.webhookUrl?.trim() ?? ""}${
        options.webhookSecretProvided
          ? " with provided signing secret"
          : " with generated signing secret"
      }`,
    });
  }

  items.push({
    label: "Mentionability",
    value: `@${options.agentName.trim() || "agent"} in direct or group chat`,
  });

  return items;
}

function defaultSetupInputValues(
  roleTemplate: RoleTemplate,
  values: SetupInputValues = {},
): SetupInputValues {
  return Object.fromEntries(
    roleTemplate.setupInputs.map((input) => [
      input.id,
      values[input.id] ?? input.defaultValue ?? "",
    ]),
  );
}

function workspaceConstraint(
  workspaceMode: CreateAgentWorkspaceMode | undefined,
  workspacePath: string | undefined,
): string | undefined {
  if (workspaceMode !== "custom" || !workspacePath?.trim()) return undefined;
  return `Use the custom workspace path selected during setup: ${workspacePath.trim()}`;
}

function isAvailable(item: ClientDriverAvailabilityItem): boolean {
  return item.available ?? item.status === "available";
}
