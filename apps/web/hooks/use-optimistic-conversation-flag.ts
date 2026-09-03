"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { apiFetch } from "../lib/api/choruz-api";
import { trace } from "../lib/api/choruz-trace";
import {
  applyPendingOverrides,
  withoutConversation,
  type FlagEntry,
  type FlagOverride,
} from "../lib/messages/conversation-flags";

type Telemetry = { event: string; errorEvent: string; data?: Record<string, unknown> };

type Options<T extends FlagEntry> = {
  initial: T[];
  /** Conversations still known to the client; entries for others are pruned. */
  conversations: ReadonlyArray<{ id: string }>;
  sessionToken: string;
  /** Newest-first ordering key, e.g. `pinned_at`. */
  sortKey: (entry: T) => string;
  /** The entry to show while the server call is in flight. */
  makeEntry: (conversationId: string, previous: T | null) => T;
  /** `PUT` sets the flag, `DELETE` clears it. */
  endpoint: (conversationId: string) => string;
  telemetry: (next: boolean) => Telemetry;
  trackEvent: (event: string, data?: Record<string, unknown>) => void;
};

type SetFlagHooks = {
  /** Runs right after the optimistic list is applied. */
  onOptimistic?: () => void;
  /** Runs after the list is rolled back because the request failed. */
  onRollback?: () => void;
};

/**
 * One optimistic conversation flag (pin, archive, hide); the engine behind
 * `useConversationFlags`, which owns how the three relate. The list is
 * mirrored into a ref so websocket and bootstrap handlers can read the
 * latest value without re-subscribing; `applyPendingOverrides` lets them
 * layer in-flight changes over what the server reports.
 */
export function useOptimisticConversationFlag<T extends FlagEntry>({
  initial,
  conversations,
  sessionToken,
  sortKey,
  makeEntry,
  endpoint,
  telemetry,
  trackEvent,
}: Options<T>) {
  const [entries, setEntries] = useState<T[]>(initial);
  const entriesRef = useRef<T[]>(initial);
  const overridesRef = useRef<Map<string, FlagOverride<T>>>(new Map());
  const [pendingIds, setPendingIds] = useState<Set<string>>(new Set());
  const pendingIdsRef = useRef<Set<string>>(new Set());

  const replace = useCallback((next: T[]) => {
    entriesRef.current = next;
    setEntries(next);
  }, []);

  const replacePending = useCallback((next: Set<string>) => {
    pendingIdsRef.current = next;
    setPendingIds(next);
  }, []);

  const applyOverrides = useCallback(
    (server: T[], snapshotStartedAt: number) =>
      applyPendingOverrides(overridesRef.current, server, snapshotStartedAt, sortKey),
    [sortKey],
  );

  const find = useCallback(
    (conversationId: string): T | null =>
      entriesRef.current.find((entry) => entry.conversation_id === conversationId) ?? null,
    [],
  );

  /** Absorbs a server-side change for one conversation (`null` clears it). */
  const syncEntry = useCallback(
    (conversationId: string, entry: T | null) => {
      const rest = withoutConversation(entriesRef.current, conversationId);
      replace(applyOverrides(entry ? [entry, ...rest] : rest, Date.now()));
    },
    [applyOverrides, replace],
  );

  // Drop entries for conversations that are no longer known.
  useEffect(() => {
    const known = new Set(conversations.map((conversation) => conversation.id));
    const next = entriesRef.current.filter((entry) => known.has(entry.conversation_id));
    if (next.length !== entriesRef.current.length) replace(next);
  }, [conversations, replace]);

  const setFlag = useCallback(
    async (conversationId: string, next: boolean, hooks: SetFlagHooks = {}) => {
      if (pendingIdsRef.current.has(conversationId)) return;

      const previous = find(conversationId);
      const rest = withoutConversation(entriesRef.current, conversationId);
      const optimistic = next ? makeEntry(conversationId, previous) : null;

      overridesRef.current.set(conversationId, { entry: optimistic, settledAt: null });
      replace(optimistic ? [optimistic, ...rest] : rest);
      hooks.onOptimistic?.();
      replacePending(new Set(pendingIdsRef.current).add(conversationId));

      const { event, errorEvent, data = {} } = telemetry(next);
      try {
        await apiFetch<void>(endpoint(conversationId), sessionToken, { method: next ? "PUT" : "DELETE" });
        const override = overridesRef.current.get(conversationId);
        if (override) {
          overridesRef.current.set(conversationId, { ...override, settledAt: Date.now() });
        }
        trackEvent(event, { conversation_id: conversationId, ...data });
      } catch (err) {
        trace.event(errorEvent, { conversation_id: conversationId, ...data, error: String(err) });
        overridesRef.current.delete(conversationId);
        const current = withoutConversation(entriesRef.current, conversationId);
        replace(previous ? [previous, ...current] : current);
        hooks.onRollback?.();
      } finally {
        const pending = new Set(pendingIdsRef.current);
        pending.delete(conversationId);
        replacePending(pending);
      }
    },
    [endpoint, find, makeEntry, replace, replacePending, sessionToken, telemetry, trackEvent],
  );

  return {
    entries,
    entriesRef,
    pendingIds,
    replace,
    applyOverrides,
    syncEntry,
    find,
    setFlag,
  };
}
