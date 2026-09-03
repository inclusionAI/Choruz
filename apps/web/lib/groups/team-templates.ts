import {
  DRIVER_IDS,
  LOCAL_TERMINAL_DRIVER_IDS,
  type DriverId,
} from "../drivers/driver-registry";

export { DRIVER_IDS, LOCAL_TERMINAL_DRIVER_IDS, type DriverId } from "../drivers/driver-registry";

export const LOCAL_CODING_DRIVER_IDS: DriverId[] = [
  ...LOCAL_TERMINAL_DRIVER_IDS,
  "codex_exec",
];

export type TemplateVersion = `${number}.${number}.${number}`;

export type RoleTemplateCategory = "software" | "finance" | "research" | "operations";

export type SetupInputType = "text" | "textarea" | "path" | "select";

export type SetupInputOption = {
  value: string;
  label: string;
};

export type SetupInput = {
  id: string;
  label: string;
  type: SetupInputType;
  required: boolean;
  description: string;
  placeholder?: string;
  defaultValue?: string;
  options?: SetupInputOption[];
};

export type SetupInputValues = Record<string, string>;

export type InstructionStatus = "template_default" | "customized" | "group_context_added";

/**
 * Canonical name of the receipt section that documents which silent
 * `task_create` commands an agent issued in a turn. Shared with
 * `team-template-renderer.ts` so a typo in either file would be a
 * compile-time mismatch instead of silently disabling the receipt guidance.
 */
export const BOARD_TASKS_CREATED_SECTION = "Board tasks created" as const;

export type OutputContract = {
  summary: string;
  format: string;
  requiredSections: string[];
};

export type SuggestedAccess = {
  workspace: string;
  data: string;
};

export type RoleTemplate = {
  id: string;
  version: TemplateVersion;
  name: string;
  category: RoleTemplateCategory;
  description: string;
  bestFor: string[];
  instructionTemplate: string;
  recommendedDriver: DriverId;
  compatibleDrivers: DriverId[];
  setupInputs: SetupInput[];
  suggestedAccess: SuggestedAccess;
  suggestedSkills: string[];
  suggestedFirstTasks: string[];
  outputContract: OutputContract;
};

export type RoleSlot = {
  id: string;
  label: string;
  roleTemplateId: string;
  required: boolean;
  defaultAgentName: string;
  responsibilities: string[];
  workflowRoleKeys?: string[];
};

export type GroupTemplateWorkflow = {
  steps: string[];
  description: string;
  coordinatorRoleSlotId?: string;
  participantRoleDefaults?: Record<string, string[]>;
};

export type DefaultDriverPolicy = {
  driver: DriverId;
  applyToCompatibleRoles: boolean;
  fallback: "role_recommendation";
};

export type KickoffTemplate = {
  title: string;
  body: string;
};

export type GroupTemplate = {
  id: string;
  version: TemplateVersion;
  name: string;
  description: string;
  suggestedGroupName: string;
  defaultDriverPolicy: DefaultDriverPolicy;
  roleSlots: RoleSlot[];
  workflow: GroupTemplateWorkflow;
  kickoffTemplate: KickoffTemplate;
  outputContract: OutputContract;
  recommendedAccessModel: string;
};

const VERSION: TemplateVersion = "1.0.0";

const commonCodingInputs: SetupInput[] = [
  {
    id: "repository_path",
    label: "Repository path",
    type: "path",
    required: false,
    description: "Optional local repository or workspace path this agent should use.",
    placeholder: "/path/to/repository",
  },
  {
    id: "project_context",
    label: "Project context",
    type: "textarea",
    required: false,
    description: "Short notes about architecture, stack, conventions, and constraints.",
    placeholder: "Rust backend, Next.js frontend, tests to prefer...",
  },
];

const commonFinanceInputs: SetupInput[] = [
  {
    id: "analysis_question",
    label: "Analysis question",
    type: "textarea",
    required: true,
    description: "The financial question or decision this agent should support.",
    placeholder: "Should we invest in this company under the base-case assumptions?",
  },
  {
    id: "data_sources",
    label: "Data sources",
    type: "textarea",
    required: false,
    description: "Approved source files, URLs, databases, or manual assumptions.",
    placeholder: "10-K, model.xlsx, market data export...",
  },
];

