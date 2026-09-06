"use client";

import { useCallback, useEffect, useState } from "react";
import { apiFetch } from "../lib/api/choruz-api";
import type { SearchResultItem } from "../lib/api/choruz-types";

const SEARCH_DEBOUNCE_MS = 300;
const SEARCH_PAGE_SIZE = 30;

/**
 * Debounced message search for the detail panel's Search tab. The query
 * is scoped to the active conversation when there is one; without it the
 * gateway searches every conversation the user belongs to.
 * Continuation preserves loaded results on failure; changing query or identity
 * invalidates every prior page completion through the same effect cleanup.
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
  onSelectResult: (conversationId: string, messageId: string) => void;
}) {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResultItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [hasMore, setHasMore] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [request, setRequest] = useState<{ cursor: SearchResultItem | null }>({ cursor: null });
  const handleInput = useCallback((value: string) => {
    setQuery(value);
    setRequest({ cursor: null });
  }, []);
  useEffect(() => { handleInput(""); }, [activeConversationId, principalId, sessionToken, handleInput]);

  useEffect(() => {
    let cancelled = false;
    if (!request.cursor) {
      setResults([]);
      setHasMore(false);
    }
    setError(null);
    setLoading(Boolean(query.trim()));
    if (!query.trim()) return;
    const timer = setTimeout(async () => {
      try {
        const params = new URLSearchParams({ principal_id: principalId, q: query.trim(), limit: String(SEARCH_PAGE_SIZE + 1) });
        if (activeConversationId) params.set("conversation_id", activeConversationId);
        if (request.cursor) {
          params.set("before_created_at", request.cursor.created_at);
          params.set("before_message_id", request.cursor.message_id);
        }
        const found = await apiFetch<SearchResultItem[]>(`/v1/messages/search?${params.toString()}`, sessionToken);
        if (!cancelled) {
          const page = found.slice(0, SEARCH_PAGE_SIZE);
          setResults((previous) => request.cursor ? [...previous, ...page] : page);
          setHasMore(found.length > SEARCH_PAGE_SIZE);
        }
      } catch {
        if (!cancelled) setError("Could not load search results.");
      } finally {
        if (!cancelled) setLoading(false);
      }
    }, request.cursor ? 0 : SEARCH_DEBOUNCE_MS);
    return () => { cancelled = true; clearTimeout(timer); };
  }, [query, activeConversationId, principalId, sessionToken, request]);

  const handleResultClick = useCallback(
    (conversationId: string, messageId: string) => {
      onSelectResult(conversationId, messageId);
      handleInput("");
      setResults([]);
    },
    [onSelectResult, handleInput],
  );

  const loadMore = () => {
    if (!loading && hasMore) setRequest({ cursor: results.at(-1) ?? null });
  };
  const retry = () => { if (!loading) setRequest({ ...request }); };

  return { query, results, loading, hasMore, error, loadMore, retry, handleInput, handleResultClick };
}
