// ---------------------------------------------------------------------------
// Pure helpers for manipulating the per-conversation message cache
// (messagesByConv). Extracted so the merge logic can be unit-tested without
// mounting React components.
// ---------------------------------------------------------------------------

import type { ChatMessage } from "../api/choruz-types";

export type MessagesByConv = Record<string, ChatMessage[]>;

/** Sentinel server_seq carried by optimistic (not yet server-confirmed)
 * messages. THE single definition — every comparison against the sentinel
 * (===, <) must use this constant: chat-app's optimistic inserts, the
 * threads.ts sweep/rollback helpers, and message-db's persistence filter
 * all key correctness on the same value. */
export const OPTIMISTIC_SERVER_SEQ = Number.MAX_SAFE_INTEGER;

function isRealMessage(message: ChatMessage): boolean {
  return message.server_seq < OPTIMISTIC_SERVER_SEQ;
}

function lastRealServerSeq(messages: ChatMessage[]): number {
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    if (isRealMessage(messages[i])) return messages[i].server_seq;
  }
  return 0;
}

function lastRealMessage(messages: ChatMessage[]): ChatMessage | null {
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    if (isRealMessage(messages[i])) return messages[i];
  }
  return null;
}

/**
 * Index of the optimistic entry whose idempotency_key matches `message`, or -1.
 * A server message carries the client's idempotency_key as its own, so this is
 * how every merge path recognises the confirmation of an optimistic message.
 */
function optimisticIndexFor(messages: ChatMessage[], message: ChatMessage): number {
  if (!message.idempotency_key) return -1;
  return messages.findIndex(
    (m) => !isRealMessage(m) && m.idempotency_key === message.idempotency_key,
  );
}

/**
 * Insert one server-confirmed message into a conversation's cache, replacing
 * the optimistic entry it confirms. When the server copy is already cached
 * (a preview or fetch landed first) the optimistic entry is still removed, so
 * whichever path arrives second never leaves a duplicate bubble behind.
 *
 * Returns the same array reference when nothing changes.
 */
export function upsertConfirmedMessage(
  messages: ChatMessage[],
  message: ChatMessage,
): ChatMessage[] {
  const optimisticIdx = optimisticIndexFor(messages, message);
  const alreadyCached = messages.some((m) => m.id === message.id);
  if (alreadyCached) {
    if (optimisticIdx < 0) return messages;
    return messages.filter((_, idx) => idx !== optimisticIdx);
  }
  if (optimisticIdx >= 0) {
    const updated = [...messages];
    updated[optimisticIdx] = message;
    return updated;
  }
  return [...messages, message];
}

/**
 * Merge a "sidebar preview" payload (typically {conv_id: [last_msg]} from
 * /v1/console) into the existing messagesByConv cache WITHOUT truncating any
 * conversation's full history.
 *
 * Rules:
 *   - If the cache is empty for a conv, seed it with whatever the preview has.
 *   - A preview that confirms an optimistic message (same idempotency_key)
 *     replaces it in place, whatever its server_seq.
 *   - Otherwise APPEND-IF-NEWER: only messages whose server_seq is strictly
 *     greater than the last cached message AND whose id is not already the
 *     tail are added.
 *   - Never replace or truncate an existing conv's history.
 *
 * This is what prevents the "messages disappear after 10-30s" bug: the
 * periodic console snapshot used to replace [m1..m15] with [m15], wiping
 * everything the user had scrolled through.
 */
export function mergePreviewIntoMessages(
  existing: MessagesByConv,
  previewByConv: Record<string, ChatMessage[] | undefined>,
): MessagesByConv {
  let changed = false;
  const next: MessagesByConv = { ...existing };

  for (const [cid, preview] of Object.entries(previewByConv)) {
    if (!preview || preview.length === 0) continue;

    const cached = existing[cid] ?? [];
    if (cached.length === 0) {
      next[cid] = preview;
      changed = true;
      continue;
    }

    let merged = cached;
    for (const m of preview) {
      const lastCached = lastRealMessage(merged);
      const confirmsOptimistic = optimisticIndexFor(merged, m) >= 0;
      const isNewer = (m.server_seq ?? 0) > (lastCached?.server_seq ?? 0);
      if (!confirmsOptimistic && !isNewer) continue;
      merged = upsertConfirmedMessage(merged, m);
    }
    if (merged !== cached) {
      next[cid] = merged;
      changed = true;
    }
  }

  return changed ? next : existing;
}

