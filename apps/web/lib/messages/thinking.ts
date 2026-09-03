/**
 * A thinking marker belongs to the principal that was mentioned, not to the
 * current client-side roster. A reply can arrive before that roster finishes
 * refreshing, especially for an imported session, so every message sender is
 * eligible to clear its own marker.
 */
export function thinkingMarkerClearIds(
  messages: Iterable<{ sender_id: string }>,
): Set<string> {
  return new Set(Array.from(messages, (message) => message.sender_id));
}
