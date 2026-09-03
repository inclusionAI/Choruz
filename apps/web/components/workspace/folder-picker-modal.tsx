"use client";

import { useState, useEffect, useCallback, useRef } from "react";
import { Spinner } from "../ui/spinner";
import type { DirEntry } from "../../lib/api/choruz-types";
import { fetchHomeDirectory, listDirectory, usePathSuggestions } from "../../hooks/use-path-suggestions";
import { Modal } from "../ui/modal";

interface FolderPickerModalProps {
  initialPath?: string;
  selectionError?: string | null;
  onSelect: (path: string) => Promise<void> | void;
  /** Removes the optional workspace association without touching its files. */
  onClearFolder?: () => Promise<void> | void;
  onClose: () => void;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function FolderPickerModal({
  initialPath,
  selectionError = null,
  onSelect,
  onClearFolder,
  onClose,
}: FolderPickerModalProps) {
  const [currentPath, setCurrentPath] = useState(initialPath || "");
  const [entries, setEntries] = useState<DirEntry[]>([]);
  const [parentPath, setParentPath] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [selectionPending, setSelectionPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selectedEntry, setSelectedEntry] = useState<string | null>(null);
  const [pathInput, setPathInput] = useState("");
  const {
    suggestions: pathSuggestions,
    open: showSuggestions,
    index: suggestionIndex,
    highlighted: highlightedSuggestion,
    fetchSuggestions: fetchPathSuggestions,
    scheduleFetch: schedulePathSuggestions,
    close: closeSuggestions,
    handleNavigationKey,
  } = usePathSuggestions();
  const abortRef = useRef<AbortController | undefined>(undefined);

  const handleClose = useCallback(() => {
    if (!selectionPending) onClose();
  }, [onClose, selectionPending]);

  // Fetch home directory on mount if no initial path
  useEffect(() => {
    if (!initialPath) {
      fetchHomeDirectory()
        .then((home) => {
          if (!home) return;
          setCurrentPath(home);
          setPathInput(home);
          fetchEntries(home);
        })
        .catch(() => {});
    } else {
      setPathInput(initialPath);
      fetchEntries(initialPath);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const fetchEntries = useCallback(async (dirPath: string) => {
    abortRef.current?.abort();
    const controller = new AbortController();
    abortRef.current = controller;

    setLoading(true);
    setError(null);
    setSelectedEntry(null);

    try {
      const listing = await listDirectory(dirPath, controller.signal);
      setCurrentPath(listing.path || dirPath);
      setPathInput(listing.path || dirPath);
      setParentPath(listing.parent || null);
      setEntries(listing.entries);
    } catch (e) {
      if ((e as Error).name !== "AbortError") {
        setError("Cannot read this directory");
        setEntries([]);
      }
    } finally {
      if (!controller.signal.aborted) setLoading(false);
    }
  }, []);

  const navigateTo = useCallback(
    (path: string) => {
      if (selectionPending) return;
      fetchEntries(path);
      closeSuggestions();
    },
    [fetchEntries, selectionPending, closeSuggestions],
  );

  // Tab completes the highlighted directory into the input; Enter navigates
  // to it, unless the typed path names a directory outright (trailing "/"),
  // which wins over the highlighted child listed under it.
  const handlePathInputKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (handleNavigationKey(e)) return;
      if (e.key === "Tab" && highlightedSuggestion) {
        e.preventDefault();
        const next = highlightedSuggestion.path + "/";
        setPathInput(next);
        void fetchPathSuggestions(next);
      } else if (e.key === "Enter") {
        e.preventDefault();
        navigateTo(
          pathInput.endsWith("/") ? pathInput : (highlightedSuggestion?.path ?? pathInput),
        );
      }
    },
    [handleNavigationKey, highlightedSuggestion, pathInput, fetchPathSuggestions, navigateTo],
  );

  const handleDoubleClick = useCallback(
    (entry: DirEntry) => {
      navigateTo(entry.path);
    },
    [navigateTo],
  );

  const handleSelect = useCallback(async () => {
    // Select either the highlighted entry or the current directory
    const target = selectedEntry || currentPath;
    if (target) {
      if (selectionPending) return;
      setSelectionPending(true);
      try {
        await onSelect(target);
      } catch {
        // The parent exposes the failed workspace update through
        // `selectionError`; keep this dialog open for a corrected retry.
      } finally {
        setSelectionPending(false);
      }
    }
  }, [selectedEntry, currentPath, onSelect, selectionPending]);

  const handleClearFolder = useCallback(async () => {
    if (!onClearFolder || selectionPending) return;
    setSelectionPending(true);
    try {
      await onClearFolder();
    } catch {
      // See handleSelect: retain the dialog and let the parent render the
      // actionable error instead of producing an unhandled event promise.
    } finally {
      setSelectionPending(false);
    }
  }, [onClearFolder, selectionPending]);

  // Build breadcrumb segments from current path
  const breadcrumbs = (() => {
    if (!currentPath) return [];
    const parts = currentPath.split("/").filter(Boolean);
    const segments: { name: string; path: string }[] = [
      { name: "/", path: "/" },
    ];
    for (let i = 0; i < parts.length; i++) {
      segments.push({
        name: parts[i],
        path: "/" + parts.slice(0, i + 1).join("/"),
      });
    }
    return segments;
  })();

  // Keyboard navigation in list
  const handleListKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (selectionPending) return;
      const allItems = parentPath
        ? [{ name: "..", type: "directory", path: parentPath }, ...entries]
        : entries;
      const currentIndex = allItems.findIndex(
        (item) => item.path === selectedEntry,
      );

      if (e.key === "ArrowDown") {
        e.preventDefault();
        const next = Math.min(currentIndex + 1, allItems.length - 1);
        setSelectedEntry(allItems[next]?.path || null);
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        const prev = Math.max(currentIndex - 1, 0);
        setSelectedEntry(allItems[prev]?.path || null);
      } else if (e.key === "Enter") {
        e.preventDefault();
        if (selectedEntry) {
          const item = allItems.find((i) => i.path === selectedEntry);
          if (item) navigateTo(item.path);
        }
      }
    },
    [entries, parentPath, selectedEntry, navigateTo, selectionPending],
  );

