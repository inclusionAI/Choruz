import {
  emptyFields,
  fieldsToMarkdown,
  type AgentInstructionFields,
} from "../agents/agent-instructions";
import {
  BOARD_TASKS_CREATED_SECTION,
  type GroupTemplate,
  type InstructionStatus,
  type RoleSlot,
  type RoleTemplate,
  type SetupInputValues,
} from "./team-templates";

export type RenderRoleInstructionOptions = {
  roleTemplate: RoleTemplate;
  setupInputs?: SetupInputValues;
  groupMission?: string;
  roleSlot?: RoleSlot;
  workspaceConstraints?: string;
  dataConstraints?: string;
  outputExpectations?: string;
  customizedMarkdown?: string;
};

export type RenderedRoleInstructions = {
  fields: AgentInstructionFields;
  markdown: string;
  status: InstructionStatus;
};

export type GroupKickoffMember = {
  name: string;
  roleLabel?: string;
};

export function renderRoleInstructions(options: RenderRoleInstructionOptions): RenderedRoleInstructions {
  const fields = buildTemplateFields(options);
  const templateMarkdown = fieldsToMarkdown(fields);

  if (options.customizedMarkdown !== undefined && options.customizedMarkdown !== templateMarkdown) {
    return {
      fields,
      markdown: options.customizedMarkdown,
      status: "customized",
    };
  }

  return {
    fields,
    markdown: templateMarkdown,
    status: hasGroupContext(options) ? "group_context_added" : "template_default",
  };
}

export function resetRoleInstructionsToTemplate(options: Omit<RenderRoleInstructionOptions, "customizedMarkdown">): RenderedRoleInstructions {
  return renderRoleInstructions(options);
}

export function renderGroupKickoff(
  template: GroupTemplate,
  mission: string,
  options: { members?: GroupKickoffMember[] } = {},
): string {
  const missionText = mission.trim() || "To be confirmed by the user.";
  const memberSummary = options.members
    ?.map((member) => member.roleLabel ? `${member.name} (${member.roleLabel})` : member.name)
    .filter(Boolean)
    .join(", ");
  const roleSummary = memberSummary || template.roleSlots
    .filter((slot) => slot.required)
    .map((slot) => slot.label)
    .join(", ");
  return [
    template.kickoffTemplate.body.replaceAll("{{mission}}", missionText),
    memberSummary ? `Current members: ${roleSummary}.` : `Roles: ${roleSummary}.`,
    "Next user action: send the first concrete work item or question when ready. Until then, work waits for the user kickoff.",
  ].join("\n\n");
}

