import { describe, expect, it } from "vitest";

import type { ChatMessage } from "../api/choruz-types";
import {
  OPTIMISTIC_SERVER_SEQ,
  isBroadcastThreadReply,
  isThreadReply,
  mergeThreadReplies,
  partitionThreadMessages,
  resolveThreadRoot,
  rollbackOptimisticMessage,
} from "./threads";

let seq = 0;
function msg(
  id: string,
  metadata: Record<string, unknown> = {},
  overrides: Partial<ChatMessage> = {},
): ChatMessage {
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
    ...overrides,
  };
}

describe("isThreadReply", () => {
  it("requires JSON boolean true plus a non-empty reply_to_id", () => {
    expect(isThreadReply(msg("a", { thread: true, reply_to_id: "root" }))).toBe(true);
    // String "true" is NOT a thread — mirrors the server's jsonb equality.
    expect(isThreadReply(msg("b", { thread: "true", reply_to_id: "root" }))).toBe(false);
    expect(isThreadReply(msg("c", { thread: true }))).toBe(false);
    expect(isThreadReply(msg("d", { thread: true, reply_to_id: "" }))).toBe(false);
    // Legacy quote-reply: reply_to_id without the thread flag.
    expect(isThreadReply(msg("e", { reply_to_id: "root" }))).toBe(false);
  });
});

describe("isBroadcastThreadReply", () => {
  it("requires the thread flag AND broadcast === true", () => {
    expect(
      isBroadcastThreadReply(msg("a", { thread: true, reply_to_id: "r", broadcast: true })),
    ).toBe(true);
    expect(isBroadcastThreadReply(msg("b", { thread: true, reply_to_id: "r" }))).toBe(false);
    // broadcast without thread flag is a normal message, not a thread reply
    expect(isBroadcastThreadReply(msg("c", { reply_to_id: "r", broadcast: true }))).toBe(false);
  });
});

describe("resolveThreadRoot", () => {
  it("walks reply chains to the first non-thread-reply target", () => {
    const root = msg("root");
    const r1 = msg("r1", { thread: true, reply_to_id: "root" });
    const r2 = msg("r2", { thread: true, reply_to_id: "r1" });
    const byId = new Map([["root", root], ["r1", r1], ["r2", r2]]);
    expect(resolveThreadRoot(r1, byId)).toBe("root");
    expect(resolveThreadRoot(r2, byId)).toBe("root");
  });

  it("treats a legacy quote-reply target as the root itself", () => {
    const quote = msg("quote", { reply_to_id: "older" });
    const r1 = msg("r1", { thread: true, reply_to_id: "quote" });
    const byId = new Map([["quote", quote], ["r1", r1]]);
    expect(resolveThreadRoot(r1, byId)).toBe("quote");
  });

  it("resolves to the deepest reachable id when history is not loaded", () => {
    const r1 = msg("r1", { thread: true, reply_to_id: "unloaded" });
    expect(resolveThreadRoot(r1, new Map([["r1", r1]]))).toBe("unloaded");
  });

  it("does not loop forever on malformed cycles", () => {
    const a = msg("a", { thread: true, reply_to_id: "b" });
    const b = msg("b", { thread: true, reply_to_id: "a" });
    const byId = new Map([["a", a], ["b", b]]);
    expect(typeof resolveThreadRoot(a, byId)).toBe("string");
  });

  it("returns null for non-thread messages", () => {
    expect(resolveThreadRoot(msg("a"), new Map())).toBe(null);
  });
});

