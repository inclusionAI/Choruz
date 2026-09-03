"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { apiFetch } from "../lib/api/choruz-api";
import type { SearchResultItem } from "../lib/api/choruz-types";

const SEARCH_DEBOUNCE_MS = 300;

/**
 * Debounced message search for the detail panel's Search tab. The query
 * is scoped to the active conversation when there is one; without it the
 * gateway searches every conversation the user belongs to.
 */
export function useMessageSearch({
  principalId,
  sessionToken,
  activeConversationId,
  onSelectResult,
}: {
  principalId: string;
  sessionToken: string;
  activeConversationId: string | null;
  onSelectResult: (conversationId: string) => void;
}) {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResultItem[]>([]);
  const [loading, setLoading] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(
    () => () => {
      if (timerRef.current) clearTimeout(timerRef.current);
    },
    [],
  );

  const handleInput = useCallback(
    (value: string) => {
      setQuery(value);
      if (timerRef.current) clearTimeout(timerRef.current);
      if (!value.trim()) {
        setResults([]);
        setLoading(false);
        return;
      }
      setLoading(true);
      // Scoped to the conversation active at the keystroke, not when the
      // debounce fires.
      const conversationId = activeConversationId;
      timerRef.current = setTimeout(async () => {
        try {
          const params = new URLSearchParams({ principal_id: principalId, q: value.trim(), limit: "30" });
          if (conversationId) params.set("conversation_id", conversationId);
          setResults(await apiFetch<SearchResultItem[]>(`/v1/messages/search?${params.toString()}`, sessionToken));
        } catch {
          setResults([]);
        } finally {
          setLoading(false);
        }
      }, SEARCH_DEBOUNCE_MS);
    },
    [activeConversationId, principalId, sessionToken],
  );

  const handleResultClick = useCallback(
    (conversationId: string) => {
      onSelectResult(conversationId);
      setQuery("");
      setResults([]);
    },
    [onSelectResult],
  );

  return { query, results, loading, handleInput, handleResultClick };
}
