// ---------------------------------------------------------------------------
// Structured agent instruction fields
// Five sections make up the role block of an agent's instruction file; the
// Choruz protocol is injected by the backend and never shown in the UI.
// ---------------------------------------------------------------------------

export type AgentInstructionFields = {
  /** Who the agent is and what it owns: identity, expertise, goals. */
  role: string;
  /** Facts about the codebase: stack, key paths, where things live. */
  projectContext: string;
  /** What it may do, what it must never do, how its output should look. */
  boundaries: string;
  /** The steps it follows, what counts as done, what to do on failure. */
  workflow: string;
  /** Who it talks to, when it is triggered, when to escalate. */
  collaboration: string;
};

/** Field metadata for rendering the UI */
export type FieldMeta = {
  key: keyof AgentInstructionFields;
  label: string;
  /** Shown behind the info icon next to the label. */
  help: string;
  placeholder: string;
  rows: number;
};

export const INSTRUCTION_FIELDS: FieldMeta[] = [
  {
    key: "role",
    label: "Role",
    help: "Who this agent is and what it is responsible for: expertise, backstory, the outcomes it owns.",
    placeholder: "e.g. You are a senior code reviewer. You own code quality across all PRs: catch bugs and security issues before merge.",
    rows: 3,
  },
  {
    key: "projectContext",
    label: "Project Context",
    help: "Facts the agent needs about the codebase: stack, key paths, where things live.",
    placeholder: "e.g. Rust + Next.js monorepo. Backend: crates/, frontend: apps/web/",
    rows: 2,
  },
  {
    key: "boundaries",
    label: "Boundaries",
    help: "What it may do, what it must never do, and how its output should look: language, format, length.",
    placeholder: "e.g. May read files, run tests and linters. Never modify code, push, or commit secrets. Reply in Chinese with file:line references, concise.",
    rows: 3,
  },
  {
    key: "workflow",
    label: "Workflow",
    help: "The steps it follows, what counts as done, and what to do when a step fails.",
    placeholder: "e.g. 1. Read diff 2. Run tests 3. Check security 4. Post verdict. Done when tests pass and the verdict is posted. If tests cannot run, report the blocker instead of guessing.",
    rows: 3,
  },
  {
    key: "collaboration",
    label: "Collaboration",
    help: "Who it talks to, what triggers it, and when to escalate. The [DONE]/[BLOCKED] format and @mention mechanics are already part of the platform protocol.",
    placeholder: "e.g. Respond to review requests and @mentions. Report results to the submitter. Security findings go to @leader immediately; unclear requirements: ask @leader.",
    rows: 3,
  },
];

const SECTION_HEADINGS: Record<keyof AgentInstructionFields, string> = {
  role: "Role",
  projectContext: "Project Context",
  boundaries: "Boundaries",
  workflow: "Workflow",
  collaboration: "Collaboration",
};

/** Default empty fields */
export function emptyFields(): AgentInstructionFields {
  return {
    role: "",
    projectContext: "",
    boundaries: "",
    workflow: "",
    collaboration: "",
  };
}

/** Assemble structured fields into a single CLAUDE.md string (without the Choruz protocol). */
export function fieldsToMarkdown(fields: AgentInstructionFields): string {
  return INSTRUCTION_FIELDS
    .map((meta) => ({ heading: SECTION_HEADINGS[meta.key], content: fields[meta.key] }))
    .filter((s) => s.content.trim())
    .map((s) => `## ${s.heading}\n\n${s.content.trim()}`)
    .join("\n\n---\n\n");
}

/**
 * Where each `##` heading lands. The five current headings map to their own
 * field; the headings of the earlier thirteen-section layout fold into the
 * field that absorbed them, so an agent written under that layout still opens
 * in the editor with nothing lost.
 */
const HEADING_MAP: Record<string, keyof AgentInstructionFields> = {
  "role": "role",
  "identity & role": "role",
  "identity": "role",
  "goals & responsibilities": "role",
  "goals": "role",
  "project context": "projectContext",
  "project context & paths": "projectContext",
  "boundaries": "boundaries",
  "allowed operations": "boundaries",
  "forbidden operations": "boundaries",
  "work style & output format": "boundaries",
  "work style & output": "boundaries",
  "work style": "boundaries",
  "output format": "boundaries",
  "workflow": "workflow",
  "sop / workflow": "workflow",
  "sop": "workflow",
  "definition of done": "workflow",
  "completion criteria": "workflow",
  "error handling": "workflow",
  "scheduled tasks": "workflow",
  "collaboration": "collaboration",
  "communication & triggers": "collaboration",
  "communication": "collaboration",
  "collaboration & reporting": "collaboration",
  "escalation protocol": "collaboration",
  "escalation": "collaboration",
};

const CURRENT_HEADINGS = new Set(Object.values(SECTION_HEADINGS).map((h) => h.toLowerCase()));

/**
 * Parse a CLAUDE.md back into structured fields.
 * Looks for ## headings that match our section names.
 * Content between Choruz protocol markers is skipped.
 */
export function markdownToFields(md: string): AgentInstructionFields {
  const fields = emptyFields();

  // Strip Choruz protocol block if present
  const protocolStart = md.indexOf("<!-- choruz-protocol");
  const roleMarker = "## Your Role";
  let cleaned = md;
  if (protocolStart !== -1) {
    const roleIdx = md.indexOf(roleMarker);
    if (roleIdx !== -1) {
      // Everything after "## Your Role" is user content
      cleaned = md.slice(roleIdx + roleMarker.length);
    } else {
      // No role marker — try to find our structured sections
      cleaned = md;
    }
  }
  cleaned = cleaned
    .replace(/<!-- choruz-role:start -->\s*/g, "")
    .replace(/\s*<!-- choruz-role:end -->/g, "");

  // Split by ## headings
  const parts = cleaned.split(/^##\s+/m);
  for (const part of parts) {
    const newlineIdx = part.indexOf("\n");
    if (newlineIdx === -1) continue;
    const headingText = part.slice(0, newlineIdx).trim();
    const heading = headingText.toLowerCase();
    const content = part.slice(newlineIdx + 1).replace(/^---\s*$/m, "").trim();
    const key = HEADING_MAP[heading];
    if (!key || !content) continue;
    // A section from the earlier layout keeps its heading as a bold line so
    // "allowed" and "forbidden" stay distinguishable once they share a field.
    const block = CURRENT_HEADINGS.has(heading) ? content : `**${headingText}**\n\n${content}`;
    fields[key] = fields[key] ? `${fields[key]}\n\n${block}` : block;
  }

  // If no structured sections found, put everything in role as fallback
  const hasAny = Object.values(fields).some((v) => v.trim());
  if (!hasAny && cleaned.trim()) {
    fields.role = cleaned.trim();
  }

  return fields;
}
