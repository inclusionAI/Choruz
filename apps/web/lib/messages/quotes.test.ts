import { describe, expect, it } from "vitest";

import type { ChatMessage } from "../api/choruz-types";
import {
  collectMissingQuoteTargets,
  resolveQuoteTarget,
  type QuotedMessage,
} from "./quotes";

let seq = 0;
function msg(id: string, metadata: Record<string, unknown> = {}): ChatMessage {
  seq += 1;
  return {
    id,
    workspace_id: "ws",
    conversation_id: "conv",
    sender_id: "alice",
    content: `content-${id}`,
    content_type: "text",
    metadata,
    edited_at: null,
    edited_by: null,
    server_seq: seq,
    idempotency_key: `idem-${id}`,
    created_at: new Date(1700000000000 + seq * 1000).toISOString(),
  };
}

describe("collectMissingQuoteTargets", () => {
  it("returns reply targets absent from history, store, and in-flight set", () => {
    const reply = msg("r1", { reply_to_id: "old-1" });
    expect(collectMissingQuoteTargets([reply], new Map(), new Set())).toEqual(["old-1"]);
  });

  it("skips targets already loaded in history", () => {
    const original = msg("old-1");
    const reply = msg("r1", { reply_to_id: "old-1" });
    expect(collectMissingQuoteTargets([original, reply], new Map(), new Set())).toEqual([]);
  });

  it("skips targets already fetched (including 'missing' verdicts) or in flight", () => {
    const r1 = msg("r1", { reply_to_id: "fetched" });
    const r2 = msg("r2", { reply_to_id: "gone" });
    const r3 = msg("r3", { reply_to_id: "pending" });
    const quoted = new Map<string, QuotedMessage>([
      ["fetched", msg("fetched")],
      ["gone", "missing"],
    ]);
    expect(collectMissingQuoteTargets([r1, r2, r3], quoted, new Set(["pending"]))).toEqual([]);
  });

  it("dedups repeated targets and ignores malformed reply_to_id values", () => {
    const r1 = msg("r1", { reply_to_id: "old-1" });
    const r2 = msg("r2", { reply_to_id: "old-1" });
    const bad1 = msg("b1", { reply_to_id: "" });
    const bad2 = msg("b2", { reply_to_id: 42 });
    expect(
      collectMissingQuoteTargets([r1, r2, bad1, bad2], new Map(), new Set()),
    ).toEqual(["old-1"]);
  });
});

describe("resolveQuoteTarget", () => {
  it("prefers the live loaded message over the on-demand store", () => {
    const live = msg("t");
    const stale = msg("t");
    const byId = new Map([["t", live]]);
    const quoted = new Map<string, QuotedMessage>([["t", stale]]);
    expect(resolveQuoteTarget("t", byId, quoted)).toBe(live);
  });

  it("falls back to the on-demand store, including the missing verdict", () => {
    const fetched = msg("t");
    expect(
      resolveQuoteTarget("t", new Map(), new Map<string, QuotedMessage>([["t", fetched]])),
    ).toBe(fetched);
    expect(
      resolveQuoteTarget("gone", new Map(), new Map<string, QuotedMessage>([["gone", "missing"]])),
    ).toBe("missing");
    expect(resolveQuoteTarget("pending", new Map(), new Map())).toBeUndefined();
  });
});
