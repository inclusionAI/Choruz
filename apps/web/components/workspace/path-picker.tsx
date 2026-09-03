"use client";

import { useEffect, useRef, useCallback } from "react";
import { Spinner } from "../ui/spinner";
import type { DirEntry } from "../../lib/api/choruz-types";
import { fetchHomeDirectory, usePathSuggestions } from "../../hooks/use-path-suggestions";

interface PathPickerProps {
  value: string;
  onChange: (path: string) => void;
  placeholder?: string;
  /** When true (default), auto-fill with the user's HOME directory on mount. */
  autoHome?: boolean;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function PathPicker({ value, onChange, placeholder, autoHome = true }: PathPickerProps) {
  const {
    suggestions,
    open,
    index,
    loading,
    highlighted,
    fetchSuggestions,
    scheduleFetch,
    close,
    handleNavigationKey,
  } = usePathSuggestions({ includeParent: true });
  const inputRef = useRef<HTMLInputElement>(null);
  const dropdownRef = useRef<HTMLUListElement>(null);
  // One home lookup at a time; dropped on unmount so a late answer cannot
  // write into a parent that has moved on.
  const homeAbortRef = useRef<AbortController | undefined>(undefined);

  const resolveHome = useCallback((onHome: (home: string) => void) => {
    homeAbortRef.current?.abort();
    const controller = new AbortController();
    homeAbortRef.current = controller;
    fetchHomeDirectory(controller.signal)
      .then((home) => {
        if (home && !controller.signal.aborted) onHome(home);
      })
      .catch(() => {/* ignore – backend may not be ready */});
  }, []);

  useEffect(() => () => homeAbortRef.current?.abort(), []);

  // Initialise with the user's home directory when value is empty
  useEffect(() => {
    if (autoHome && !value) resolveHome(onChange);
    // Only run once on mount
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** Step into a directory: fill it in and list its children. */
  const enter = useCallback(
    (entry: DirEntry) => {
      const next = entry.path + "/";
      onChange(next);
      void fetchSuggestions(next);
    },
    [onChange, fetchSuggestions],
  );

  /** Dismiss the list and drop a HOME lookup that could reopen it. */
  const dismiss = useCallback(() => {
    homeAbortRef.current?.abort();
    close();
  }, [close]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Escape") homeAbortRef.current?.abort();
      if (!open) {
        // Open on arrow-down even when closed
        if (e.key === "ArrowDown" && value) {
          e.preventDefault();
          void fetchSuggestions(value);
        }
        return;
      }
      if (handleNavigationKey(e)) return;
      if ((e.key === "Tab" || e.key === "Enter") && highlighted) {
        e.preventDefault();
        enter(highlighted);
      }
    },
    [open, value, fetchSuggestions, handleNavigationKey, highlighted, enter],
  );

  const handleFocus = () => {
    if (value) {
      void fetchSuggestions(value);
      return;
    }
    // Empty input: start browsing from HOME. Only fill it in when the
    // caller asked for that; otherwise the value stays whatever the user picks.
    resolveHome((home) => {
      const homePath = home + "/";
      if (autoHome) onChange(homePath);
      void fetchSuggestions(homePath);
    });
  };

  // Close dropdown when clicking outside
  useEffect(() => {
    const onPointerDown = (e: PointerEvent) => {
      if (
        inputRef.current &&
        !inputRef.current.contains(e.target as Node) &&
        // The list may not be mounted yet (HOME lookup pending); a click
        // elsewhere must still drop that lookup so it cannot open it later.
        !dropdownRef.current?.contains(e.target as Node)
      ) {
        dismiss();
      }
    };
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [dismiss]);

  // Keep selected item scrolled into view
  useEffect(() => {
    const item = dropdownRef.current?.children[index] as HTMLElement | undefined;
    item?.scrollIntoView({ block: "nearest" });
  }, [index]);

  return (
    <div style={{ position: "relative" }}>
      <div style={{ position: "relative" }}>
        <input
          ref={inputRef}
          type="text"
          value={value}
          onChange={(e) => {
            // Typing wins over a HOME lookup that is still in flight.
            homeAbortRef.current?.abort();
            onChange(e.target.value);
            scheduleFetch(e.target.value);
          }}
          onFocus={handleFocus}
          onKeyDown={handleKeyDown}
          placeholder={placeholder || "/path/to/workspace"}
          spellCheck={false}
          autoComplete="off"
          className="path-picker-input"
        />
        {loading && (
          <span className="path-picker-loading" role="status">
            <Spinner size={12} />
            <span className="sr-only">Loading directory…</span>
          </span>
        )}
      </div>

      {open && suggestions.length > 0 && (
        <ul ref={dropdownRef} className="path-picker-dropdown">
          {suggestions.map((entry, i) => (
            <li
              key={entry.path}
              onPointerDown={(e) => {
                e.preventDefault();
                enter(entry);
                inputRef.current?.focus();
              }}
              className={`path-picker-option${i === index ? " is-selected" : ""}`}
            >
              <span className="path-picker-option-icon">
                {entry.name === ".." ? "↑" : "📁"}
              </span>
              <span>{entry.name}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
