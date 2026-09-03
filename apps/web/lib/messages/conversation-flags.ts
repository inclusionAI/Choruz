/**
 * Pure half of the optimistic conversation flags (pin / archive / hide):
 * how a locally applied change is layered over what the server reports.
 * `use-optimistic-conversation-flag.ts` owns the React state around it.
 */

export type FlagEntry = { conversation_id: string };

/**
 * A change the user made locally. `entry` is null when the flag was
 * cleared. `settledAt` is set once the server acknowledged it, after which
 * the override only applies to snapshots that started before that moment.
 */
export type FlagOverride<T extends FlagEntry> = {
  entry: T | null;
  settledAt: number | null;
};

/**
 * The rule the backend enforces when it writes the flags: a hidden
 * conversation is neither pinned nor archived, and an archived one is not
 * pinned. Lists that already satisfy it are returned as-is.
 */
export function reconcileFlags<P extends FlagEntry, A extends FlagEntry>(
  pinned: P[],
  archived: A[],
  hidden: FlagEntry[],
): { pinned: P[]; archived: A[] } {
  const hiddenIds = new Set(hidden.map((entry) => entry.conversation_id));
  const nextArchived = archived.filter((entry) => !hiddenIds.has(entry.conversation_id));
  const excludedIds = new Set([...hiddenIds, ...nextArchived.map((entry) => entry.conversation_id)]);
  const nextPinned = pinned.filter((entry) => !excludedIds.has(entry.conversation_id));
  return {
    pinned: nextPinned.length === pinned.length ? pinned : nextPinned,
    archived: nextArchived.length === archived.length ? archived : nextArchived,
  };
}

/** Everything except the entry for `conversationId`. */
export function withoutConversation<T extends FlagEntry>(entries: T[], conversationId: string): T[] {
  return entries.filter((entry) => entry.conversation_id !== conversationId);
}

/**
 * Layers in-flight overrides over a server list. Overrides that settled
 * before the snapshot started are already reflected in it and are dropped
 * from `overrides` as a side effect; the rest replace the server's entry
 * for their conversation. The result is newest-first by `sortKey`.
 */
export function applyPendingOverrides<T extends FlagEntry>(
  overrides: Map<string, FlagOverride<T>>,
  server: T[],
  snapshotStartedAt: number,
  sortKey: (entry: T) => string,
): T[] {
  const active = [...overrides.entries()].filter(([conversationId, override]) => {
    const shouldApply = override.settledAt === null || snapshotStartedAt <= override.settledAt;
    if (!shouldApply) overrides.delete(conversationId);
    return shouldApply;
  });
  if (active.length === 0) return server;
  const overriddenIds = new Set(active.map(([conversationId]) => conversationId));
  const base = server.filter((entry) => !overriddenIds.has(entry.conversation_id));
  const optimistic = active
    .map(([, override]) => override.entry)
    .filter((entry): entry is T => entry !== null);
  return [...optimistic, ...base].sort(
    (left, right) => Date.parse(sortKey(right)) - Date.parse(sortKey(left)),
  );
}