function buildTemplateFields(options: RenderRoleInstructionOptions): AgentInstructionFields {
  const { roleTemplate, setupInputs = {}, roleSlot } = options;
  const parts = {
    identity: "",
    goals: "",
    projectContext: "",
    commScope: "",
    allowedOps: "",
    forbiddenOps: "",
    sop: "",
    workStyle: "",
    collaboration: "",
    escalation: "",
    completionCriteria: "",
    errorHandling: "",
  };
  const setupSummary = summarizeSetupInputs(roleTemplate, setupInputs);
  const constraints = summarizeConstraints(options);
  const outputExpectation = options.outputExpectations || roleTemplate.outputContract.summary;

  parts.identity = [
    `You are the ${roleTemplate.name}.`,
    roleTemplate.description,
    `Role instruction source:\n${roleTemplate.instructionTemplate}`,
    roleSlot ? `In this group, you fill the "${roleSlot.label}" role slot.` : "",
  ]
    .filter(Boolean)
    .join("\n\n");

  parts.goals = [
    "Primary responsibilities:",
    bulletList(roleSlot?.responsibilities.length ? roleSlot.responsibilities : roleTemplate.bestFor),
    setupSummary ? `Setup inputs:\n${setupSummary}` : "",
  ]
    .filter(Boolean)
    .join("\n\n");

  parts.projectContext = [
    setupInputs.project_context ? `Project context:\n${setupInputs.project_context}` : "",
    setupInputs.repository_path ? `Repository path:\n${setupInputs.repository_path}` : "",
    setupInputs.data_sources ? `Approved data sources:\n${setupInputs.data_sources}` : "",
    setupInputs.source_constraints ? `Source constraints:\n${setupInputs.source_constraints}` : "",
    `Suggested workspace access:\n${roleTemplate.suggestedAccess.workspace}`,
    `Suggested data access:\n${roleTemplate.suggestedAccess.data}`,
    constraints,
  ]
    .filter(Boolean)
    .join("\n\n");

  parts.commScope = [
    "Respond when the user or group conversation asks for this role's work.",
    options.groupMission ? "Coordinate through the group mission and role responsibilities; do not assume work has started until the user kicks it off." : "",
  ]
    .filter(Boolean)
    .join("\n\n");

  parts.allowedOps = [
    "Read relevant project files, user-provided context, and approved data sources.",
    "Run focused checks that are appropriate for the selected driver and workspace.",
    roleTemplate.suggestedSkills.length ? `Suggested skills:\n${bulletList(roleTemplate.suggestedSkills)}` : "",
  ]
    .filter(Boolean)
    .join("\n\n");

  parts.forbiddenOps = [
    "Do not expose secrets, credentials, private key material, or raw tokens.",
    "Do not commit, push, delete, or mutate external systems unless the user explicitly asks.",
    "Do not start group work from the launch kickoff alone; wait for the user's first task or confirmation.",
  ].join("\n");

  parts.sop = [
    options.groupMission ? `Group mission:\n${options.groupMission}` : "",
    `Role-specific instruction template:\n${roleTemplate.instructionTemplate}`,
    "Workflow:",
    bulletList([
      "Clarify the task and assumptions when needed.",
      "Inspect the relevant context before acting.",
      "Do the narrowest useful work for the role.",
      "Report results in the expected output format.",
    ]),
  ]
    .filter(Boolean)
    .join("\n\n");

  parts.workStyle = [
    "Be concise, specific, and evidence-driven.",
    `Expected output: ${outputExpectation}.`,
    `Output format: ${roleTemplate.outputContract.format}.`,
    `Required sections:\n${bulletList(roleTemplate.outputContract.requiredSections)}`,
    boardTasksReceiptGuidance(roleTemplate.outputContract.requiredSections),
  ]
    .filter(Boolean)
    .join("\n\n");

  parts.collaboration = [
    roleSlot ? `Role-slot responsibilities:\n${bulletList(roleSlot.responsibilities)}` : "",
    "Share blockers early and hand off clear findings to the user or coordinating role.",
  ]
    .filter(Boolean)
    .join("\n\n");

  parts.escalation = "Ask the user before broadening scope, changing persistent state, using unapproved data, or making irreversible changes.";
  parts.completionCriteria = [
    "The requested role output is delivered.",
    "Assumptions, verification, and residual risks are clearly stated.",
    "Any missing access or blocked work is explicitly reported.",
  ].join("\n");
  parts.errorHandling = "If context, files, tools, or data are unavailable, explain the blocker and provide the next best safe path.";

  const fields = emptyFields();
  fields.role = `${parts.identity}\n\n${parts.goals}`;
  fields.projectContext = parts.projectContext;
  fields.boundaries = `${parts.allowedOps}\n\n${parts.forbiddenOps}\n\n${parts.workStyle}`;
  fields.workflow = `${parts.sop}\n\nDone when:\n${parts.completionCriteria}\n\n${parts.errorHandling}`;
  fields.collaboration = [parts.commScope, parts.collaboration, parts.escalation].filter(Boolean).join("\n\n");
  return fields;
}

function summarizeSetupInputs(roleTemplate: RoleTemplate, values: SetupInputValues): string {
  return roleTemplate.setupInputs
    .map((input) => {
      const value = values[input.id]?.trim() || input.defaultValue || "";
      if (!value) return "";
      return `- ${input.label}: ${value}`;
    })
    .filter(Boolean)
    .join("\n");
}

function summarizeConstraints(options: RenderRoleInstructionOptions): string {
  return [
    options.workspaceConstraints ? `Workspace constraints:\n${options.workspaceConstraints}` : "",
    options.dataConstraints ? `Data constraints:\n${options.dataConstraints}` : "",
  ]
    .filter(Boolean)
    .join("\n\n");
}

function hasGroupContext(options: RenderRoleInstructionOptions): boolean {
  return Boolean(options.groupMission?.trim() || options.roleSlot);
}

function bulletList(items: string[]): string {
  return items.map((item) => `- ${item}`).join("\n");
}

function boardTasksReceiptGuidance(requiredSections: string[]): string {
  if (!requiredSections.includes(BOARD_TASKS_CREATED_SECTION)) {
    return "";
  }
  return [
    `${BOARD_TASKS_CREATED_SECTION} is a receipt of the silent task_create commands you issued in this same turn, not a prose plan.`,
    "Format the section as a numbered list of `task_id — title — assignee` lines, one per task_create call.",
    `If no Kanban-worthy work was warranted this turn, write exactly \`${BOARD_TASKS_CREATED_SECTION}: none — <reason>\` and do not invent task IDs.`,
    "Prose assignments do not satisfy this section: a turn without task_create commands (or an explicit none line) fails the output contract.",
  ].join("\n");
}
