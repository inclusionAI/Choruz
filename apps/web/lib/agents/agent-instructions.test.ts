import { describe, expect, it } from "vitest";
import {
  INSTRUCTION_FIELDS,
  emptyFields,
  fieldsToMarkdown,
  markdownToFields,
  type AgentInstructionFields,
} from "./agent-instructions";

describe("emptyFields", () => {
  it("returns all 5 fields as empty strings", () => {
    const f = emptyFields();
    const values = Object.values(f);
    expect(values).toHaveLength(5);
    expect(values.every((v) => v === "")).toBe(true);
  });
});

describe("INSTRUCTION_FIELDS", () => {
  it("carries a label and help text for every field", () => {
    expect(INSTRUCTION_FIELDS.map((meta) => meta.key)).toEqual(Object.keys(emptyFields()));
    for (const meta of INSTRUCTION_FIELDS) {
      expect(meta.label.trim()).not.toBe("");
      expect(meta.help.trim()).not.toBe("");
    }
  });
});

// fieldsToMarkdown ------------------------------------------------------------

describe("fieldsToMarkdown", () => {
  it("returns empty string when all fields are empty", () => {
    expect(fieldsToMarkdown(emptyFields())).toBe("");
  });

  it("emits one ## section per non-empty field, joined by --- separators", () => {
    const f = emptyFields();
    f.role = "I am a tester";
    f.workflow = "1. Test";
    const md = fieldsToMarkdown(f);
    expect(md).toContain("## Role\n\nI am a tester");
    expect(md).toContain("## Workflow\n\n1. Test");
    expect(md.match(/^---$/gm)).toHaveLength(1);
  });

  it("skips fields containing only whitespace", () => {
    const f = emptyFields();
    f.role = "Role";
    f.projectContext = "   \n  ";
    const md = fieldsToMarkdown(f);
    expect(md).toContain("Role");
    expect(md).not.toContain("Project Context");
  });

  it("trims content within each section", () => {
    const f = emptyFields();
    f.role = "  hello  \n";
    expect(fieldsToMarkdown(f)).toBe("## Role\n\nhello");
  });
});

// markdownToFields ------------------------------------------------------------

describe("markdownToFields", () => {
  it("parses a Role heading", () => {
    const f = markdownToFields("## Role\n\nI am a tester\n");
    expect(f.role).toBe("I am a tester");
  });

  it("matches headings case-insensitively", () => {
    const f = markdownToFields("## ROLE\n\nfoo");
    expect(f.role).toBe("foo");
  });

  it("ignores unknown headings", () => {
    const f = markdownToFields("## My Custom Heading\n\nfoo");
    expect(f.role).toBe("## My Custom Heading\n\nfoo");
  });

  it("folds the thirteen-section layout into the five fields, keeping each old heading as a bold line", () => {
    const legacy = [
      "## Identity & Role\n\nI am the reviewer.",
      "## Goals & Responsibilities\n\nCatch bugs.",
      "## Project Context\n\nRust monorepo.",
      "## Communication & Triggers\n\nTrigger on @mentions.",
      "## Allowed Operations\n\nRead, run tests.",
      "## Forbidden Operations\n\nDo NOT modify code.",
      "## SOP / Workflow\n\n1. Read.\n2. Report.",
      "## Work Style & Output\n\nConcise.",
      "## Collaboration & Reporting\n\nUse [DONE].",
      "## Escalation Protocol\n\nSecurity → @leader.",
      "## Definition of Done\n\nTests pass.",
      "## Error Handling\n\nLog and notify.",
      "## Scheduled Tasks\n\nNightly lints.",
    ].join("\n\n---\n\n");
    const f = markdownToFields(legacy);
    expect(f.role).toBe("**Identity & Role**\n\nI am the reviewer.\n\n**Goals & Responsibilities**\n\nCatch bugs.");
    expect(f.projectContext).toBe("Rust monorepo.");
    expect(f.boundaries).toBe(
      "**Allowed Operations**\n\nRead, run tests.\n\n**Forbidden Operations**\n\nDo NOT modify code.\n\n**Work Style & Output**\n\nConcise.",
    );
    expect(f.workflow).toBe(
      "**SOP / Workflow**\n\n1. Read.\n2. Report.\n\n**Definition of Done**\n\nTests pass.\n\n**Error Handling**\n\nLog and notify.\n\n**Scheduled Tasks**\n\nNightly lints.",
    );
    expect(f.collaboration).toBe(
      "**Communication & Triggers**\n\nTrigger on @mentions.\n\n**Collaboration & Reporting**\n\nUse [DONE].\n\n**Escalation Protocol**\n\nSecurity → @leader.",
    );
    expect(markdownToFields(fieldsToMarkdown(f))).toEqual(f);
  });

  it("drops everything before '## Your Role' when choruz-protocol marker is present", () => {
    const md = `<!-- choruz-protocol: v1 -->
# Platform stuff
## Some protocol section
protocol text

## Your Role

## Role
I am the test agent
`;
    const f = markdownToFields(md);
    expect(f.role).toBe("I am the test agent");
  });

  it("strips managed role delimiters from a canonical template", () => {
    const md = `<!-- choruz-protocol: v2-maildir -->
# Platform stuff

## Your Role

<!-- choruz-role:start -->
## Role
I am the test agent

## Project Context
apps/web
<!-- choruz-role:end -->
`;
    const f = markdownToFields(md);
    expect(f.role).toBe("I am the test agent");
    expect(f.projectContext).toBe("apps/web");
  });

  it("puts un-matched content into role if no recognised heading exists", () => {
    const md = "Just some prose without headings.";
    expect(markdownToFields(md).role).toBe("Just some prose without headings.");
  });

  it("returns all-empty fields for a totally empty input", () => {
    const f = markdownToFields("");
    expect(Object.values(f).every((v) => v === "")).toBe(true);
  });
});

// round-trip ------------------------------------------------------------------

describe("round-trip fieldsToMarkdown → markdownToFields", () => {
  it("preserves a fully-populated record", () => {
    const original: AgentInstructionFields = {
      role: "I am the senior code reviewer.\nCatch bugs.",
      projectContext: "Rust + Next.js monorepo.",
      boundaries: "Read, run tests.\nDo NOT modify code.\nConcise. Chinese.",
      workflow: "1. Read.\n2. Test.\n3. Report.\nDone when tests pass.",
      collaboration: "Trigger on @mentions.\nSecurity → @leader.",
    };
    const md = fieldsToMarkdown(original);
    const parsed = markdownToFields(md);
    expect(parsed).toEqual(original);
  });

  it("preserves a partially-populated record (empty fields stay empty)", () => {
    const original = emptyFields();
    original.role = "tester";
    original.workflow = "find issues";
    const parsed = markdownToFields(fieldsToMarkdown(original));
    expect(parsed.role).toBe("tester");
    expect(parsed.workflow).toBe("find issues");
    expect(parsed.boundaries).toBe("");
    expect(parsed.collaboration).toBe("");
  });
});