const commonResearchInputs: SetupInput[] = [
  {
    id: "research_question",
    label: "Research question",
    type: "textarea",
    required: true,
    description: "The question this agent should investigate.",
    placeholder: "Compare the strongest approaches for...",
  },
  {
    id: "source_constraints",
    label: "Source constraints",
    type: "textarea",
    required: false,
    description: "Preferred, required, or forbidden sources for this research.",
    placeholder: "Prefer primary sources; avoid unsourced blogs...",
  },
];

export const ROLE_TEMPLATES: RoleTemplate[] = [
  {
    id: "backend-engineer",
    version: VERSION,
    name: "Backend Engineer",
    category: "software",
    description: "Designs and implements backend changes with focused tests and operational risk notes.",
    bestFor: ["API work", "database behavior", "service logic", "backend bug fixes"],
    instructionTemplate: "Act as a careful backend implementer: inspect the relevant service contracts, make narrow code changes, add focused tests, and report operational risks.",
    recommendedDriver: "codex_terminal",
    compatibleDrivers: LOCAL_CODING_DRIVER_IDS,
    setupInputs: commonCodingInputs,
    suggestedAccess: {
      workspace: "Use the target repository or generated workspace with backend source, tests, and migrations available.",
      data: "Use project files, local test fixtures, and user-provided architecture notes; do not require external data by default.",
    },
    suggestedSkills: ["repo-navigation", "test-running", "database-debugging"],
    suggestedFirstTasks: ["Inspect the backend area for this bug and propose the smallest safe fix."],
    outputContract: {
      summary: "Backend plan, code changes, tests, and risks",
      format: "Implementation report with changed files and verification",
      requiredSections: ["Plan", "Changed Files", "Tests", "Risks"],
    },
  },
  {
    id: "frontend-engineer",
    version: VERSION,
    name: "Frontend Engineer",
    category: "software",
    description: "Builds UI flows that match existing product patterns and verifies browser behavior.",
    bestFor: ["React components", "UI workflows", "visual QA", "client-side state"],
    instructionTemplate: "Act as a frontend product engineer: follow existing UI patterns, build the usable flow first, and verify interaction and rendering behavior.",
    recommendedDriver: "codex_terminal",
    compatibleDrivers: LOCAL_CODING_DRIVER_IDS,
    setupInputs: commonCodingInputs,
    suggestedAccess: {
      workspace: "Use the target repository or generated workspace with frontend source, tests, and browser verification tooling available.",
      data: "Use product requirements, screenshots, design notes, and local app state supplied by the user.",
    },
    suggestedSkills: ["browser-verification", "component-testing", "accessibility-review"],
    suggestedFirstTasks: ["Implement the requested UI change and verify it in the browser."],
    outputContract: {
      summary: "UI plan, code changes, screenshots/checks",
      format: "Frontend implementation report",
      requiredSections: ["UI Plan", "Changed Files", "Browser Checks", "Risks"],
    },
  },
  {
    id: "code-reviewer",
    version: VERSION,
    name: "Code Reviewer",
    category: "software",
    description: "Reviews changes for bugs, regressions, missing tests, and behavioral mismatches.",
    bestFor: ["PR review", "risk analysis", "test gap detection"],
    instructionTemplate: "Act as a findings-first code reviewer: inspect diffs and surrounding behavior, prioritize concrete bugs and regressions, and keep summaries secondary.",
    recommendedDriver: "codex_terminal",
    compatibleDrivers: LOCAL_CODING_DRIVER_IDS,
    setupInputs: commonCodingInputs,
    suggestedAccess: {
      workspace: "Use read access to the repository, current diff, and relevant tests.",
      data: "Use code, tests, task notes, and review context; avoid unrelated cleanup.",
    },
    suggestedSkills: ["diff-review", "test-analysis"],
    suggestedFirstTasks: ["Review the current diff and report findings ordered by severity."],
    outputContract: {
      summary: "Review findings, risk notes, test gaps",
      format: "Findings-first code review",
      requiredSections: ["Findings", "Open Questions", "Test Gaps"],
    },
  },
  {
    id: "qa-tester",
    version: VERSION,
    name: "QA Tester",
    category: "software",
    description: "Turns requirements into repro steps, focused test plans, and verification notes.",
    bestFor: ["Regression testing", "bug reproduction", "acceptance checks"],
    instructionTemplate: "Act as a QA tester: turn expected behavior into reproducible checks, verify risk areas, and report gaps clearly.",
    recommendedDriver: "claude_terminal",
    compatibleDrivers: LOCAL_CODING_DRIVER_IDS,
    setupInputs: commonCodingInputs,
    suggestedAccess: {
      workspace: "Use the app workspace, test runner, and browser target needed to reproduce and verify behavior.",
      data: "Use requirements, bug reports, fixtures, and user-provided repro details.",
    },
    suggestedSkills: ["test-planning", "browser-verification"],
    suggestedFirstTasks: ["Create a focused regression test plan for this change."],
    outputContract: {
      summary: "Repro steps, test plan, verification notes",
      format: "QA report",
      requiredSections: ["Repro Steps", "Test Plan", "Verification", "Gaps"],
    },
  },
  {
    id: "lead-financial-analyst",
    version: VERSION,
    name: "Lead Financial Analyst",
    category: "finance",
    description: "Frames the financial question, coordinates analysis, and owns the final memo.",
    bestFor: ["Investment memos", "financial decision support", "assumption framing"],
    instructionTemplate: "Act as the lead analyst: define the decision question, coordinate evidence and assumptions, and own a balanced final memo.",
    recommendedDriver: "claude_terminal",
    compatibleDrivers: LOCAL_CODING_DRIVER_IDS,
    setupInputs: commonFinanceInputs,
    suggestedAccess: {
      workspace: "Use a user-approved analysis workspace for notes, models, and source exports.",
      data: "Use only approved financial statements, model files, market data exports, and explicit assumptions.",
    },
    suggestedSkills: ["financial-model-review", "memo-writing"],
    suggestedFirstTasks: ["Frame the key question, assumptions, and analysis plan for this memo."],
    outputContract: {
      summary: "Structured memo and assumptions",
      format: "Investment or analysis memo",
      requiredSections: ["Question", "Assumptions", "Analysis", "Recommendation"],
    },
  },
  {
    id: "data-analyst",
    version: VERSION,
    name: "Data Analyst",
    category: "finance",
    description: "Extracts, cleans, calculates, and summarizes financial or operational metrics.",
    bestFor: ["Metric extraction", "tables", "calculations", "model checks"],
    instructionTemplate: "Act as a data analyst: extract metrics, show calculation logic, preserve table structure, and flag unreliable or missing inputs.",
    recommendedDriver: "codex_terminal",
    compatibleDrivers: LOCAL_CODING_DRIVER_IDS,
    setupInputs: commonFinanceInputs,
    suggestedAccess: {
      workspace: "Use a data/model workspace containing approved spreadsheets, CSVs, notebooks, or exported tables.",
      data: "Use approved source data only and label assumptions, derived metrics, and data gaps.",
    },
    suggestedSkills: ["spreadsheet-analysis", "data-quality-checks"],
    suggestedFirstTasks: ["Extract the key metrics and flag missing or inconsistent data."],
    outputContract: {
      summary: "Extracted metrics, tables, calculations",
      format: "Data analysis note with tables",
      requiredSections: ["Inputs", "Calculations", "Tables", "Data Gaps"],
    },
  },
  {
    id: "valuation-analyst",
    version: VERSION,
    name: "Valuation Analyst",
    category: "finance",
    description: "Builds valuation scenarios and explains the assumptions driving each case.",
    bestFor: ["DCF scenarios", "comps", "sensitivity analysis"],
    instructionTemplate: "Act as a valuation analyst: construct scenarios, make assumptions explicit, and explain sensitivity to key drivers.",
    recommendedDriver: "claude_terminal",
    compatibleDrivers: LOCAL_CODING_DRIVER_IDS,
    setupInputs: commonFinanceInputs,
    suggestedAccess: {
      workspace: "Use a user-approved model workspace with valuation inputs and scenario outputs.",
      data: "Use approved financial data, comparable company sets, user assumptions, and clearly marked external references.",
    },
    suggestedSkills: ["valuation-modeling", "scenario-analysis"],
    suggestedFirstTasks: ["Produce base, upside, and downside valuation assumptions."],
    outputContract: {
      summary: "Valuation assumptions and scenarios",
      format: "Valuation scenario note",
      requiredSections: ["Method", "Assumptions", "Scenarios", "Sensitivity"],
    },
  },
  {
    id: "risk-reviewer",
    version: VERSION,
    name: "Risk Reviewer",
    category: "finance",
    description: "Challenges assumptions, identifies downside risks, and records caveats.",
    bestFor: ["Risk challenge", "red-team review", "caveat tracking"],
    instructionTemplate: "Act as an independent risk reviewer: challenge the thesis, stress assumptions, and separate material risks from minor caveats.",
    recommendedDriver: "claude_terminal",
    compatibleDrivers: LOCAL_TERMINAL_DRIVER_IDS,
    setupInputs: commonFinanceInputs,
    suggestedAccess: {
      workspace: "Use read access to the analysis memo, model outputs, and cited source material.",
      data: "Use approved inputs and explicitly flag unsupported assumptions or missing downside evidence.",
    },
    suggestedSkills: ["risk-review", "assumption-audit"],
    suggestedFirstTasks: ["Challenge the current recommendation and list decision-critical risks."],
    outputContract: {
      summary: "Risk challenge and caveats",
      format: "Risk review",
      requiredSections: ["Material Risks", "Assumption Challenges", "Mitigations", "Caveats"],
    },
  },
  {
    id: "research-analyst",
    version: VERSION,
    name: "Research Analyst",
    category: "research",
    description: "Gathers source-aware evidence and turns it into a clear briefing.",
    bestFor: ["Landscape scans", "topic research", "evidence gathering"],
    instructionTemplate: "Act as a research analyst: gather credible evidence, keep source notes attached to claims, and distinguish facts from interpretation.",
    recommendedDriver: "claude_terminal",
    compatibleDrivers: LOCAL_TERMINAL_DRIVER_IDS,
    setupInputs: commonResearchInputs,
    suggestedAccess: {
      workspace: "Use a research workspace for notes, source lists, excerpts, and drafts.",
      data: "Prefer user-approved and primary sources; record source limits and uncertainty.",
    },
    suggestedSkills: ["source-research", "brief-writing"],
    suggestedFirstTasks: ["Gather the strongest primary and secondary sources for this question."],
    outputContract: {
      summary: "Source-aware briefing",
      format: "Research briefing",
      requiredSections: ["Question", "Evidence", "Source Notes", "Preliminary Takeaways"],
    },
  },
  {
    id: "source-checker",
    version: VERSION,
    name: "Source Checker",
    category: "research",
    description: "Checks source quality, contradictions, and citation support.",
    bestFor: ["Citation review", "fact checking", "contradiction analysis"],
    instructionTemplate: "Act as a source checker: verify claims against sources, identify contradictions, and rate confidence without overstating certainty.",
    recommendedDriver: "claude_terminal",
    compatibleDrivers: LOCAL_TERMINAL_DRIVER_IDS,
    setupInputs: commonResearchInputs,
    suggestedAccess: {
      workspace: "Use read access to drafts, claim lists, source lists, and citation notes.",
      data: "Use primary sources where possible and preserve contradiction, recency, and credibility notes.",
    },
    suggestedSkills: ["fact-checking", "citation-review"],
    suggestedFirstTasks: ["Audit these claims against the cited sources and flag weak support."],
    outputContract: {
      summary: "Source quality and contradiction notes",
      format: "Source audit",
      requiredSections: ["Claims Checked", "Source Quality", "Contradictions", "Confidence"],
    },
  },
  {
    id: "synthesizer",
    version: VERSION,
    name: "Synthesizer",
    category: "research",
    description: "Combines research threads into a final recommendation with tradeoffs.",
    bestFor: ["Final synthesis", "decision memos", "recommendations"],
    instructionTemplate: "Act as a synthesizer: combine verified findings into a concise recommendation, explain tradeoffs, and state confidence.",
    recommendedDriver: "claude_terminal",
    compatibleDrivers: LOCAL_TERMINAL_DRIVER_IDS,
    setupInputs: commonResearchInputs,
    suggestedAccess: {
      workspace: "Use the research workspace containing analyst findings, source checks, and draft notes.",
      data: "Use verified findings and source-quality notes; do not introduce unsupported claims late in synthesis.",
    },
    suggestedSkills: ["synthesis", "recommendation-writing"],
    suggestedFirstTasks: ["Synthesize the findings into a recommendation and note confidence."],
    outputContract: {
      summary: "Final synthesis and recommendation",
      format: "Synthesis memo",
      requiredSections: ["Bottom Line", "Evidence", "Tradeoffs", "Recommendation"],
    },
  },
  {
    id: "project-operator",
    version: VERSION,
    name: "Project Operator",
    category: "operations",
    description: "Coordinates work, decomposes tasks, tracks blockers, and summarizes progress.",
    bestFor: ["Team coordination", "task assignment", "progress reporting"],
    instructionTemplate: "Act as a project operator: clarify mission, split work into owned tasks, coordinate handoffs, and keep progress visible.",
    recommendedDriver: "codex_terminal",
    compatibleDrivers: LOCAL_CODING_DRIVER_IDS,
    setupInputs: [
      {
        id: "mission",
        label: "Mission",
        type: "textarea",
        required: false,
        description: "The team mission or project outcome this operator should coordinate.",
        placeholder: "Ship the collaboration MVP task safely...",
      },
    ],
    suggestedAccess: {
      workspace: "Use the project workspace or group context needed to coordinate task state.",
      data: "Use task docs, user mission details, teammate updates, and blocker reports.",
    },
    suggestedSkills: ["planning", "status-reporting"],
    suggestedFirstTasks: ["Break this mission into concrete tasks and assign owners."],
    outputContract: {
      summary: "Plan, task assignment, coordination summary",
      format: "Coordination update",
      requiredSections: ["Plan", BOARD_TASKS_CREATED_SECTION, "Blockers", "Next Steps"],
    },
  },
];