  return (
    <Modal
      title="Select Folder"
      onClose={handleClose}
      closeDisabled={selectionPending}
      layout="flush"
      className="folder-picker-modal"
    >
      {/* Breadcrumb */}
      <div className="folder-picker-breadcrumb">
        {breadcrumbs.map((seg, i) => (
          <span key={seg.path}>
            {i > 0 && <span className="breadcrumb-sep">/</span>}
            <button
              className="breadcrumb-btn"
              disabled={selectionPending}
              onClick={() => navigateTo(seg.path)}
            >
              {seg.name === "/" ? (
                <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" style={{ verticalAlign: "middle" }}>
                  <path d="M3 9l5-5 5 5" />
                  <path d="M4 14V9h8v5" />
                </svg>
              ) : (
                seg.name
              )}
            </button>
          </span>
        ))}
      </div>

      {/* Path input with autocomplete */}
      <div className="folder-picker-path-input" style={{ position: "relative" }}>
        <input
          type="text"
          value={pathInput}
          onChange={(e) => {
            setPathInput(e.target.value);
            schedulePathSuggestions(e.target.value);
          }}
          disabled={selectionPending}
          onKeyDown={handlePathInputKeyDown}
          onFocus={() => {
            if (pathInput) fetchPathSuggestions(pathInput);
          }}
          onBlur={() => {
            // Delay to allow click on suggestion
            setTimeout(closeSuggestions, 150);
          }}
          placeholder="/path/to/folder"
          spellCheck={false}
          autoComplete="off"
        />
        {showSuggestions && pathSuggestions.length > 0 && (
          <ul className="folder-picker-suggestions">
            {pathSuggestions.map((entry, i) => (
              <li
                key={entry.path}
                className={i === suggestionIndex ? "active" : ""}
                onMouseDown={(e) => {
                  if (selectionPending) return;
                  e.preventDefault();
                  navigateTo(entry.path);
                }}
              >
                <span className="folder-icon-small">
                  <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
                    <path d="M2 4.5C2 3.67 2.67 3 3.5 3h2.59a1 1 0 01.7.29L8 4.5h4.5c.83 0 1.5.67 1.5 1.5v5.5c0 .83-.67 1.5-1.5 1.5h-9A1.5 1.5 0 012 11.5V4.5z"/>
                  </svg>
                </span>
                {entry.name}
              </li>
            ))}
          </ul>
        )}
      </div>

      {/* Directory listing */}
      <div
        className="folder-picker-list"
        tabIndex={0}
        onKeyDown={handleListKeyDown}
        role="listbox"
        aria-label="Folder contents"
      >
        {loading && (
          <div className="folder-picker-loading"><Spinner label="Loading…" /></div>
        )}
        {(selectionError || error) && (
          <div className="folder-picker-error">{selectionError || error}</div>
        )}
        {!loading && !error && !selectionError && (
          <>
            {parentPath && (
              <div
                className={`folder-picker-item${selectedEntry === parentPath ? " selected" : ""}`}
                role="option"
                aria-selected={selectedEntry === parentPath}
                onClick={() => !selectionPending && setSelectedEntry(parentPath)}
                onDoubleClick={() => !selectionPending && navigateTo(parentPath)}
              >
                <span className="folder-item-icon folder-item-icon-up">
                  <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
                    <path d="M8 12V4M4 8l4-4 4 4"/>
                  </svg>
                </span>
                <span className="folder-item-name">..</span>
              </div>
            )}
            {entries.length === 0 && !parentPath && (
              <div className="folder-picker-empty">Empty directory</div>
            )}
            {entries.map((entry) => (
              <div
                key={entry.path}
                className={`folder-picker-item${selectedEntry === entry.path ? " selected" : ""}`}
                role="option"
                aria-selected={selectedEntry === entry.path}
                onClick={() => !selectionPending && setSelectedEntry(entry.path)}
                onDoubleClick={() => !selectionPending && handleDoubleClick(entry)}
              >
                <span className="folder-item-icon">
                  <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
                    <path d="M2 4.5C2 3.67 2.67 3 3.5 3h2.59a1 1 0 01.7.29L8 4.5h4.5c.83 0 1.5.67 1.5 1.5v5.5c0 .83-.67 1.5-1.5 1.5h-9A1.5 1.5 0 012 11.5V4.5z"/>
                  </svg>
                </span>
                <span className="folder-item-name">{entry.name}</span>
              </div>
            ))}
          </>
        )}
      </div>

      {/* Footer showing selected path */}
      <div className="folder-picker-selected-info">
        <span className="folder-picker-selected-label">Selected:</span>
        <span className="folder-picker-selected-path">
          {selectedEntry || currentPath || "None"}
        </span>
      </div>

      {/* Actions */}
      <div className="modal-actions">
        {onClearFolder && initialPath && (
          <button
            type="button"
            className="btn-cancel folder-picker-clear-workspace"
            onClick={() => void handleClearFolder()}
            disabled={selectionPending}
          >
            Remove workspace folder
          </button>
        )}
        <button className="btn-cancel" onClick={onClose} disabled={selectionPending}>
          Cancel
        </button>
        <button
          className="btn-primary"
          onClick={() => void handleSelect()}
          disabled={selectionPending || (!currentPath && !selectedEntry)}
        >
          Select This Folder
        </button>
      </div>
    </Modal>
  );
}
