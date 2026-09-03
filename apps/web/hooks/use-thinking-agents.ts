"use client";

import { useCallback, useEffect, useRef, useState } from "react";

/**
 * Which agents show a "thinking…" bubble. Each mark auto-clears after
 * `ttlMs` if no reply arrives; a reply clears it early. The TTL is UX
 * only and does not bound the agent's real work.
 */
export function useThinkingAgents(ttlMs: number) {
  const [thinkingAgents, setThinkingAgents] = useState<Set<string>>(new Set());
  const timeoutsRef = useRef<Map<string, ReturnType<typeof setTimeout>>>(new Map());

  const clearThinking = useCallback((agentIds: Iterable<string>) => {
    const ids = Array.from(agentIds);
    if (ids.length === 0) return;
    for (const id of ids) {
      const timeout = timeoutsRef.current.get(id);
      if (timeout) {
        clearTimeout(timeout);
        timeoutsRef.current.delete(id);
      }
    }
    setThinkingAgents((prev) => {
      let changed = false;
      const next = new Set(prev);
      for (const id of ids) {
        if (next.delete(id)) changed = true;
      }
      return changed ? next : prev;
    });
  }, []);

  const markThinking = useCallback(
    (agentIds: Iterable<string>) => {
      const ids = Array.from(new Set(agentIds));
      if (ids.length === 0) return;
      setThinkingAgents((prev) => {
        const next = new Set(prev);
        for (const id of ids) next.add(id);
        return next;
      });
      for (const id of ids) {
        const existing = timeoutsRef.current.get(id);
        if (existing) clearTimeout(existing);
        const timeout = setTimeout(() => {
          timeoutsRef.current.delete(id);
          setThinkingAgents((prev) => {
            if (!prev.has(id)) return prev;
            const next = new Set(prev);
            next.delete(id);
            return next;
          });
        }, ttlMs);
        timeoutsRef.current.set(id, timeout);
      }
    },
    [ttlMs],
  );

  useEffect(
    () => () => {
      for (const timeout of timeoutsRef.current.values()) clearTimeout(timeout);
      timeoutsRef.current.clear();
    },
    [],
  );

  return { thinkingAgents, markThinking, clearThinking };
}
