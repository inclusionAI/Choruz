"use client";

import { useCallback } from "react";
import type {
  ArchivedConversation,
  DashboardBootstrap,
  DashboardSyncChange,
  HiddenConversation,
  PinnedConversation,
} from "../lib/api/choruz-types";
import { reconcileFlags, withoutConversation, type FlagEntry } from "../lib/messages/conversation-flags";
import { useOptimisticConversationFlag } from "./use-optimistic-conversation-flag";

// The three flags share one shape; only the timestamp field, endpoint and
// telemetry differ. These stay module-level on purpose: every callback the
// hook returns lists them as dependencies, so an inline object would make
// them all churn per render.
const PIN_FLAG = {
  sortKey: (pin: PinnedConversation) => pin.pinned_at,
  makeEntry: (conversation_id: string, previous: PinnedConversation | null): PinnedConversation => ({
    conversation_id,
    pinned_at: previous?.pinned_at ?? new Date().toISOString(),
  }),
  endpoint: (conversationId: string) => `/v1/conversations/${conversationId}/pin`,
  telemetry: (pinned: boolean) => ({
    event: "conversation_pin_toggle",
    errorEvent: "conversation_pin_toggle_error",
    data: { pinned },
  }),
};

const ARCHIVE_FLAG = {
  sortKey: (archive: ArchivedConversation) => archive.archived_at,
  makeEntry: (conversation_id: string, previous: ArchivedConversation | null): ArchivedConversation => ({
    conversation_id,
    archived_at: previous?.archived_at ?? new Date().toISOString(),
  }),
  endpoint: (conversationId: string) => `/v1/conversations/${conversationId}/archive`,
  telemetry: (archived: boolean) => ({
    event: "conversation_archive_toggle",
    errorEvent: "conversation_archive_toggle_error",
    data: { archived },
  }),
};

const HIDDEN_FLAG = {
  sortKey: (hidden: HiddenConversation) => hidden.hidden_at,
  makeEntry: (conversation_id: string, previous: HiddenConversation | null): HiddenConversation => ({
    conversation_id,
    hidden_at: previous?.hidden_at ?? new Date().toISOString(),
  }),
  endpoint: (conversationId: string) => `/v1/conversations/${conversationId}/hide`,
  telemetry: (hidden: boolean) =>
    hidden
      ? { event: "conversation_hidden", errorEvent: "conversation_hide_error" }
      : { event: "conversation_restored", errorEvent: "conversation_restore_error" },
};

type Options = {
  initial: {
    pinned: PinnedConversation[];
    archived: ArchivedConversation[];
    hidden: HiddenConversation[];
  };
  conversations: ReadonlyArray<{ id: string }>;
  sessionToken: string;
  trackEvent: (event: string, data?: Record<string, unknown>) => void;
};

/**
 * Pinned, archived and hidden conversations as one unit. Each list is an
 * optimistic flag of its own; this hook owns the rule that ties them
 * together (archiving unpins, hiding unpins and unarchives, as the backend
 * does) and applies it after every local change, sync event and bootstrap.
 */
