import { describe, expect, it } from "vitest";

import { mergePreviewIntoMessages, appendIncrementalMessages, mergeFetchedMessages, messagesMissingFromPrevious, maxCachedSeq, upsertConfirmedMessage, OPTIMISTIC_SERVER_SEQ, type MessagesByConv } from "./messages";
import type { ChatMessage } from "../api/choruz-types";

function msg(
  id: string,
  conv: string,
  seq: number,
  content = id,
): ChatMessage {
  return {
    id,
    workspace_id: "ws-1",
    conversation_id: conv,
    sender_id: "user-1",
    content,
    content_type: "text",
    metadata: {},
    edited_at: null,
    edited_by: null,
    server_seq: seq,
    idempotency_key: id,
    created_at: "2026-04-08T12:00:00Z",
  };
}

function optimistic(key: string, conv: string, content: string): ChatMessage {
  return { ...msg(key, conv, OPTIMISTIC_SERVER_SEQ, content), idempotency_key: key };
}

function confirmed(id: string, conv: string, seq: number, key: string, content: string): ChatMessage {
  return { ...msg(id, conv, seq, content), idempotency_key: key };
}

describe("upsertConfirmedMessage", () => {
  it("replaces the optimistic entry that the server message confirms", () => {
    const cached = [msg("m1", "c1", 1), optimistic("key-1", "c1", "hello")];

    const result = upsertConfirmedMessage(cached, confirmed("srv-1", "c1", 2, "key-1", "hello"));

    expect(result.map((m) => m.id)).toEqual(["m1", "srv-1"]);
  });

  it("drops the optimistic entry when the server copy is already cached", () => {
    // A bootstrap preview merged the server row before the sync feed's
    // message.created arrived — the second arrival must not leave two bubbles.
    const cached = [
      msg("m1", "c1", 1),
      optimistic("key-1", "c1", "hello"),
      confirmed("srv-1", "c1", 2, "key-1", "hello"),
    ];

    const result = upsertConfirmedMessage(cached, confirmed("srv-1", "c1", 2, "key-1", "hello"));

    expect(result.map((m) => m.id)).toEqual(["m1", "srv-1"]);
  });

  it("returns the same reference when the message is already cached and nothing is pending", () => {
    const cached = [msg("m1", "c1", 1), confirmed("srv-1", "c1", 2, "key-1", "hello")];

    expect(upsertConfirmedMessage(cached, confirmed("srv-1", "c1", 2, "key-1", "hello"))).toBe(cached);
  });

  it("appends a message from another sender", () => {
    const cached = [msg("m1", "c1", 1)];

    const result = upsertConfirmedMessage(cached, msg("m2", "c1", 2));

    expect(result.map((m) => m.id)).toEqual(["m1", "m2"]);
  });
});

