import { describe, expect, it } from "vitest";

import type { ChatMessage } from "../api/choruz-types";
import {
  applyLocallyViewed,
  buildUnreadMap,
  clearConversationUnread,
  createUnreadCommitGate,
  registerLocallyViewed,
  wsUnreadEffect,
  type LocallyViewedRegistry,
} from "./thread-unreads";

function msg(metadata: Record<string, unknown>): ChatMessage {
  return {
    id: "m-1",
    workspace_id: "ws",
    conversation_id: "conv",
    sender_id: "bob",
    content: "hi",
    content_type: "text",
    metadata,
    edited_at: null,
    edited_by: null,
    server_seq: 1,
    idempotency_key: "k",
    created_at: new Date(1700000000000).toISOString(),
  };
}

describe("buildUnreadMap", () => {
  it("maps rows including the thread counter, defaulting to 0", () => {
    const map = buildUnreadMap([
      { conversation_id: "a", unread_count: 2, mention_count: 1, thread_unread_count: 3 },
      { conversation_id: "b", unread_count: 0, mention_count: 0 },
    ]);
    expect(map["a"]).toEqual({ unread: 2, mentions: 1, threadUnread: 3 });
    expect(map["b"]).toEqual({ unread: 0, mentions: 0, threadUnread: 0 });
  });
});

describe("clearConversationUnread", () => {
  it("zeroes unread/mentions but PRESERVES the thread counter", () => {
    const map = {
      a: { unread: 5, mentions: 2, threadUnread: 4 },
    };
    const next = clearConversationUnread(map, "a");
    expect(next["a"]).toEqual({ unread: 0, mentions: 0, threadUnread: 4 });
  });

  it("creates a zeroed entry for an unknown conversation", () => {
    const next = clearConversationUnread({}, "new");
    expect(next["new"].unread).toBe(0);
    expect(next["new"].mentions).toBe(0);
  });
});

describe("createUnreadCommitGate", () => {
  // Pins the Gate-8 regression: the gate must order on the last COMMITTED
  // seq, never the last claimed one.
  it("a burned claim does not veto a concurrent successful commit", () => {
    const gate = createUnreadCommitGate();
    const unreadsSeq = gate.claim(); // 1 — /v1/unreads, slow but will succeed
    gate.claim(); // 2 — snapshot fetch that FAILS (claim burned, no commit)
    expect(gate.tryCommit(unreadsSeq)).toBe(true);
  });

  it("rejects responses older than the last committed one", () => {
    const gate = createUnreadCommitGate();
    const older = gate.claim(); // 1
    const newer = gate.claim(); // 2
    expect(gate.tryCommit(newer)).toBe(true);
    expect(gate.tryCommit(older)).toBe(false);
    // Re-committing the same seq is also rejected (idempotent guard).
    expect(gate.tryCommit(newer)).toBe(false);
  });

  it("claimedSeq exposes the latest claim for registry registration", () => {
    const gate = createUnreadCommitGate();
    expect(gate.claimedSeq()).toBe(0);
    gate.claim();
    gate.claim();
    expect(gate.claimedSeq()).toBe(2);
  });
});

describe("applyLocallyViewed", () => {
  // Pins the R5→R6 Critical badge regression and its R7 refinements: the
  // registry papers over the POST /view race window, is consumed by the
  // first response that post-dates the registration, and is NOT consumed
  // by an older in-flight response.
  it("clears registered conversations and consumes entries on a post-registration response", () => {
    const registry: LocallyViewedRegistry = new Map();
    registerLocallyViewed(registry, "a", 1);
    const map = { a: { unread: 3, mentions: 1, threadUnread: 2 } };
    const next = applyLocallyViewed(map, registry, 2);
    expect(next["a"]).toEqual({ unread: 0, mentions: 0, threadUnread: 2 });
    expect(registry.has("a")).toBe(false);
  });

  it("does NOT re-suppress on later responses once consumed (genuine new unreads survive)", () => {
    const registry: LocallyViewedRegistry = new Map();
    registerLocallyViewed(registry, "a", 1);
    applyLocallyViewed({ a: { unread: 1, mentions: 0 } }, registry, 2);
    // New genuine unread arrives; the next refresh must keep it.
    const later = applyLocallyViewed({ a: { unread: 4, mentions: 2 } }, registry, 3);
    expect(later["a"].unread).toBe(4);
    expect(later["a"].mentions).toBe(2);
  });

  it("an older in-flight response applies the clear but does not consume the entry", () => {
    const registry: LocallyViewedRegistry = new Map();
    // Viewed while request seq 5 was already in flight.
    registerLocallyViewed(registry, "a", 5);
    const stale = applyLocallyViewed({ a: { unread: 2, mentions: 0 } }, registry, 5);
    expect(stale["a"].unread).toBe(0);
    expect(registry.has("a")).toBe(true);
    // The first response that post-dates the view consumes it.
    const fresh = applyLocallyViewed({ a: { unread: 0, mentions: 0 } }, registry, 6);
    expect(fresh["a"].unread).toBe(0);
    expect(registry.has("a")).toBe(false);
  });
});

describe("wsUnreadEffect", () => {
  it("normal messages bump the conversation badge only", () => {
    expect(wsUnreadEffect(msg({}))).toEqual({
      bumpConversationUnread: true,
      refreshThreadUnread: false,
    });
  });

  it("legacy quote-replies behave like normal messages", () => {
    expect(wsUnreadEffect(msg({ reply_to_id: "root" }))).toEqual({
      bumpConversationUnread: true,
      refreshThreadUnread: false,
    });
  });

  it("quiet thread replies skip the conversation badge and refresh threads", () => {
    expect(wsUnreadEffect(msg({ thread: true, reply_to_id: "root" }))).toEqual({
      bumpConversationUnread: false,
      refreshThreadUnread: true,
    });
  });

  it("broadcast thread replies bump the badge AND refresh threads (server counts them in the LATERAL too)", () => {
    expect(
      wsUnreadEffect(msg({ thread: true, reply_to_id: "root", broadcast: true })),
    ).toEqual({
      bumpConversationUnread: true,
      refreshThreadUnread: true,
    });
  });

  it("string discriminators are not threads (mirrors the server's jsonb equality)", () => {
    expect(wsUnreadEffect(msg({ thread: "true", reply_to_id: "root" }))).toEqual({
      bumpConversationUnread: true,
      refreshThreadUnread: false,
    });
  });
});