/**
 * Merge an incremental `since_seq` response into the cache for a single
 * conversation. The incoming `newMsgs` array is expected to be chronological
 * (oldest-first) and contain only messages with `server_seq > last-cached`.
 *
 * Returns the updated cache (same reference if nothing changed).
 */
export function appendIncrementalMessages(
  existing: MessagesByConv,
  convId: string,
  newMsgs: ChatMessage[],
): MessagesByConv {
  if (newMsgs.length === 0) return existing;

  const cached = existing[convId] ?? [];
  const lastSeq = lastRealServerSeq(cached);

  let merged = cached;
  for (const msg of newMsgs) {
    // An older server row is only interesting when it confirms an optimistic
    // entry that is still waiting; anything else below the cursor is stale.
    if (msg.server_seq <= lastSeq && optimisticIndexFor(merged, msg) < 0) continue;
    merged = upsertConfirmedMessage(merged, msg);
  }

  if (merged === cached) return existing;
  return { ...existing, [convId]: merged };
}

/**
 * Merge a full first-page fetch into any messages that arrived while the
 * fetch was in flight. Unlike the incremental path, the full response can
 * contain older server messages that need to be backfilled ahead of an
 * already-cached WS/poll event.
 */
export function mergeFetchedMessages(
  existing: MessagesByConv,
  convId: string,
  fetchedMsgs: ChatMessage[],
): MessagesByConv {
  const cached = existing[convId] ?? [];
  if (cached.length === 0) return { ...existing, [convId]: fetchedMsgs };
  if (fetchedMsgs.length === 0) return existing;

  let changed = false;
  const byId = new Map<string, ChatMessage>();
  const optimisticByKey = new Map<string, string>();

  for (const msg of cached) {
    byId.set(msg.id, msg);
    if (!isRealMessage(msg) && msg.idempotency_key) {
      optimisticByKey.set(msg.idempotency_key, msg.id);
    }
  }

  for (const msg of fetchedMsgs) {
    const optimisticId = msg.idempotency_key
      ? optimisticByKey.get(msg.idempotency_key)
      : undefined;
    if (optimisticId) {
      byId.delete(optimisticId);
      byId.set(msg.id, msg);
      changed = true;
      continue;
    }

    if (!byId.has(msg.id)) {
      byId.set(msg.id, msg);
      changed = true;
    }
  }

  if (!changed) return existing;

  const merged = Array.from(byId.values()).sort((a, b) => {
    if (a.server_seq !== b.server_seq) return a.server_seq - b.server_seq;
    return new Date(a.created_at).getTime() - new Date(b.created_at).getTime();
  });
  return { ...existing, [convId]: merged };
}

/**
 * Return messages from `next` that still need IndexedDB write-through.
 *
 * React state merges are not always append-only: a full fetch can backfill older
 * messages ahead of a cached preview or WebSocket tail. Comparing by id avoids
 * dropping those older messages while keeping normal append writes small.
 */
export function messagesMissingFromPrevious(
  prev: ChatMessage[] | undefined,
  next: ChatMessage[],
): ChatMessage[] {
  if (next.length === 0) return [];
  if (!prev || prev.length === 0) return next;

  const prevIds = new Set(prev.map((msg) => msg.id));
  return next.filter((msg) => !prevIds.has(msg.id));
}

/**
 * Return the maximum `server_seq` in the cache for a conversation, or 0 if
 * no messages are cached.  Used as the `since_seq` cursor for incremental
 * fetches.
 */
export function maxCachedSeq(
  cache: MessagesByConv,
  convId: string,
): number {
  const msgs = cache[convId];
  if (!msgs || msgs.length === 0) return 0;
  return lastRealServerSeq(msgs);
}