describe("partitionThreadMessages", () => {
  it("hides quiet replies from the timeline and rolls them up on the root", () => {
    const root = msg("root");
    const quiet = msg("q1", { thread: true, reply_to_id: "root" }, { sender_id: "bob" });
    const normal = msg("n1");
    const { timeline, rollups } = partitionThreadMessages([root, quiet, normal]);

    expect(timeline.map((m) => m.id)).toEqual(["root", "n1"]);
    const rollup = rollups.get("root");
    expect(rollup?.replyCount).toBe(1);
    expect(rollup?.replies.map((m) => m.id)).toEqual(["q1"]);
    expect(rollup?.participantIds).toEqual(["bob"]);
    expect(rollup?.lastReplyAt).toBe(quiet.created_at);
  });

  it("shows broadcast replies in BOTH the timeline and the thread", () => {
    const root = msg("root");
    const bcast = msg("b1", { thread: true, reply_to_id: "root", broadcast: true });
    const { timeline, rollups } = partitionThreadMessages([root, bcast]);

    expect(timeline.map((m) => m.id)).toEqual(["root", "b1"]);
    expect(rollups.get("root")?.replies.map((m) => m.id)).toEqual(["b1"]);
  });

  it("keeps legacy quote-replies as plain timeline messages", () => {
    const root = msg("root");
    const quote = msg("q1", { reply_to_id: "root" });
    const { timeline, rollups } = partitionThreadMessages([root, quote]);

    expect(timeline.map((m) => m.id)).toEqual(["root", "q1"]);
    expect(rollups.size).toBe(0);
  });

  it("attaches reply-to-reply messages to the canonical root", () => {
    const root = msg("root");
    const r1 = msg("r1", { thread: true, reply_to_id: "root" }, { sender_id: "bob" });
    const r2 = msg("r2", { thread: true, reply_to_id: "r1" }, { sender_id: "carol" });
    const { rollups } = partitionThreadMessages([root, r1, r2]);

    expect(rollups.size).toBe(1);
    const rollup = rollups.get("root");
    expect(rollup?.replyCount).toBe(2);
    expect(rollup?.participantIds).toEqual(["bob", "carol"]);
    expect(rollup?.lastReplyAt).toBe(r2.created_at);
  });

  it("dedups participants and tracks the latest reply time", () => {
    const root = msg("root");
    const r1 = msg("r1", { thread: true, reply_to_id: "root" }, { sender_id: "bob" });
    const r2 = msg("r2", { thread: true, reply_to_id: "root" }, { sender_id: "bob" });
    const { rollups } = partitionThreadMessages([root, r1, r2]);

    const rollup = rollups.get("root");
    expect(rollup?.participantIds).toEqual(["bob"]);
    expect(rollup?.lastReplyAt).toBe(r2.created_at);
  });
});

describe("mergeThreadReplies", () => {
  it("appends only unseen messages and re-sorts by server_seq", () => {
    const a = msg("a");
    const c = msg("c");
    const b = msg("b");
    // a.seq < c.seq < b.seq by construction order; fetched contains a dup + b
    const merged = mergeThreadReplies([a, c], [a, b]);
    expect(merged.map((m) => m.id)).toEqual(["a", "c", "b"]);
  });

  it("returns the same array reference when nothing is new", () => {
    const a = msg("a");
    const existing = [a];
    expect(mergeThreadReplies(existing, [a])).toBe(existing);
  });

  it("replaces a still-optimistic row when the fetched message carries its idempotency_key", () => {
    // fetchThread landing inside the POST→WS-confirm window must not pin
    // a ghost duplicate of the reply being sent.
    const optimistic = msg("temp-key", {}, {
      server_seq: OPTIMISTIC_SERVER_SEQ,
      idempotency_key: "key-1",
    });
    const confirmed = msg("real-id", {}, { idempotency_key: "key-1" });
    const merged = mergeThreadReplies([optimistic], [confirmed]);
    expect(merged.map((m) => m.id)).toEqual(["real-id"]);
  });

  it("does not treat confirmed rows with a matching key as optimistic", () => {
    // Same key but a REAL seq (WS already replaced it) — must be kept and
    // the identical fetched copy deduped by id.
    const confirmed = msg("real-id", {}, { idempotency_key: "key-1" });
    const merged = mergeThreadReplies([confirmed], [confirmed]);
    expect(merged).toEqual([confirmed]);
  });
});

describe("rollbackOptimisticMessage", () => {
  it("removes the optimistic row for the failed key", () => {
    const optimistic = msg("temp", {}, {
      server_seq: OPTIMISTIC_SERVER_SEQ,
      idempotency_key: "key-1",
    });
    const other = msg("other");
    expect(rollbackOptimisticMessage([optimistic, other], "key-1")).toEqual([other]);
  });

  it("does NOT delete a WS-confirmed message that already replaced the optimistic row", () => {
    // Gate-5 F01 regression pin: late POST failure after the WS
    // confirmation swapped in the real message (same key, real seq) —
    // rolling back by key alone would destroy committed data.
    const confirmed = msg("real-id", {}, { idempotency_key: "key-1" });
    expect(rollbackOptimisticMessage([confirmed], "key-1")).toEqual([confirmed]);
  });
});
