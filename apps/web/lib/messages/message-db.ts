// ---------------------------------------------------------------------------
// IndexedDB message cache using Dexie.
//
// Provides a local, persistent store of chat messages so the user sees
// history instantly on page load (no network wait) and only fetches
// incremental updates from the server.
//
// Schema: messages table keyed by [conversation_id+server_seq] for fast
// per-conversation range queries.
// ---------------------------------------------------------------------------

import Dexie from "dexie";
import type { ChatMessage } from "../api/choruz-types";
import { OPTIMISTIC_SERVER_SEQ, type MessagesByConv } from "./messages";
import { trace } from "../api/choruz-trace";

// IndexedDB failures are benign (server is the source of truth and the chat
// path will re-fetch over HTTP) but should still be observable so ops can
// track how often the local cache is unavailable — quota, incognito mode,
// schema-version upgrades, etc.
function reportIdbFail(op: string, err: unknown): void {
  try {
    trace.event("indexeddb_fallback", {
      op,
      error: err instanceof Error ? err.message : String(err),
    });
  } catch {
    // never let telemetry failure break the caller
  }
}

// ---------------------------------------------------------------------------
// Database
// ---------------------------------------------------------------------------

class MessageDatabase extends Dexie {
  messages!: Dexie.Table<ChatMessage, [string, number]>;
  syncState!: Dexie.Table<DashboardSyncState, string>;

  constructor() {
    super("choruz_messages");
    this.version(1).stores({
      // Compound primary key: [conversation_id, server_seq]
      // Index on conversation_id for per-conv queries.
      messages: "[conversation_id+server_seq], conversation_id",
    });
    this.version(2).stores({
      messages: "[conversation_id+server_seq], conversation_id",
      syncState: "&principal_id",
    });
  }
}

export type DashboardSyncState = {
  principal_id: string;
  device_id: string;
  ack_cursor: number;
};

let _db: MessageDatabase | null = null;

function getDb(): MessageDatabase {
  if (!_db) {
    _db = new MessageDatabase();
  }
  return _db;
}

/** Close and discard the singleton. Used by tests to get a fresh DB. */
export async function resetMessageDb(): Promise<void> {
  if (_db) {
    _db.close();
    _db = null;
  }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/**
 * Persist messages to IndexedDB (upsert by [conversation_id, server_seq]).
 * Optimistic messages (server_seq === OPTIMISTIC_SERVER_SEQ) are skipped.
 */
export async function persistMessages(msgs: ChatMessage[]): Promise<void> {
  if (typeof indexedDB === "undefined") return; // SSR guard
  try {
    const real = msgs.filter((m) => m.server_seq < OPTIMISTIC_SERVER_SEQ);
    if (real.length === 0) return;
    await getDb().messages.bulkPut(real);
  } catch (err) {
    reportIdbFail("persist", err);
  }
}

/**
 * Load all cached messages for a conversation from IndexedDB, ordered by
 * server_seq ascending.
 */
export async function loadConversationMessages(
  conversationId: string,
): Promise<ChatMessage[]> {
  if (typeof indexedDB === "undefined") return [];
  try {
    // Query via the compound primary key [conversation_id+server_seq] so
    // results are inherently ordered by server_seq ascending.
    return await getDb().messages
      .where("[conversation_id+server_seq]")
      .between(
        [conversationId, Dexie.minKey],
        [conversationId, Dexie.maxKey],
      )
      .toArray();
  } catch (err) {
    reportIdbFail("load_conversation", err);
    return [];
  }
}

/**
 * Load the latest N messages for each conversation that has cached data.
 * Returns a MessagesByConv map suitable for seeding React state on cold start.
 *
 * Only loads conversations present in `convIds` to avoid pulling stale data
 * for deleted conversations.
 */
export async function loadAllCachedMessages(
  convIds: string[],
): Promise<MessagesByConv> {
  if (typeof indexedDB === "undefined") return {};
  try {
    const db = getDb();
    const result: MessagesByConv = {};
    // Batch: fetch all rows for the requested conversations.
    const all = await db.messages
      .where("conversation_id")
      .anyOf(convIds)
      .sortBy("server_seq");
    for (const msg of all) {
      (result[msg.conversation_id] ??= []).push(msg);
    }
    return result;
  } catch (err) {
    reportIdbFail("load_all", err);
    return {};
  }
}

/**
 * Return the maximum server_seq stored in IndexedDB for a conversation,
 * or 0 if no cached messages exist.
 */
export async function maxPersistedSeq(
  conversationId: string,
): Promise<number> {
  if (typeof indexedDB === "undefined") return 0;
  try {
    // Use compound primary key range query so .last() returns the entry
    // with the highest server_seq for this conversation.
    const last = await getDb().messages
      .where("[conversation_id+server_seq]")
      .between(
        [conversationId, Dexie.minKey],
        [conversationId, Dexie.maxKey],
      )
      .last();
    return last?.server_seq ?? 0;
  } catch (err) {
    reportIdbFail("max_seq", err);
    return 0;
  }
}

export async function loadDashboardSyncState(
  principalId: string,
  bootstrapCursor: number,
): Promise<DashboardSyncState> {
  const fallback = {
    principal_id: principalId,
    device_id: globalThis.crypto?.randomUUID?.() ?? `browser-${Date.now()}-${Math.random()}`,
    ack_cursor: bootstrapCursor,
  };
  if (typeof indexedDB === "undefined") return fallback;
  try {
    const stored = await getDb().syncState.get(principalId);
    if (stored) {
      // A browser can outlive a restored or recreated backend database. When
      // its cursor is ahead of the bootstrap snapshot's authoritative head,
      // that cursor belongs to the old feed and would make every WebSocket
      // handshake fail forever. The snapshot already contains all state
      // through bootstrapCursor, so register a fresh device from there.
      if (stored.ack_cursor > bootstrapCursor) {
        await getDb().syncState.put(fallback);
        return fallback;
      }
      return { ...stored, ack_cursor: Math.max(stored.ack_cursor, bootstrapCursor) };
    }
    await getDb().syncState.put(fallback);
    return fallback;
  } catch (err) {
    reportIdbFail("load_sync_state", err);
    return fallback;
  }
}

export async function persistDashboardSyncCursor(
  state: DashboardSyncState,
  cursor: number,
): Promise<DashboardSyncState> {
  const next = { ...state, ack_cursor: Math.max(state.ack_cursor, cursor) };
  if (typeof indexedDB === "undefined") return next;
  try {
    await getDb().syncState.put(next);
  } catch (err) {
    reportIdbFail("persist_sync_cursor", err);
  }
  return next;
}
