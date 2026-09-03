// ---------------------------------------------------------------------------
// Unread bookkeeping for message threads.
//
// Pure logic extracted from chat-app.tsx so the unread seams are unit-
// testable: (1) parsing /v1/unreads rows, (2) clearing a conversation's
// badge WITHOUT touching its thread counter (threads clear per thread via
// POST /threads/{root}/view, never by opening the conversation), and
// (3) deciding what a WebSocket message does to the local counters.
// ---------------------------------------------------------------------------

import type { ChatMessage } from "../api/choruz-types";
import { isBroadcastThreadReply, isThreadReply } from "./threads";

export type UnreadEntry = {
  unread: number;
  mentions: number;
  /** Server-canonical COUNT(DISTINCT thread) with unread replies. */
  threadUnread?: number;
};

export type UnreadRow = {
  conversation_id: string;
  unread_count: number;
  mention_count: number;
  thread_unread_count?: number;
};

/** Map /v1/unreads (or snapshot.unreads) rows into the local badge state. */
export function buildUnreadMap(rows: UnreadRow[]): Record<string, UnreadEntry> {
  const map: Record<string, UnreadEntry> = {};
  for (const u of rows) {
    map[u.conversation_id] = {
      unread: u.unread_count,
      mentions: u.mention_count,
      threadUnread: u.thread_unread_count ?? 0,
    };
  }
  return map;
}

/** Zero a conversation's unread/mention badge while PRESERVING its thread
 * counter — viewing the conversation does not read its threads. */
export function clearConversationUnread(
  map: Record<string, UnreadEntry>,
  conversationId: string,
): Record<string, UnreadEntry> {
  return {
    ...map,
    [conversationId]: { ...map[conversationId], unread: 0, mentions: 0 },
  };
}

/**
 * Conversations viewed locally whose `POST /view` may not be reflected in
 * an in-flight unreads response yet. Maps convId → the unreads request seq
 * that was CURRENT when the view happened.
 *
 * Consume rule: a response may remove (consume) an entry only when its
 * request was issued AFTER the registration (`addedAtSeq < requestSeq`).
 * Precisely: it post-dates the LOCAL registration — the POST /view itself
 * is fire-and-forget, so server-side ordering is not strictly guaranteed;
 * the rare lose-the-server-race window self-heals on the next poll. A
 * response from an OLDER request still gets the entry applied (papered
 * over) but must NOT eat the protection, and entries are never kept past
 * the first consuming response (keeping them forever would permanently
 * suppress genuine new unreads — the R5→R6 Critical regression).
 */
export type LocallyViewedRegistry = Map<string, number>;

export function registerLocallyViewed(
  registry: LocallyViewedRegistry,
  conversationId: string,
  currentRequestSeq: number,
): void {
  registry.set(conversationId, currentRequestSeq);
}

/** Apply (and selectively consume) the locally-viewed registry onto a
 * freshly fetched unread map. Mutates `registry` per the consume rule. */
export function applyLocallyViewed(
  map: Record<string, UnreadEntry>,
  registry: LocallyViewedRegistry,
  requestSeq: number,
): Record<string, UnreadEntry> {
  let next = map;
  for (const [conversationId, addedAtSeq] of registry) {
    next = clearConversationUnread(next, conversationId);
    if (addedAtSeq < requestSeq) {
      registry.delete(conversationId);
    }
  }
  return next;
}

/**
 * Claim/commit ordering gate for the unread-map writers.
 *
 * - `claim()` allocates a request seq at DISPATCH time (before the fetch).
 * - `tryCommit(seq)` admits a response only if it is newer than the last
 *   COMMITTED one — gating on the last CLAIM instead would let a burned
 *   claim (a fetch that failed, or a snapshot without an unreads field)
 *   veto a concurrent successful response, leaving the badge stale until
 *   the next poll (the Gate-8 regression this type exists to pin).
 * - `claimedSeq()` exposes the latest claim for LocallyViewedRegistry
 *   registration (an entry must only be consumed by responses whose
 *   request post-dates the registration).
 */
export type UnreadCommitGate = {
  claim(): number;
  tryCommit(requestSeq: number): boolean;
  claimedSeq(): number;
};

export function createUnreadCommitGate(): UnreadCommitGate {
  let claimed = 0;
  let committed = 0;
  return {
    claim() {
      claimed += 1;
      return claimed;
    },
    tryCommit(requestSeq: number) {
      if (requestSeq <= committed) return false;
      committed = requestSeq;
      return true;
    },
    claimedSeq() {
      return claimed;
    },
  };
}

export type WsUnreadEffect = {
  /** Bump the conversation-level unread badge by one (mirrors the server's
   * bumps_conversation_unread gate: quiet thread replies don't). */
  bumpConversationUnread: boolean;
  /** Re-fetch /v1/unreads for the thread counter. The server counts
   * DISTINCT threads with unread replies — a local per-message increment
   * would drift (3 quiet replies in one thread ≠ 3 unread threads), so the
   * thread badge is never bumped locally, only refreshed from the
   * canonical count. Broadcast replies ALSO refresh: the server's LATERAL
   * counts every threaded reply regardless of broadcast. */
  refreshThreadUnread: boolean;
};

/** What an incoming WS message FROM ANOTHER SENDER does to the local
 * unread state. Runs for every conversation, active or not: the caller
 * applies `bumpConversationUnread` only to non-active conversations (the
 * active one is being read), but `refreshThreadUnread` applies
 * unconditionally — a quiet thread reply in the ACTIVE conversation never
 * hits the main timeline, so skipping the refresh there would leave the
 * thread badge stale (the Gate-7 F3 defect; do not re-add that gate). */
export function wsUnreadEffect(msg: ChatMessage): WsUnreadEffect {
  if (isThreadReply(msg)) {
    return {
      bumpConversationUnread: isBroadcastThreadReply(msg),
      refreshThreadUnread: true,
    };
  }
  return { bumpConversationUnread: true, refreshThreadUnread: false };
}
