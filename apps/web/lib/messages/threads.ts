// ---------------------------------------------------------------------------
// Message-thread client logic.
//
// The backend discriminates threaded replies from legacy quote-replies via
// `metadata.thread === true` (JSON boolean — string "true" is NOT a thread,
// mirroring the server's jsonb-equality predicate). `metadata.broadcast ===
// true` means "also send to channel": the reply shows in BOTH the main
// timeline and its thread. Quiet replies (no broadcast) live only in the
// thread and are rolled up as an "N replies" chip on the root message.
// ---------------------------------------------------------------------------

import type { ChatMessage } from "../api/choruz-types";
import { OPTIMISTIC_SERVER_SEQ } from "./messages";

// Re-exported for thread-side consumers (chat-app's thread wiring, tests);
// the single definition lives in lib/messages/messages.ts.
export { OPTIMISTIC_SERVER_SEQ } from "./messages";

/** Max reply_to_id hops when resolving a reply chain to its root.
 * The server canonicalizes `reply_event_id` at write time, but the FE only
 * sees `metadata.reply_to_id`, which is preserved as the ORIGINAL target —
 * so a reply-to-a-reply needs chain walking. Threads are flat server-side;
 * the cap only guards against malformed cycles in metadata. */
const MAX_ROOT_HOPS = 16;

export function isThreadReply(msg: ChatMessage): boolean {
  return (
    msg.metadata?.thread === true &&
    typeof msg.metadata?.reply_to_id === "string" &&
    (msg.metadata.reply_to_id as string).length > 0
  );
}

export function isBroadcastThreadReply(msg: ChatMessage): boolean {
  return isThreadReply(msg) && msg.metadata?.broadcast === true;
}

/**
 * Resolve the canonical thread root for a threaded reply, mirroring the
 * server's `canonicalize_thread_root_in_tx`: follow `reply_to_id` while the
 * target is itself a threaded reply; the first non-thread-reply target is
 * the root. A target not in `byId` (history not loaded) resolves to the
 * deepest id we could reach — the rollup then attaches to that message if
 * it loads later.
 */
export function resolveThreadRoot(
  msg: ChatMessage,
  byId: Map<string, ChatMessage>,
): string | null {
  if (!isThreadReply(msg)) return null;
  let rootId = msg.metadata.reply_to_id as string;
  for (let hop = 0; hop < MAX_ROOT_HOPS; hop++) {
    const target = byId.get(rootId);
    if (!target || !isThreadReply(target)) return rootId;
    rootId = target.metadata.reply_to_id as string;
  }
  return rootId;
}

export type ThreadRollup = {
  rootId: string;
  /** Thread replies in arrival order (the caller's message order). */
  replies: ChatMessage[];
  replyCount: number;
  lastReplyAt: string;
  /** Unique sender ids, first-reply order, capped for avatar display. */
  participantIds: string[];
};

export type ThreadPartition = {
  /** Main-timeline messages: everything except quiet thread replies. */
  timeline: ChatMessage[];
  /** Per-root thread replies (quiet AND broadcast — a thread shows all). */
  rollups: Map<string, ThreadRollup>;
};

const PARTICIPANT_SAMPLE_CAP = 5;

/**
 * Split a conversation's messages into the main timeline and per-thread
 * rollups. Mirrors the server's `?view=timeline` contract: quiet thread
 * replies are hidden from the timeline; broadcast replies appear in both;
 * legacy quote-replies (no `thread` flag) stay timeline-only messages.
 */
export function partitionThreadMessages(messages: ChatMessage[]): ThreadPartition {
  const byId = new Map<string, ChatMessage>();
  for (const m of messages) byId.set(m.id, m);

  const timeline: ChatMessage[] = [];
  const rollups = new Map<string, ThreadRollup>();

  for (const msg of messages) {
    if (!isThreadReply(msg)) {
      timeline.push(msg);
      continue;
    }
    if (isBroadcastThreadReply(msg)) {
      timeline.push(msg);
    }
    const rootId = resolveThreadRoot(msg, byId);
    if (!rootId || rootId === msg.id) continue;
    let rollup = rollups.get(rootId);
    if (!rollup) {
      rollup = {
        rootId,
        replies: [],
        replyCount: 0,
        lastReplyAt: msg.created_at,
        participantIds: [],
      };
      rollups.set(rootId, rollup);
    }
    rollup.replies.push(msg);
    rollup.replyCount += 1;
    if (msg.created_at >= rollup.lastReplyAt) {
      rollup.lastReplyAt = msg.created_at;
    }
    if (
      rollup.participantIds.length < PARTICIPANT_SAMPLE_CAP &&
      !rollup.participantIds.includes(msg.sender_id)
    ) {
      rollup.participantIds.push(msg.sender_id);
    }
  }

  return { timeline, rollups };
}

/** Merge thread-detail replies fetched from the API into a conversation's
 * message list without duplicating ids, preserving server_seq order.
 * Like mergeFetchedMessages, a fetched (server-confirmed) message REPLACES
 * a still-optimistic row with the same idempotency_key — a fetchThread
 * landing inside the POST→WS-confirm window must not pin a ghost
 * duplicate of the reply being sent. */
export function mergeThreadReplies(
  existing: ChatMessage[],
  fetched: ChatMessage[],
): ChatMessage[] {
  const seenIds = new Set(existing.map((m) => m.id));
  const fetchedKeys = new Set(
    fetched.map((m) => m.idempotency_key).filter((k) => k.length > 0),
  );
  const additions = fetched.filter((m) => !seenIds.has(m.id));
  const replacesOptimistic = (m: ChatMessage) =>
    m.server_seq === OPTIMISTIC_SERVER_SEQ && fetchedKeys.has(m.idempotency_key);
  if (additions.length === 0 && !existing.some(replacesOptimistic)) {
    return existing;
  }
  const kept = existing.filter((m) => !replacesOptimistic(m));
  return [...kept, ...additions].sort((a, b) => a.server_seq - b.server_seq);
}

/** Remove a FAILED optimistic send from the message list. Matches BOTH the
 * idempotency key AND the optimistic sentinel seq: if the WS confirmation
 * raced ahead of a late POST failure, the row has already been replaced by
 * the REAL committed message (same key, real seq) — deleting by key alone
 * would destroy a message the incremental backfill cursor
 * (since_seq = maxCachedSeq) can never restore. */
export function rollbackOptimisticMessage(
  messages: ChatMessage[],
  idempotencyKey: string,
): ChatMessage[] {
  return messages.filter(
    (m) =>
      !(
        m.idempotency_key === idempotencyKey &&
        m.server_seq === OPTIMISTIC_SERVER_SEQ
      ),
  );
}
