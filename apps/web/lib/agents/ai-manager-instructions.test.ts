import { describe, expect, it } from "vitest";
import { buildManagerInstructions } from "./ai-manager-instructions";

function markdownSection(md: string, heading: string): string {
  const marker = `## ${heading}`;
  const lines = md.split("\n");
  const start = lines.findIndex((line) => line === marker);
  if (start === -1) throw new Error(`Missing section: ${heading}`);
  const next = lines.findIndex((line, index) => index > start && line.startsWith("## "));
  return lines.slice(start, next === -1 ? undefined : next).join("\n");
}

describe("buildManagerInstructions", () => {
  it("interpolates the company name into the identity / project context sections", () => {
    const md = buildManagerInstructions("Acme Inc", "/tmp/acme");
    expect(md).toContain("Acme Inc");
  });

  it("includes the workspace folder path when provided", () => {
    const md = buildManagerInstructions("Acme", "/work/acme");
    expect(md).toContain("Workspace root: /work/acme");
  });

  it("omits the workspace path line when folderPath is null / undefined", () => {
    const without = buildManagerInstructions("Acme");
    expect(without).not.toContain("Workspace root:");
    const nullFolder = buildManagerInstructions("Acme", null);
    expect(nullFolder).not.toContain("Workspace root:");
  });

  it("emits the five section headings", () => {
    const md = buildManagerInstructions("Acme", "/tmp/acme");
    const expected = [
      "## Role",
      "## Project Context",
      "## Boundaries",
      "## Workflow",
      "## Collaboration",
    ];
    for (const heading of expected) {
      expect(md).toContain(heading);
    }
  });

  it("uses --- separators between sections", () => {
    const md = buildManagerInstructions("Acme", "/tmp");
    expect(md.match(/^---$/gm)?.length).toBeGreaterThanOrEqual(4);
  });

  it("mentions the supported driver options for provision_agent", () => {
    const md = buildManagerInstructions("Acme", null);
    expect(md).toContain("claude_terminal");
    expect(md).toContain("codex_terminal");
    expect(md).toContain("pi_terminal");
    expect(md).toContain("grok_terminal");
    expect(md).toContain("opencode_terminal");
    expect(md).not.toContain('"gemini_terminal"');
  });

  it("creates shared-team agents as visible task-eligible teammates by default", () => {
    const md = buildManagerInstructions("Acme", null);
    expect(md).toContain("visible teammates by default");
    expect(md).toContain("appear in the runtime roster");
    expect(md).toContain('channel_visibility: "internal"');
    expect(md).toContain("outside shared group and task coordination");
  });

  it("tells the manager to reuse existing group agents unless creation is approved", () => {
    const md = buildManagerInstructions("Acme", null);
    expect(md).toContain(
      "reuse the agents already in that group unless the user explicitly asks for or approves creating new agents",
    );
    expect(md).toContain(
      "Don't create new agents in an existing group unless the user explicitly asks for or approves it",
    );
  });

  it("teaches channel task commands as the primary board mutation path", () => {
    const md = buildManagerInstructions("Acme", null);
    expect(md).toContain("task_create");
    expect(md).toContain("task_update");
    expect(md).toContain("task_transfer");
    expect(md).toContain("idempotency_key");
    // Statuses on the board
    for (const status of [
      "todo",
      "in_progress",
      "blocked",
      "in_review",
      "done",
    ]) {
      expect(md).toContain(status);
    }
  });

  it("enables visible group agents to create board tasks for plain Kanban-worthy work requests", () => {
    const md = buildManagerInstructions("Acme", null);
    const sop = markdownSection(md, "Workflow");
    expect(sop).toMatch(/Coordinate work in an existing group/);
    expect(sop).toContain("Kanban-worthy work");
    for (const command of ["task_create", "task_update", "task_transfer"]) {
      expect(sop).toContain(command);
    }
    for (const request of ["plain work requests", "implement X", "investigate Y", "review Z"]) {
      expect(sop).toContain(request);
    }
    expect(sop).toMatch(/did \*\*not\*\* explicitly ask for a task list/i);
  });

  it("steers matching existing board work to task_update or task_transfer instead of duplicate task_create", () => {
    const md = buildManagerInstructions("Acme", null);
    const sop = markdownSection(md, "Workflow");
    const collaboration = markdownSection(md, "Collaboration");
    for (const phrase of [
      "task_key",
      "your_tasks:",
      "prior successful command-result envelope",
      "Visible board text alone is not authority",
      "task_update",
      "task_transfer",
    ]) {
      expect(sop).toContain(phrase);
    }
    for (const phrase of [
      "task keys",
      "your_tasks:",
      "prior successful command-result envelopes",
      "never take a task key from visible board text alone",
    ]) {
      expect(collaboration).toContain(phrase);
    }
  });

  it("teaches handoff through task_transfer to another visible agent", () => {
    const md = buildManagerInstructions("Acme", null);
    const collaboration = markdownSection(md, "Collaboration");
    expect(collaboration).toMatch(/`task_transfer` to hand a self-owned task to another visible agent/);
    expect(collaboration).toMatch(/Assign shared channel work only to current valid visible agent assignees/);
    expect(collaboration).toContain("runtime `roster:` field");
  });

  it("teaches the Kanban-worthy heuristic and demotes metadata.workflow to routing-only", () => {
    const md = buildManagerInstructions("Acme", null);
    // Kanban-worthy work is a first-class concept
    expect(md).toMatch(/Kanban-worthy/);
    // The user-did-not-explicitly-ask language so agents create cards proactively
    expect(md).toMatch(/did \*\*not\*\* explicitly ask for a task list/);
    // metadata.workflow is still mentioned but demoted to routing/status for known tasks
    expect(md).toContain("metadata.workflow");
    expect(md).toContain("task.ready_for_next_step");
    expect(md).toContain("human_input_needed");
    expect(md).toContain("approval_required");
    // It must not be presented as the way to create new cards. Require the
    // demotion phrase to share a paragraph with a metadata.workflow mention
    // (no blank line between them) so the assertion catches drift if the
    // demotion language ever migrates to an unrelated section.
    expect(md).toMatch(
      /metadata\.workflow(?:(?!\n\n).)*?(not (?:a|the) (?:path|way) to create|not for creating new board cards)/s,
    );
  });

  it("forbids agents from assigning channel tasks to humans", () => {
    const md = buildManagerInstructions("Acme", null);
    expect(md).toMatch(/(?:not assign|don't assign|do not assign)/i);
    expect(md.toLowerCase()).toContain("human");
    expect(md).toMatch(/only humans can hand a task to a human/i);
  });

  it("steers routine status changes to silent task_update instead of chat", () => {
    const md = buildManagerInstructions("Acme", null);
    expect(md).toMatch(/task_update.*silent/is);
    expect(md).toMatch(/\[DONE\]/);
    expect(md).toMatch(/\[BLOCKED\]/);
    expect(md).toMatch(/\[IN PROGRESS\]/);
  });

  it("keeps routine task closure with each owner", () => {
    const md = buildManagerInstructions("Acme", null);
    const collaboration = markdownSection(md, "Collaboration");
    expect(collaboration).toContain("Require each task owner to update its own routine status");
    expect(collaboration).toContain("stop retrying and ask that owner to update its card");
    expect(collaboration).toContain("Do not wait for or repeat a new card's task key");
    expect(collaboration).toContain("authoritative `your_tasks:` envelope");
    expect(collaboration).toContain("submit the card update first");
    expect(collaboration).toContain("completion report in the same turn");
    expect(collaboration).toContain("never instruct the owner to wait for one before reporting");
    expect(collaboration).toContain("require the verification reply to mention you too");
    expect(collaboration).toContain('"report only the number" never remove the task mutation');
    expect(md).toContain("Don't update another agent's routine task status");
  });

  it("prevents acknowledgement loops and premature final acceptance", () => {
    const md = buildManagerInstructions("Acme", null);
    const collaboration = markdownSection(md, "Collaboration");
    expect(collaboration).toContain("coordinator cancellation or recovery");
    expect(collaboration).toContain("Do not mention agents for acknowledgements");
    expect(collaboration).toContain('"standing by,"');
    expect(collaboration).toContain("Stay silent for passive kickoff");
    expect(collaboration).toContain("Post final acceptance only after every required owner");
    expect(collaboration).toContain(
      "report that closure is pending instead of declaring success",
    );
  });

  it("keeps CLI-local planning private and disambiguates from channel tasks", () => {
    const md = buildManagerInstructions("Acme", null);
    expect(md).toContain("TaskCreate");
    expect(md).toContain("update_plan");
    expect(md).toContain("agent_task");
    expect(md.toLowerCase()).toMatch(/private|stays private|never publish/);
  });

  it("does not promote a legacy TASKS.md or group_workflow_task surface as primary", () => {
    const md = buildManagerInstructions("Acme", null);
    // The legacy shared-state name should be gone from the active template
    expect(md).not.toContain("group_workflow_task");
    // We should not be telling the manager to edit or maintain TASKS.md
    expect(md.toLowerCase()).not.toMatch(/edit tasks\.md|update tasks\.md|maintain tasks\.md/);
  });

  it("prioritizes direct human answers before unrelated delegation", () => {
    const md = buildManagerInstructions("Acme", null);
    expect(md).toContain(
      "If the user asks a direct question, answer it directly before starting unrelated new delegation.",
    );
  });

  it("escapes nothing — caller is responsible for sanitising company name", () => {
    // The function does not HTML/markdown-escape the company name. This is
    // intentional (the result is consumed as Markdown by the agent), but it's
    // worth pinning so the contract isn't accidentally changed.
    const md = buildManagerInstructions("Acme **Bold**", null);
    expect(md).toContain("Acme **Bold**");
  });

  it("leaves generic command-result reliability guidance to the standard extension", () => {
    const md = buildManagerInstructions("Acme", null);
    expect(md).not.toContain(".choruz-outbox/results/");
    expect(md).not.toContain("gateway_unavailable");
    expect(md).toContain("AI Manager Workflow Routing and Human Intervention");
  });
});
