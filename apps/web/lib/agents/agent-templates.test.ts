import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

import { AI_MANAGER_WORKFLOW_EXTENSION } from "./ai-manager-workflow-extension";
import { buildManagerInstructions } from "./ai-manager-instructions";
import {
  composeAgentInstructionTemplate,
  CORE_PROTOCOL_FILE,
  STANDARD_EXTENSION_FILES,
} from "./agent-instruction-template";

const ROOT = path.resolve(__dirname, "..", "..", "..", "..", "agent-templates");
const read = (file: string) => readFileSync(path.join(ROOT, file), "utf-8");
const fragments = {
  coreProtocol: read(CORE_PROTOCOL_FILE),
  standardExtensions: STANDARD_EXTENSION_FILES.map(read),
};

describe.each([
  ["Claude", "agent-claude-md-template.md"],
  ["AGENTS.md-compatible", "agent-codex-md-template.md"],
])("%s modular agent instructions", (_, file) => {
  const shell = read(file);
  const rendered = composeAgentInstructionTemplate(shell, "You are the release reviewer.", fragments);

  it("renders the complete standard capability set and designed role", () => {
    expect(rendered).toMatch(/^<!-- choruz-bootstrap-version: 10 -->/);
    expect(rendered).toContain("[choruz-incoming]");
    for (const capability of [
      "roster:",
      "task_create",
      ".choruz-outbox/results/<message_id>.json",
      '"type":"share_file"',
      '"type":"provision_agent"',
      '"type":"create_group"',
      '"type":"set_cron"',
      "absolute file paths",
      "parallel editing",
    ]) {
      expect(rendered).toContain(capability);
    }
    expect(rendered).toContain("<!-- choruz-role:start -->\nYou are the release reviewer.\n<!-- choruz-role:end -->");
    expect(rendered).not.toMatch(/\{\{[A-Z_]+\}\}/);
  });

  it("keeps AI Manager workflow and human-intervention commands conditional", () => {
    expect(rendered).not.toContain("metadata.workflow");
    expect(rendered).not.toContain("human_input_needed");
    expect(rendered).not.toContain("approval_required");
    expect(AI_MANAGER_WORKFLOW_EXTENSION).toContain("metadata.workflow");
    expect(AI_MANAGER_WORKFLOW_EXTENSION).toContain("human_input_needed");
    expect(AI_MANAGER_WORKFLOW_EXTENSION).toContain("approval_required");

    const manager = composeAgentInstructionTemplate(
      shell,
      buildManagerInstructions("Acme"),
      fragments,
    );
    expect(manager).toContain("metadata.workflow");
    expect(manager).toContain("human_input_needed");
    expect(manager).toContain("approval_required");
  });

  it("documents reliable mutations without exposing sensitive data", () => {
    for (const field of ["command_type", "ok", "error_code", "idempotency_key", "emitted_at"]) {
      expect(rendered).toContain(field);
    }
    for (const excluded of ["tokens", "prompts", "hidden principal ids", "raw gateway diagnostics"]) {
      expect(rendered).toContain(excluded);
    }
  });

  it("teaches latest-state batching and action-only mentions", () => {
    expect(rendered).toContain("read them oldest to newest");
    expect(rendered).toContain("newest envelope is authoritative");
    expect(rendered).toContain("Never restart, reopen, duplicate, or redelegate");
    expect(rendered).toContain("correct the side effect instead of only reporting the new state");
    expect(rendered).toContain("Do not mention agents merely to acknowledge");
    expect(rendered).toContain('Treat passive kickoff, wait, and "stand by" messages as silence instructions');
    expect(rendered).toContain("Do not update another agent's routine status");
    expect(rendered).toContain("does not need to repeat a new card's `task_key`");
    expect(rendered).toContain("authoritative card from its incoming `your_tasks:` field");
    expect(rendered).toContain("completion report in the same turn");
    expect(rendered).toContain("never tell the owner to wait for one before sending that report");
    expect(rendered).toContain("verification reply must also `@mention` the coordinator");
    expect(rendered).toContain('"report only the number" limits the narrative payload');
    expect(rendered).toContain("an `@mention` completion report in the same turn");
    expect(rendered).toContain(
      "Do not publish final acceptance while required owners still report open work",
    );
  });
});

describe("template composer", () => {
  it("rejects incomplete shells instead of silently dropping a module", () => {
    expect(() => composeAgentInstructionTemplate("{{AGENT_INSTRUCTIONS}}", "role", fragments)).toThrow(
      "missing {{CHORUZ_CORE_PROTOCOL}}",
    );
  });
});
