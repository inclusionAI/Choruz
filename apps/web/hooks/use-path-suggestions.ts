"use client";

import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from "react";
import type { DirEntry } from "../lib/api/choruz-types";
import { transportFetch } from "../lib/api/transport";

const DEBOUNCE_MS = 150;

export type DirectoryListing = {
  /** Canonical path of the listed directory, when the server reports it. */
  path?: string;
  parent?: string;
  entries: DirEntry[];
};

/** Lists a directory's subdirectories. Rejects on HTTP errors and on abort. */
export async function listDirectory(path: string, signal?: AbortSignal): Promise<DirectoryListing> {
  const res = await fetch(
    `/api/filesystem?action=list&path=${encodeURIComponent(path)}&include_files=false`,
    { signal },
  );
  if (!res.ok) throw new Error(`Cannot read ${path}`);
  const data = (await res.json()) as { path?: string; parent?: string; entries?: DirEntry[] };
  return { path: data.path, parent: data.parent, entries: data.entries ?? [] };
}

/** The user's home directory, or null when the backend cannot say. */
export async function fetchHomeDirectory(signal?: AbortSignal): Promise<string | null> {
  const res = await transportFetch("/api/filesystem?action=home", { signal });
  const data = (await res.json()) as { home?: string };
  return data.home ?? null;
}

/**
 * Directory autocomplete for a path input. The typed value is split at its
 * last "/" into a parent directory to list and a prefix to filter by; the
 * hook owns the request lifecycle and the open / highlighted state of the
 * suggestion list. Callers own the input, the list markup, and what Tab
 * and Enter do with `highlighted`.
 */
export function usePathSuggestions({ includeParent = false }: { includeParent?: boolean } = {}) {
  const [suggestions, setSuggestions] = useState<DirEntry[]>([]);
  const [open, setOpen] = useState(false);
  const [index, setIndex] = useState(0);
  const [loading, setLoading] = useState(false);
  const abortRef = useRef<AbortController | undefined>(undefined);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  const fetchSuggestions = useCallback(
    async (inputPath: string) => {
      // A direct listing supersedes both the request and the keystroke
      // debounce that were pending, so neither can land on top of it.
      clearTimeout(debounceRef.current);
      abortRef.current?.abort();
      const controller = new AbortController();
      abortRef.current = controller;

      const lastSlash = inputPath.lastIndexOf("/");
      const parentDir = lastSlash >= 0 ? inputPath.slice(0, lastSlash + 1) : inputPath;
      const prefix = lastSlash >= 0 ? inputPath.slice(lastSlash + 1).toLowerCase() : "";

      setLoading(true);
      try {
        const listing = await listDirectory(parentDir, controller.signal);
        let entries = listing.entries.filter((e) => e.type === "directory");
        if (prefix) entries = entries.filter((e) => e.name.toLowerCase().startsWith(prefix));
        if (includeParent && listing.parent) {
          entries.unshift({ name: "..", type: "directory", path: listing.parent });
        }
        setSuggestions(entries);
        setIndex(0);
        setOpen(entries.length > 0);
      } catch (e) {
        if ((e as Error).name !== "AbortError") {
          setSuggestions([]);
          setOpen(false);
        }
      } finally {
        // A superseded request must not clear the spinner of the one that replaced it.
        if (!controller.signal.aborted) setLoading(false);
      }
    },
    [includeParent],
  );

  /**
   * Debounced fetch for typing; a trailing "/" lists immediately. The
   * request for the previous value is dropped at once so it cannot open
   * stale suggestions during the debounce window.
   */
  const scheduleFetch = useCallback(
    (value: string) => {
      clearTimeout(debounceRef.current);
      abortRef.current?.abort();
      if (value.endsWith("/")) void fetchSuggestions(value);
      else debounceRef.current = setTimeout(() => void fetchSuggestions(value), DEBOUNCE_MS);
    },
    [fetchSuggestions],
  );

  /** Closes the list and drops any pending listing so it cannot reopen it. */
  const close = useCallback(() => {
    abortRef.current?.abort();
    clearTimeout(debounceRef.current);
    setLoading(false);
    setOpen(false);
  }, []);

  useEffect(
    () => () => {
      abortRef.current?.abort();
      clearTimeout(debounceRef.current);
    },
    [],
  );

  /** ArrowUp / ArrowDown move the highlight, Escape closes. True when consumed. */
  const handleNavigationKey = useCallback(
    (e: KeyboardEvent): boolean => {
      if (!open || suggestions.length === 0) return false;
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setIndex((i) => Math.min(i + 1, suggestions.length - 1));
        return true;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setIndex((i) => Math.max(i - 1, 0));
        return true;
      }
      if (e.key === "Escape") {
        close();
        return true;
      }
      return false;
    },
    [open, suggestions.length, close],
  );

  return {
    suggestions,
    open,
    index,
    loading,
    highlighted: open ? suggestions[index] : undefined,
    fetchSuggestions,
    scheduleFetch,
    close,
    handleNavigationKey,
  };
}