export function useConversationFlags({ initial, conversations, sessionToken, trackEvent }: Options) {
  const pins = useOptimisticConversationFlag<PinnedConversation>({
    ...PIN_FLAG, initial: initial.pinned, conversations, sessionToken, trackEvent,
  });
  const archives = useOptimisticConversationFlag<ArchivedConversation>({
    ...ARCHIVE_FLAG, initial: initial.archived, conversations, sessionToken, trackEvent,
  });
  const hidden = useOptimisticConversationFlag<HiddenConversation>({
    ...HIDDEN_FLAG, initial: initial.hidden, conversations, sessionToken, trackEvent,
  });

  const reconcile = useCallback(() => {
    const next = reconcileFlags(pins.entriesRef.current, archives.entriesRef.current, hidden.entriesRef.current);
    if (next.pinned !== pins.entriesRef.current) pins.replace(next.pinned);
    if (next.archived !== archives.entriesRef.current) archives.replace(next.archived);
  }, [archives.entriesRef, archives.replace, hidden.entriesRef, pins.entriesRef, pins.replace]);

  const togglePin = useCallback(
    (conversationId: string, nextPinned: boolean) => pins.setFlag(conversationId, nextPinned),
    [pins.setFlag],
  );

  // A failed archive puts the pin back.
  const toggleArchive = useCallback(
    async (conversationId: string, nextArchived: boolean) => {
      const previousPin = pins.find(conversationId);
      await archives.setFlag(conversationId, nextArchived, {
        onOptimistic: reconcile,
        onRollback: () => {
          if (previousPin) pins.replace([previousPin, ...withoutConversation(pins.entriesRef.current, conversationId)]);
        },
      });
    },
    [archives.setFlag, pins.entriesRef, pins.find, pins.replace, reconcile],
  );

  // `onOptimistic` runs alongside the local change, so the caller can close
  // the conversation in the same render. A failed hide restores the entries.
  const hide = useCallback(
    async (conversationId: string, onOptimistic?: () => void) => {
      const previousPin = pins.find(conversationId);
      const previousArchive = archives.find(conversationId);
      await hidden.setFlag(conversationId, true, {
        onOptimistic: () => {
          reconcile();
          onOptimistic?.();
        },
        onRollback: () => {
          if (previousPin) pins.replace([previousPin, ...withoutConversation(pins.entriesRef.current, conversationId)]);
          if (previousArchive) {
            archives.replace([previousArchive, ...withoutConversation(archives.entriesRef.current, conversationId)]);
          }
        },
      });
    },
    [archives.entriesRef, archives.find, archives.replace, hidden.setFlag, pins.entriesRef, pins.find, pins.replace, reconcile],
  );

  const restore = useCallback(
    (conversationId: string) => hidden.setFlag(conversationId, false),
    [hidden.setFlag],
  );

  /** Absorbs a flag event from the sync stream; false when it is not one. */
  const applyChange = useCallback(
    (change: DashboardSyncChange): boolean => {
      const conversationId = change.conversation_id;
      if (!conversationId) return false;
      switch (change.event_type) {
        case "conversation.pin_set":
          pins.syncEntry(conversationId, { conversation_id: conversationId, pinned_at: change.created_at });
          return true;
        case "conversation.pin_removed":
          pins.syncEntry(conversationId, null);
          return true;
        case "conversation.archive_set":
          archives.syncEntry(conversationId, { conversation_id: conversationId, archived_at: change.created_at });
          reconcile();
          return true;
        case "conversation.archive_removed":
          archives.syncEntry(conversationId, null);
          return true;
        case "conversation.hidden_set":
          hidden.syncEntry(conversationId, { conversation_id: conversationId, hidden_at: change.created_at });
          reconcile();
          return true;
        case "conversation.hidden_removed":
          hidden.syncEntry(conversationId, null);
          return true;
        default:
          return false;
      }
    },
    [archives.syncEntry, hidden.syncEntry, pins.syncEntry, reconcile],
  );

  /**
   * Takes the flags from a bootstrap page. A `replace` snapshot is the
   * whole truth; other modes keep entries for conversations outside the
   * page. Hidden conversations always arrive as one complete list.
   */
  const applyBootstrap = useCallback(
    (bootstrap: DashboardBootstrap, mode: "replace" | "append" | "refresh", snapshotStartedAt: number) => {
      const items = bootstrap.conversations.items;
      const pageIds = new Set(items.map((item) => item.conversation.id));
      const outsidePage = <T extends FlagEntry>(entries: T[]) =>
        mode === "replace" ? [] : entries.filter((entry) => !pageIds.has(entry.conversation_id));
      const pageArchives = items
        .filter((item) => item.archived_at)
        .map((item) => ({ conversation_id: item.conversation.id, archived_at: item.archived_at! }));
      const pagePins = items
        .filter((item) => item.pinned_at)
        .map((item) => ({ conversation_id: item.conversation.id, pinned_at: item.pinned_at! }));
      archives.replace(archives.applyOverrides([...pageArchives, ...outsidePage(archives.entriesRef.current)], snapshotStartedAt));
      pins.replace(pins.applyOverrides([...pagePins, ...outsidePage(pins.entriesRef.current)], snapshotStartedAt));
      hidden.replace(hidden.applyOverrides(bootstrap.hidden_conversations, snapshotStartedAt));
      reconcile();
    },
    [
      archives.applyOverrides, archives.entriesRef, archives.replace,
      hidden.applyOverrides, hidden.replace,
      pins.applyOverrides, pins.entriesRef, pins.replace,
      reconcile,
    ],
  );

  return {
    pinned: pins.entries,
    archived: archives.entries,
    hidden: hidden.entries,
    pendingPinIds: pins.pendingIds,
    pendingArchiveIds: archives.pendingIds,
    pendingHiddenIds: hidden.pendingIds,
    togglePin,
    toggleArchive,
    hide,
    restore,
    applyChange,
    applyBootstrap,
  };
}
