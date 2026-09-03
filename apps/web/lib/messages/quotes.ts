// ---------------------------------------------------------------------------
// Quote-reply preview resolution (WeChat/Feishu-style).
//
// A reply's quote block shows the ORIGINAL message. The original usually
// lives in the loaded history window; when it doesn't (old message, page
// limit), the client fetches it on demand via
// GET /v1/conversations/{id}/messages/{message_id} instead of rendering a
// "not loaded" placeholder. Fetched originals live in a dedicated
// quoted-message store — NOT in messagesByConv, where an isolated old
// message would surface mid-timeline without its surroundings.
// ---------------------------------------------------------------------------

import type { ChatMessage } from "../api/choruz-types";

/** Resolution states for an on-demand quote target. */
export type QuotedMessage = ChatMessage | "missing";

/**
 * Collect reply targets that need an on-demand fetch: referenced by a
 * loaded message, but absent from the loaded history, the quoted store,
 * and the in-flight set. Returns unique ids in first-reference order.
 */
export function collectMissingQuoteTargets(
  messages: ChatMessage[],
  quoted: ReadonlyMap<string, QuotedMessage>,
  inFlight: ReadonlySet<string>,
): string[] {
  const loaded = new Set(messages.map((m) => m.id));
  const out: string[] = [];
  const seen = new Set<string>();
  for (const msg of messages) {
    const target = msg.metadata?.reply_to_id;
    if (typeof target !== "string" || target.length === 0) continue;
    if (loaded.has(target) || quoted.has(target) || inFlight.has(target) || seen.has(target)) {
      continue;
    }
    seen.add(target);
    out.push(target);
  }
  return out;
}

/**
 * Resolve a quote target for rendering: prefer the live loaded message
 * (reflects edits, supports jump-to-message), then the on-demand store.
 * `undefined` = not resolved yet (fetch pending) — render a loading
 * placeholder; `"missing"` = the server said 404 (deleted / never
 * existed) — render an unavailable placeholder.
 */
export function resolveQuoteTarget(
  targetId: string,
  byId: ReadonlyMap<string, ChatMessage>,
  quoted: ReadonlyMap<string, QuotedMessage>,
): ChatMessage | "missing" | undefined {
  return byId.get(targetId) ?? quoted.get(targetId);
}
