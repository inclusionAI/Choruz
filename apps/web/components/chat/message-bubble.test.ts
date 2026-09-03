import { describe, expect, it } from "vitest";
import {
  shouldGroup,
  stripChoruzTags,
  stripTuiChars,
} from "./message-bubble";
import type { ChatMessage } from "../../lib/api/choruz-types";

// stripTuiChars ---------------------------------------------------------------

describe("stripTuiChars", () => {
  it("removes box-drawing-only divider lines", () => {
    expect(stripTuiChars("hello\n────────\nworld")).toBe("hello\nworld");
  });

  it("removes prompt indicator lines (❯ / ›)", () => {
    expect(stripTuiChars("hello\n❯\nworld")).toBe("hello\nworld");
    expect(stripTuiChars("hello\n› try this\nworld")).toBe("hello\nworld");
  });

  it("removes progress-bullet lines", () => {
    expect(stripTuiChars("ok\n✽ ✻ ✳\nstuff")).toBe("ok\nstuff");
  });

  it("removes tool-call header lines (Write/Read/Bash/...)", () => {
    expect(stripTuiChars("answer\nBash(ls -la)\nmore")).toBe("answer\nmore");
    expect(stripTuiChars("answer\nWrite(/tmp/x)\nmore")).toBe("answer\nmore");
  });

  it("removes token-status lines containing arrows", () => {
    expect(stripTuiChars("ok\n· 12 tokens ↓ 3 tokens ↑\ndone")).toBe("ok\ndone");
  });

  it("removes claude-code status hints", () => {
    expect(stripTuiChars("hi\nesc to interrupt\n? for help\nbye")).toBe("hi\nbye");
  });

  it("strips leading ⏺ from non-removed lines", () => {
    expect(stripTuiChars("⏺ first line\n⏺ second")).toBe("first line\nsecond");
  });

  it("collapses runs of 3+ box-drawing chars when intra-line", () => {
    expect(stripTuiChars("hello ────── world")).toBe("hello  world");
  });

  it("keeps normal content unchanged", () => {
    expect(stripTuiChars("Just a normal sentence.")).toBe("Just a normal sentence.");
  });
});

// stripChoruzTags -------------------------------------------------------------

describe("stripChoruzTags", () => {
  it("removes paired CHORUZ_REPLY tags", () => {
    const inp = "{{CHORUZ_REPLY group=proj-team}}hello{{/CHORUZ_REPLY}}";
    expect(stripChoruzTags(inp)).toBe("hello");
  });

  it("removes CHORUZ_REPLY direct= variant", () => {
    const inp = "{{CHORUZ_REPLY direct=alice}}hi{{/CHORUZ_REPLY}}";
    expect(stripChoruzTags(inp)).toBe("hi");
  });

  it("removes self-closing CHORUZ_SHARE_FILE tags", () => {
    expect(stripChoruzTags("see {{CHORUZ_SHARE_FILE path=src/main.rs}} please")).toBe(
      "see  please",
    );
  });

  it("removes CHORUZ_PROVISION tags", () => {
    expect(stripChoruzTags("{{CHORUZ_PROVISION name=tester}}go")).toBe("go");
  });

  it("trims surrounding whitespace", () => {
    expect(stripChoruzTags("\n\n  hello  \n\n")).toBe("hello");
  });

  it("leaves clean text untouched", () => {
    expect(stripChoruzTags("hello world")).toBe("hello world");
  });
});

// shouldGroup -----------------------------------------------------------------

function msg(over: Partial<ChatMessage> = {}): ChatMessage {
  return {
    id: "m1",
    workspace_id: "w",
    conversation_id: "c",
    sender_id: "alice",
    content: "hi",
    content_type: "text",
    metadata: {},
    server_seq: 1,
    idempotency_key: "k",
    created_at: "2026-05-13T10:00:00.000Z",
    ...over,
  } as unknown as ChatMessage;
}

describe("shouldGroup", () => {
  it("returns false when there is no previous message", () => {
    expect(shouldGroup(null, msg())).toBe(false);
  });

  it("returns true when same sender within 2 minutes", () => {
    const prev = msg({ created_at: "2026-05-13T10:00:00.000Z" });
    const curr = msg({ created_at: "2026-05-13T10:01:30.000Z" });
    expect(shouldGroup(prev, curr)).toBe(true);
  });

  it("returns false when same sender but gap >= 2 minutes", () => {
    const prev = msg({ created_at: "2026-05-13T10:00:00.000Z" });
    const curr = msg({ created_at: "2026-05-13T10:02:00.000Z" });
    expect(shouldGroup(prev, curr)).toBe(false);
  });

  it("returns false when senders differ", () => {
    const prev = msg({ sender_id: "alice" });
    const curr = msg({ sender_id: "bob", created_at: "2026-05-13T10:00:30.000Z" });
    expect(shouldGroup(prev, curr)).toBe(false);
  });

  it("returns false when either side is a system message", () => {
    const prev = msg({ content_type: "system" });
    const curr = msg({ created_at: "2026-05-13T10:00:30.000Z" });
    expect(shouldGroup(prev, curr)).toBe(false);

    const prev2 = msg();
    const curr2 = msg({ content_type: "system", created_at: "2026-05-13T10:00:30.000Z" });
    expect(shouldGroup(prev2, curr2)).toBe(false);
  });
});