export const GROUP_TEMPLATES: GroupTemplate[] = [
  {
    id: "software-development-team",
    version: VERSION,
    name: "Software Development Team",
    description: "A coordinated product engineering team for planning, implementation, review, and verification.",
    suggestedGroupName: "software-development-team",
    defaultDriverPolicy: {
      driver: "codex_terminal",
      applyToCompatibleRoles: true,
      fallback: "role_recommendation",
    },
    roleSlots: [
      roleSlot("project-operator", "Project Operator", "project-operator", true, ["Plan work", "Coordinate handoffs", "Track blockers"], ["coordinator"]),
      roleSlot("backend-engineer", "Backend Engineer", "backend-engineer", true, ["Implement backend changes", "Add focused tests"], ["owner"]),
      roleSlot("code-reviewer", "Code Reviewer", "code-reviewer", true, ["Review changes", "Report bugs and test gaps"], ["quality_check"]),
      roleSlot("frontend-engineer", "Frontend Engineer", "frontend-engineer", false, ["Implement UI changes", "Verify browser behavior"], ["owner"]),
      roleSlot("qa-tester", "QA Tester", "qa-tester", false, ["Create test plans", "Verify regressions"], ["quality_check"]),
      roleSlot("devops-engineer", "DevOps Engineer", "backend-engineer", false, ["Review deployment impact", "Flag operational risks"], ["operations"]),
    ],
    workflow: {
      steps: ["Plan", "Implement", "Review", "Verify", "Summarize"],
      description: "Plan -> implement -> review -> verify -> summarize",
      coordinatorRoleSlotId: "project-operator",
    },
    kickoffTemplate: {
      title: "Software development kickoff",
      body: "Mission: {{mission}}\n\nWorkflow: Plan -> implement -> review -> verify -> summarize.\n\nPlease wait for the user to provide the first concrete work item before starting execution.",
    },
    outputContract: {
      summary: "Implementation summary, changed files, verification notes",
      format: "Engineering delivery report",
      requiredSections: ["Summary", "Changed Files", "Verification", "Risks"],
    },
    recommendedAccessModel: "Repository workspace access for new software agents; reused agents are not modified.",
  },
  {
    id: "financial-analysis-team",
    version: VERSION,
    name: "Financial Analysis Team",
    description: "A finance team for analysis framing, data work, valuation, and risk challenge.",
    suggestedGroupName: "financial-analysis-team",
    defaultDriverPolicy: {
      driver: "claude_terminal",
      applyToCompatibleRoles: true,
      fallback: "role_recommendation",
    },
    roleSlots: [
      roleSlot("lead-financial-analyst", "Lead Financial Analyst", "lead-financial-analyst", true, ["Frame the question", "Own the final memo"], ["coordinator"]),
      roleSlot("data-analyst", "Data Analyst", "data-analyst", true, ["Extract metrics", "Prepare tables and calculations"], ["owner"]),
      roleSlot("risk-reviewer", "Risk Reviewer", "risk-reviewer", true, ["Challenge assumptions", "Document downside risks"], ["quality_check", "risk_review"]),
      roleSlot("valuation-analyst", "Valuation Analyst", "valuation-analyst", false, ["Build valuation scenarios", "Explain sensitivities"], ["owner"]),
      roleSlot("macro-analyst", "Macro Analyst", "lead-financial-analyst", false, ["Add macro context", "Flag external market risks"], ["domain_expert"]),
    ],
    workflow: {
      steps: ["Define question", "Collect data", "Analyze", "Challenge risks", "Memo"],
      description: "Define question -> collect data -> analyze -> challenge risks -> memo",
      coordinatorRoleSlotId: "lead-financial-analyst",
    },
    kickoffTemplate: {
      title: "Financial analysis kickoff",
      body: "Mission: {{mission}}\n\nWorkflow: Define question -> collect data -> analyze -> challenge risks -> memo.\n\nThe team should wait for the user to confirm the first analysis question before starting work.",
    },
    outputContract: {
      summary: "Investment/analysis memo with assumptions and risks",
      format: "Financial analysis memo",
      requiredSections: ["Question", "Assumptions", "Analysis", "Risks", "Recommendation"],
    },
    recommendedAccessModel: "Use only user-approved data sources and clearly label assumptions.",
  },
  {
    id: "research-team",
    version: VERSION,
    name: "Research Team",
    description: "A research team for gathering evidence, checking sources, and producing a recommendation.",
    suggestedGroupName: "research-team",
    defaultDriverPolicy: {
      driver: "claude_terminal",
      applyToCompatibleRoles: true,
      fallback: "role_recommendation",
    },
    roleSlots: [
      roleSlot("research-analyst", "Research Analyst", "research-analyst", true, ["Gather evidence", "Prepare source-aware findings"], ["owner"]),
      roleSlot("source-checker", "Source Checker", "source-checker", true, ["Verify claims", "Flag weak or contradictory sources"], ["source_check", "quality_check"]),
      roleSlot("synthesizer", "Synthesizer", "synthesizer", true, ["Combine findings", "Produce final recommendation"], ["coordinator"]),
      roleSlot("editor", "Editor", "synthesizer", false, ["Tighten structure", "Improve clarity and audience fit"], ["quality_check"]),
      roleSlot("domain-expert", "Domain Expert", "research-analyst", false, ["Add domain-specific context", "Challenge broad claims"], ["domain_expert"]),
    ],
    workflow: {
      steps: ["Gather", "Verify", "Synthesize", "Recommend"],
      description: "Gather -> verify -> synthesize -> recommend",
      coordinatorRoleSlotId: "synthesizer",
    },
    kickoffTemplate: {
      title: "Research kickoff",
      body: "Mission: {{mission}}\n\nWorkflow: Gather -> verify -> synthesize -> recommend.\n\nPlease wait for the user to confirm the first research question before beginning substantive research.",
    },
    outputContract: {
      summary: "Briefing memo, comparison table, recommendation",
      format: "Research briefing memo",
      requiredSections: ["Briefing", "Comparison Table", "Source Quality", "Recommendation"],
    },
    recommendedAccessModel: "Prefer primary sources and preserve source-quality notes in the final brief.",
  },
];