describe("mergePreviewIntoMessages", () => {
  // --- REGRESSION for the original bug ------------------------------------
  it("does NOT truncate an existing 15-message history when preview has 1 msg with same last seq", () => {
    const history: ChatMessage[] = Array.from({ length: 15 }, (_, i) =>
      msg(`m${i + 1}`, "c1", i + 1),
    );
    const existing: MessagesByConv = { c1: history };
    const preview = { c1: [history[14]] }; // same last msg as before

    const result = mergePreviewIntoMessages(existing, preview);

    expect(result).toBe(existing); // unchanged reference — no re-render noise
    expect(result.c1).toHaveLength(15);
    expect(result.c1[0].id).toBe("m1");
    expect(result.c1[14].id).toBe("m15");
  });

  it("does NOT truncate history even if preview seq is *older* than cache tail", () => {
    // Race: snapshot was taken before the client already received m15 via WS.
    const history = [msg("m14", "c1", 14), msg("m15", "c1", 15)];
    const existing: MessagesByConv = { c1: history };
    const preview = { c1: [msg("m14", "c1", 14)] };

    const result = mergePreviewIntoMessages(existing, preview);

    expect(result.c1).toHaveLength(2);
    expect(result.c1[1].id).toBe("m15");
  });

  // --- Append-if-newer -----------------------------------------------------
  it("appends when preview contains a strictly newer message", () => {
    const existing: MessagesByConv = {
      c1: [msg("m1", "c1", 1), msg("m2", "c1", 2)],
    };
    const preview = { c1: [msg("m3", "c1", 3)] };

    const result = mergePreviewIntoMessages(existing, preview);

    expect(result.c1.map((m) => m.id)).toEqual(["m1", "m2", "m3"]);
    expect(result).not.toBe(existing); // changed
  });

  it("appends multiple new messages when preview has more than one new item", () => {
    const existing: MessagesByConv = { c1: [msg("m1", "c1", 1)] };
    const preview = {
      c1: [msg("m2", "c1", 2), msg("m3", "c1", 3)],
    };

    const result = mergePreviewIntoMessages(existing, preview);

    expect(result.c1.map((m) => m.id)).toEqual(["m1", "m2", "m3"]);
  });

  it("filters out messages already present in the cache tail (dedup by id)", () => {
    const existing: MessagesByConv = {
      c1: [msg("m1", "c1", 1), msg("m2", "c1", 2)],
    };
    const preview = { c1: [msg("m2", "c1", 2)] };

    const result = mergePreviewIntoMessages(existing, preview);

    expect(result).toBe(existing);
    expect(result.c1).toHaveLength(2);
  });

  // --- Optimistic reconciliation -------------------------------------------
  it("replaces the optimistic entry when the preview carries its confirmation", () => {
    const existing: MessagesByConv = {
      c1: [msg("m1", "c1", 1), optimistic("key-1", "c1", "hello")],
    };
    const preview = { c1: [confirmed("srv-1", "c1", 2, "key-1", "hello")] };

    const result = mergePreviewIntoMessages(existing, preview);

    expect(result.c1.map((m) => m.id)).toEqual(["m1", "srv-1"]);
    expect(result.c1[1].server_seq).toBe(2);
  });

  it("still replaces the optimistic entry when the confirmation is not newer than the cache tail", () => {
    // The optimistic message was inserted before a later server message
    // arrived on the feed, so the confirmation's seq is behind the tail.
    const existing: MessagesByConv = {
      c1: [msg("m1", "c1", 1), optimistic("key-1", "c1", "hello"), msg("m3", "c1", 3)],
    };
    const preview = { c1: [confirmed("srv-2", "c1", 2, "key-1", "hello")] };

    const result = mergePreviewIntoMessages(existing, preview);

    expect(result.c1.map((m) => m.id)).toEqual(["m1", "srv-2", "m3"]);
  });

  // --- First-load cases ----------------------------------------------------
  it("seeds an empty conversation with the preview verbatim", () => {
    const existing: MessagesByConv = {};
    const preview = { c1: [msg("m1", "c1", 1)] };

    const result = mergePreviewIntoMessages(existing, preview);

    expect(result.c1).toHaveLength(1);
    expect(result.c1[0].id).toBe("m1");
  });

  it("handles empty / missing previews gracefully", () => {
    const existing: MessagesByConv = { c1: [msg("m1", "c1", 1)] };
    const preview = { c1: [], c2: undefined };

    const result = mergePreviewIntoMessages(existing, preview);

    expect(result).toBe(existing);
  });

  // --- Independence across conversations ----------------------------------
  it("updates one conversation without disturbing others", () => {
    const existing: MessagesByConv = {
      c1: [msg("m1", "c1", 1), msg("m2", "c1", 2)],
      c2: [msg("x1", "c2", 1)],
    };
    const preview = { c2: [msg("x2", "c2", 2)] };

    const result = mergePreviewIntoMessages(existing, preview);

    expect(result.c1).toBe(existing.c1); // same reference — untouched
    expect(result.c2.map((m) => m.id)).toEqual(["x1", "x2"]);
  });
});

// ===========================================================================
// appendIncrementalMessages (Level 2: since_seq incremental merge)
// ===========================================================================

describe("appendIncrementalMessages", () => {
  it("drops the optimistic entry when its confirmation is already cached", () => {
    const existing: MessagesByConv = {
      c1: [msg("m1", "c1", 1), optimistic("key-1", "c1", "hello"), confirmed("srv-1", "c1", 2, "key-1", "hello")],
    };

    const result = appendIncrementalMessages(existing, "c1", [confirmed("srv-1", "c1", 2, "key-1", "hello")]);

    expect(result.c1.map((m) => m.id)).toEqual(["m1", "srv-1"]);
  });

  it("appends new messages to the tail of an existing conv", () => {
    const existing: MessagesByConv = {
      c1: [msg("m1", "c1", 1), msg("m2", "c1", 2)],
    };
    const result = appendIncrementalMessages(existing, "c1", [msg("m3", "c1", 3), msg("m4", "c1", 4)]);
    expect(result.c1.map((m) => m.id)).toEqual(["m1", "m2", "m3", "m4"]);
  });

  it("deduplicates by id", () => {
    const existing: MessagesByConv = { c1: [msg("m1", "c1", 1)] };
    const result = appendIncrementalMessages(existing, "c1", [msg("m1", "c1", 1), msg("m2", "c1", 2)]);
    expect(result.c1.map((m) => m.id)).toEqual(["m1", "m2"]);
  });

  it("filters out messages with seq <= cache tail", () => {
    const existing: MessagesByConv = { c1: [msg("m3", "c1", 3)] };
    const result = appendIncrementalMessages(existing, "c1", [msg("m1", "c1", 1), msg("m2", "c1", 2)]);
    expect(result).toBe(existing); // nothing appended
  });

  it("returns same reference when no new messages", () => {
    const existing: MessagesByConv = { c1: [msg("m1", "c1", 1)] };
    const result = appendIncrementalMessages(existing, "c1", []);
    expect(result).toBe(existing);
  });

  it("seeds an empty conv with all new messages", () => {
    const existing: MessagesByConv = {};
    const result = appendIncrementalMessages(existing, "c1", [msg("m1", "c1", 1)]);
    expect(result.c1.map((m) => m.id)).toEqual(["m1"]);
  });

  it("ignores optimistic tail messages when appending incremental replies", () => {
    const optimistic = msg("optimistic", "c1", Number.MAX_SAFE_INTEGER);
    const existing: MessagesByConv = { c1: [msg("m1", "c1", 1), optimistic] };
    const result = appendIncrementalMessages(existing, "c1", [msg("m2", "c1", 2)]);
    expect(result.c1.map((m) => m.id)).toEqual(["m1", "optimistic", "m2"]);
  });

  it("replaces matching optimistic messages when poll returns the confirmed server event", () => {
    const optimistic = {
      ...msg("optimistic-local", "c1", Number.MAX_SAFE_INTEGER),
      idempotency_key: "cmid-1",
    };
    const confirmed = {
      ...msg("server-event-1", "c1", 2),
      idempotency_key: "cmid-1",
    };
    const existing: MessagesByConv = { c1: [msg("m1", "c1", 1), optimistic] };
    const result = appendIncrementalMessages(existing, "c1", [confirmed]);
    expect(result.c1.map((m) => m.id)).toEqual(["m1", "server-event-1"]);
    expect(result.c1[1].server_seq).toBe(2);
  });
});

