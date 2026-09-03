// ---------------------------------------------------------------------------
// Integration test: exercises mergePreviewIntoMessages against the real
// running choruz-api-gateway on 127.0.0.1:3000. Simulates the exact sequence that
// chat-app.tsx does (fetch full history via /v1/conversations/{id}/messages,
// then apply merge from /v1/console preview) and asserts that the cache is
// not truncated.
//
// This test is the end-to-end regression guard for the "messages disappear
// after 30s" bug. It is SKIPPED if no local choruz-api-gateway is listening.
// ---------------------------------------------------------------------------

import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { mergePreviewIntoMessages, type MessagesByConv } from "./messages";
import type { ChatMessage, Conversation, ConsoleSnapshot } from "../api/choruz-types";

const API = "http://127.0.0.1:3000";

type LoginResp = { principal: { id: string }; session_token: string };

async function tryFetch<T>(path: string, init: RequestInit = {}): Promise<T | null> {
  try {
    const res = await fetch(`${API}${path}`, init);
    if (!res.ok) return null;
    return (await res.json()) as T;
  } catch {
    return null;
  }
}

let token = "";
let principalId = "";
let available = false;

beforeAll(async () => {
  const login = await tryFetch<LoginResp>("/v1/auth/local/login", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ username: "operator", password: "choruz-local" }),
  });
  if (!login) return;
  token = login.session_token;
  principalId = login.principal.id;
  available = true;
});

describe.skipIf(!process.env.CHORUZ_INTEGRATION)("merge against live choruz-api-gateway", () => {
  it("does not truncate a real group's full history when the /v1/console preview is merged in", async () => {
    if (!available) {
      // Not skipping the whole describe because CHORUZ_INTEGRATION was set
      // intentionally, but the backend may still be down — fail loudly.
      throw new Error("choruz-api-gateway not reachable at 127.0.0.1:3000");
    }

    // ── Step 1: get the snapshot (sidebar preview) ────────────────────────
    const snap = await tryFetch<ConsoleSnapshot>("/v1/console", {
      headers: { authorization: `Bearer ${token}` },
    });
    expect(snap).not.toBeNull();
    const previewMap = snap!.messages_by_conversation;

    // ── Step 2: pick the first group with >= 2 real messages ─────────────
    const groups = snap!.conversations.filter(
      (c: Conversation) => c.conversation_type === "group",
    );
    let targetConvId = "";
    let fullHistory: ChatMessage[] = [];
    for (const g of groups) {
      const msgs = await tryFetch<ChatMessage[]>(
        `/v1/conversations/${g.id}/messages?principal_id=${encodeURIComponent(principalId)}`,
        { headers: { authorization: `Bearer ${token}` } },
      );
      if (msgs && msgs.length >= 2) {
        targetConvId = g.id;
        fullHistory = msgs;
        break;
      }
    }

    if (!targetConvId) {
      throw new Error(
        "no group with >= 2 messages available for regression test",
      );
    }

    // Sanity: the preview for this conv should have 0 or 1 msgs (confirming
    // the server-side truncation we are compensating for).
    const preview = previewMap[targetConvId];
    expect(preview.length).toBeLessThanOrEqual(1);
    expect(fullHistory.length).toBeGreaterThanOrEqual(2);

    // ── Step 3: simulate chat-app.tsx flow ───────────────────────────────
    // (a) selectConversation: messagesByConv[target] = full history
    let cache: MessagesByConv = { [targetConvId]: fullHistory };
    const lenBeforeMerge = cache[targetConvId].length;

    // (b) refreshSnapshot fires ~30s later: apply merge
    cache = mergePreviewIntoMessages(cache, previewMap);
    const lenAfterMerge = cache[targetConvId].length;

    // ── Assert: history is NOT truncated ─────────────────────────────────
    expect(lenAfterMerge).toBe(lenBeforeMerge);
    expect(cache[targetConvId][0].id).toBe(fullHistory[0].id);
    expect(cache[targetConvId][lenAfterMerge - 1].id).toBe(
      fullHistory[lenBeforeMerge - 1].id,
    );

    // ── Repeat to simulate multiple refresh cycles ───────────────────────
    for (let i = 0; i < 5; i++) {
      cache = mergePreviewIntoMessages(cache, previewMap);
    }
    expect(cache[targetConvId].length).toBe(lenBeforeMerge);
  });
});