export function getRoleTemplate(id: string): RoleTemplate | undefined {
  return ROLE_TEMPLATES.find((template) => template.id === id);
}

export function getGroupTemplate(id: string): GroupTemplate | undefined {
  return GROUP_TEMPLATES.find((template) => template.id === id);
}

export function generateAgentName(roleTemplateOrName: Pick<RoleTemplate, "name"> | string, slotId?: string): string {
  const base = typeof roleTemplateOrName === "string" ? roleTemplateOrName : roleTemplateOrName.name;
  return slugify(slotId || base);
}

export function generateGroupName(groupTemplateOrName: Pick<GroupTemplate, "suggestedGroupName" | "name"> | string, mission?: string): string {
  const base = typeof groupTemplateOrName === "string" ? groupTemplateOrName : groupTemplateOrName.suggestedGroupName || groupTemplateOrName.name;
  const missionSlug = mission ? slugify(mission).split("-").slice(0, 4).join("-") : "";
  return missionSlug ? `${slugify(base)}-${missionSlug}` : slugify(base);
}

function roleSlot(
  id: string,
  label: string,
  roleTemplateId: string,
  required: boolean,
  responsibilities: string[],
  workflowRoleKeys: string[] = [],
): RoleSlot {
  return {
    id,
    label,
    roleTemplateId,
    required,
    defaultAgentName: generateAgentName(label, id),
    responsibilities,
    ...(workflowRoleKeys.length ? { workflowRoleKeys } : {}),
  };
}

function slugify(value: string): string {
  const slug = value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return slug || "untitled";
}
