import "fake-indexeddb/auto";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import Dexie from "dexie";

import {
  persistMessages,
  loadConversationMessages,
  loadAllCachedMessages,
  maxPersistedSeq,
  resetMessageDb,
  loadDashboardSyncState,
  persistDashboardSyncCursor,
} from "./message-db";
import type { ChatMessage } from "../api/choruz-types";

function msg(id: string, conv: string, seq: number): ChatMessage {
  return {
    id,
    workspace_id: "ws-1",
    conversation_id: conv,
    sender_id: "user-1",
    content: id,
    content_type: "text",
    metadata: {},
    edited_at: null,
    edited_by: null,
    server_seq: seq,
    idempotency_key: id,
    created_at: "2026-04-09T00:00:00Z",
  };
}

beforeEach(async () => {
  await resetMessageDb();
  await Dexie.delete("choruz_messages");
});

afterEach(async () => {
  await resetMessageDb();
  await Dexie.delete("choruz_messages");
});

describe("message-db IndexedDB cache", () => {
  it("persists and loads messages for a conversation", async () => {
    await persistMessages([msg("m1", "c1", 1), msg("m2", "c1", 2)]);

    const loaded = await loadConversationMessages("c1");
    expect(loaded.map((m) => m.id)).toEqual(["m1", "m2"]);
  });

  it("returns messages sorted by server_seq ascending", async () => {
    // Insert out of order.
    await persistMessages([msg("m3", "c1", 3), msg("m1", "c1", 1), msg("m2", "c1", 2)]);

    const loaded = await loadConversationMessages("c1");
    expect(loaded.map((m) => m.server_seq)).toEqual([1, 2, 3]);
  });

  it("upserts (does not duplicate) on repeated persist", async () => {
    await persistMessages([msg("m1", "c1", 1), msg("m2", "c1", 2)]);
    await persistMessages([msg("m2", "c1", 2), msg("m3", "c1", 3)]);

    const loaded = await loadConversationMessages("c1");
    expect(loaded.map((m) => m.id)).toEqual(["m1", "m2", "m3"]);
  });

  it("skips optimistic messages (MAX_SAFE_INTEGER seq)", async () => {
    await persistMessages([
      msg("m1", "c1", 1),
      msg("opt", "c1", Number.MAX_SAFE_INTEGER),
    ]);

    const loaded = await loadConversationMessages("c1");
    expect(loaded).toHaveLength(1);
    expect(loaded[0].id).toBe("m1");
  });

  it("loadAllCachedMessages returns grouped by conv", async () => {
    await persistMessages([
      msg("a1", "c1", 1),
      msg("a2", "c1", 2),
      msg("b1", "c2", 1),
    ]);

    const result = await loadAllCachedMessages(["c1", "c2", "c3"]);
    expect(Object.keys(result).sort()).toEqual(["c1", "c2"]);
    expect(result.c1.map((m) => m.id)).toEqual(["a1", "a2"]);
    expect(result.c2.map((m) => m.id)).toEqual(["b1"]);
    expect(result.c3).toBeUndefined();
  });

  it("maxPersistedSeq returns the highest seq for a conv", async () => {
    await persistMessages([msg("m1", "c1", 5), msg("m2", "c1", 10)]);

    expect(await maxPersistedSeq("c1")).toBe(10);
  });

  it("maxPersistedSeq returns 0 for unknown conv", async () => {
    expect(await maxPersistedSeq("nonexistent")).toBe(0);
  });

  it("loadConversationMessages returns empty for unknown conv", async () => {
    const loaded = await loadConversationMessages("nonexistent");
    expect(loaded).toEqual([]);
  });

  it("persists one monotonic dashboard cursor per principal and device", async () => {
    const first = await loadDashboardSyncState("user-1", 7);
    const saved = await persistDashboardSyncCursor(first, 12);
    await persistDashboardSyncCursor(saved, 9);

    const restored = await loadDashboardSyncState("user-1", 12);
    expect(restored.device_id).toBe(first.device_id);
    expect(restored.ack_cursor).toBe(12);
  });

  it("advances a restored device to a newer bounded bootstrap", async () => {
    const first = await loadDashboardSyncState("user-2", 3);
    const restored = await loadDashboardSyncState("user-2", 20);
    expect(restored.device_id).toBe(first.device_id);
    expect(restored.ack_cursor).toBe(20);
  });

  it("starts a fresh device when a restored backend is behind the browser cursor", async () => {
    const original = await loadDashboardSyncState("user-restored", 456);
    const restored = await loadDashboardSyncState("user-restored", 224);

    expect(restored.ack_cursor).toBe(224);
    expect(restored.device_id).not.toBe(original.device_id);
  });
});