// ===========================================================================
// mergeFetchedMessages (Level 2: full first-page merge)
// ===========================================================================

describe("mergeFetchedMessages", () => {
  it("backfills older fetched history when a newer event is already cached", () => {
    const existing: MessagesByConv = { c1: [msg("m10", "c1", 10)] };
    const fetched = Array.from({ length: 10 }, (_, i) => msg(`m${i + 1}`, "c1", i + 1));

    const result = mergeFetchedMessages(existing, "c1", fetched);

    expect(result.c1.map((m) => m.id)).toEqual([
      "m1",
      "m2",
      "m3",
      "m4",
      "m5",
      "m6",
      "m7",
      "m8",
      "m9",
      "m10",
    ]);
  });

  it("replaces a matching optimistic message with fetched server confirmation", () => {
    const optimistic = {
      ...msg("optimistic-local", "c1", Number.MAX_SAFE_INTEGER),
      idempotency_key: "cmid-1",
    };
    const confirmed = {
      ...msg("server-event-1", "c1", 2),
      idempotency_key: "cmid-1",
    };
    const existing: MessagesByConv = { c1: [msg("m1", "c1", 1), optimistic] };

    const result = mergeFetchedMessages(existing, "c1", [msg("m1", "c1", 1), confirmed]);

    expect(result.c1.map((m) => m.id)).toEqual(["m1", "server-event-1"]);
    expect(result.c1[1].server_seq).toBe(2);
  });

  it("returns the existing reference when fetched messages are already cached", () => {
    const existing: MessagesByConv = { c1: [msg("m1", "c1", 1), msg("m2", "c1", 2)] };
    const result = mergeFetchedMessages(existing, "c1", [msg("m1", "c1", 1), msg("m2", "c1", 2)]);
    expect(result).toBe(existing);
  });
});

// ===========================================================================
// messagesMissingFromPrevious (IndexedDB write-through delta)
// ===========================================================================

describe("messagesMissingFromPrevious", () => {
  it("returns all messages for an empty previous cache", () => {
    const next = [msg("m1", "c1", 1), msg("m2", "c1", 2)];

    expect(messagesMissingFromPrevious(undefined, next)).toEqual(next);
  });

  it("keeps backfilled older messages when the cached tail already exists", () => {
    const previous = [msg("m50", "c1", 50)];
    const next = [msg("m1", "c1", 1), msg("m2", "c1", 2), msg("m50", "c1", 50)];

    expect(messagesMissingFromPrevious(previous, next).map((m) => m.id)).toEqual([
      "m1",
      "m2",
    ]);
  });

  it("returns no delta when the same messages are already cached", () => {
    const previous = [msg("m1", "c1", 1), msg("m2", "c1", 2)];
    const next = [msg("m1", "c1", 1), msg("m2", "c1", 2)];

    expect(messagesMissingFromPrevious(previous, next)).toEqual([]);
  });
});

// ===========================================================================
// maxCachedSeq
// ===========================================================================

describe("maxCachedSeq", () => {
  it("returns the last server_seq in the cache", () => {
    const cache: MessagesByConv = { c1: [msg("m1", "c1", 5), msg("m2", "c1", 10)] };
    expect(maxCachedSeq(cache, "c1")).toBe(10);
  });

  it("returns 0 for missing conv", () => {
    expect(maxCachedSeq({}, "c1")).toBe(0);
  });

  it("returns 0 for empty array", () => {
    expect(maxCachedSeq({ c1: [] }, "c1")).toBe(0);
  });

  it("ignores optimistic tail messages", () => {
    const cache: MessagesByConv = {
      c1: [msg("m1", "c1", 5), msg("optimistic", "c1", Number.MAX_SAFE_INTEGER)],
    };
    expect(maxCachedSeq(cache, "c1")).toBe(5);
  });
});
