"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  importWorkspaceSessions,
  scanWorkspaceSessions,
  type HarnessKind,
  type ImportedWorkspaceSession,
  type NativeSessionSummary,
} from "../../lib/api/choruz-api";
import { Modal } from "../ui/modal";
import { FolderPickerModal } from "../workspace/folder-picker-modal";
import { PathPicker } from "../workspace/path-picker";

const HARNESSES: Array<{ id: HarnessKind; label: string }> = [
  { id: "claude", label: "Claude Code" },
  { id: "codex", label: "Codex" },
  { id: "pi", label: "Pi" },
  { id: "grok", label: "Grok" },
  { id: "open_code", label: "OpenCode" },
];

type Props = {
  sessionToken: string;
  activeCompanyId: string | null;
  onClose: () => void;
  onImported: (sessions: ImportedWorkspaceSession[]) => Promise<void> | void;
};

function selectionKey(session: Pick<NativeSessionSummary, "harness" | "native_session_id" | "workspace_path">) {
  return JSON.stringify([session.harness, session.native_session_id, session.workspace_path]);
}

function relativeWorkspace(root: string | null, workspace: string) {
  if (!root || workspace === root) return workspaceName(workspace);
  const prefix = root.endsWith("/") ? root : `${root}/`;
  return workspace.startsWith(prefix) ? workspace.slice(prefix.length) : workspace;
}

function workspaceName(path: string) {
  const parts = path.replace(/[\\/]+$/, "").split(/[\\/]/);
  return parts.at(-1)?.trim() || "Workspace";
}

export function ImportWorkspaceSessionsModal({
  sessionToken,
  activeCompanyId,
  onClose,
  onImported,
}: Props) {
  const scanRequestRef = useRef<AbortController | null>(null);
  const [workspacePath, setWorkspacePath] = useState("");
  const [showFolderPicker, setShowFolderPicker] = useState(false);
  const [harnesses, setHarnesses] = useState<Set<HarnessKind>>(
    () => new Set(HARNESSES.map((harness) => harness.id)),
  );
  const [sessions, setSessions] = useState<NativeSessionSummary[] | null>(null);
  const [canonicalWorkspace, setCanonicalWorkspace] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [warnings, setWarnings] = useState<string[]>([]);
  const [scanning, setScanning] = useState(false);
  const [query, setQuery] = useState("");
  const [importing, setImporting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => () => scanRequestRef.current?.abort(), []);

  const harnessLabels = useMemo(
    () => new Map(HARNESSES.map((harness) => [harness.id, harness.label])),
    [],
  );

  const orderedSessions = useMemo(
    () => [...(sessions ?? [])].sort((left, right) => {
      const timeDifference = Date.parse(right.updated_at) - Date.parse(left.updated_at);
      if (timeDifference !== 0) return timeDifference;
      const harnessDifference = left.harness.localeCompare(right.harness);
      return harnessDifference || left.native_session_id.localeCompare(right.native_session_id);
    }),
    [sessions],
  );

  const visibleSessions = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    if (!needle) return orderedSessions;
    return orderedSessions.filter((session) => [
      session.title,
      session.workspace_path,
      session.native_session_id,
      harnessLabels.get(session.harness) ?? session.harness,
      session.model ?? "",
      session.branch ?? "",
    ].some((value) => value.toLocaleLowerCase().includes(needle)));
  }, [harnessLabels, orderedSessions, query]);

  const toggleHarness = (harness: HarnessKind) => {
    setHarnesses((current) => {
      const next = new Set(current);
      if (next.has(harness)) next.delete(harness);
      else next.add(harness);
      return next;
    });
    scanRequestRef.current?.abort();
    setScanning(false);
    setSessions(null);
    setCanonicalWorkspace(null);
    setSelected(new Set());
  };

  const scan = useCallback(async (path: string, selectedHarnesses: HarnessKind[]) => {
    scanRequestRef.current?.abort();
    const controller = new AbortController();
    scanRequestRef.current = controller;
    setScanning(true);
    setError(null);
    setWarnings([]);
    try {
      const result = await scanWorkspaceSessions(
        sessionToken,
        path,
        selectedHarnesses,
        controller.signal,
      );
      setCanonicalWorkspace(result.workspace_path);
      setSessions(result.sessions);
      setWarnings(result.warnings);
      setSelected(new Set());
    } catch (reason) {
      if ((reason as Error).name !== "AbortError") {
        setError(reason instanceof Error ? reason.message : String(reason));
      }
    } finally {
      if (scanRequestRef.current === controller) setScanning(false);
    }
  }, [sessionToken]);

  const canScan = workspacePath.trim().length > 0 && harnesses.size > 0 && !scanning;
  const startScan = () => {
    if (!canScan) return;
    setSessions(null);
    setCanonicalWorkspace(null);
    setSelected(new Set());
    void scan(workspacePath.trim(), [...harnesses]);
  };

  const importSelected = async () => {
    if (!canonicalWorkspace || !sessions || selected.size === 0 || importing) return;
    if (!activeCompanyId) {
      setError("Choose a company before importing sessions.");
      return;
    }
    setImporting(true);
    setError(null);
    try {
      const chosen = sessions.filter((session) => selected.has(selectionKey(session)));
      const result = await importWorkspaceSessions(
        sessionToken,
        activeCompanyId,
        canonicalWorkspace,
        chosen,
      );
      await onImported(result.imported);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setImporting(false);
    }
  };

  return (
    <>
      <Modal
        title="Import Sessions"
        description="Choose existing sessions from a folder and all of its subfolders. Nothing starts until you message an imported Agent."
        onClose={onClose}
        className="workspace-session-import-card"
      >
        {error ? <p className="server-manager-error-block" role="alert">{error}</p> : null}

        <section className="remote-control-section workspace-session-scope" aria-labelledby="workspace-path-title">
          <div className="workspace-session-section-heading">
            <div>
              <h3 id="workspace-path-title">Folder scope</h3>
              <p className="server-manager-hint">Choose a folder, then scan it and its subfolders.</p>
            </div>
            <span className="workspace-session-scan-status" role="status" aria-live="polite">
              {scanning ? "Scanning…" : sessions ? "Up to date" : workspacePath.trim() ? "Ready to scan" : "Choose a folder"}
            </span>
          </div>
          <div className="workspace-session-folder-row">
            <PathPicker value={workspacePath} onChange={(path) => {
              setWorkspacePath(path);
              scanRequestRef.current?.abort();
              setScanning(false);
              setSessions(null);
              setCanonicalWorkspace(null);
              setSelected(new Set());
            }} placeholder="/path/to/project" />
            <button type="button" className="server-manager-btn" onClick={() => setShowFolderPicker(true)}>
              Browse
            </button>
            <button type="button" className="server-manager-btn server-manager-btn--primary" disabled={!canScan} onClick={startScan}>
              {scanning ? "Scanning…" : "Scan"}
            </button>
          </div>
          <fieldset className="workspace-session-harnesses">
            <legend>Harnesses</legend>
            {HARNESSES.map((harness) => (
              <label key={harness.id}>
                <input
                  type="checkbox"
                  checked={harnesses.has(harness.id)}
                  onChange={() => toggleHarness(harness.id)}
                />
                <span>{harness.label}</span>
              </label>
            ))}
          </fieldset>
        </section>

        {warnings.length > 0 ? (
          <div className="workspace-session-warnings" role="status">
            {warnings.map((warning) => <p key={warning}>{warning}</p>)}
          </div>
        ) : null}

        {sessions || scanning ? (
          <section className="remote-control-section workspace-session-results" aria-labelledby="workspace-session-results-title">
            <div className="workspace-session-results-heading">
              <div>
                <h3 id="workspace-session-results-title">Choose sessions</h3>
                <p className="server-manager-hint">
                  {sessions ? `${sessions.length} found · ${selected.size} selected · newest first` : "Scanning…"}
                </p>
              </div>
              {sessions && sessions.length > 0 ? (
                <button type="button" className="server-manager-btn" onClick={() => {
                  setSelected((current) => current.size === sessions.length
                    ? new Set()
                    : new Set(sessions.map(selectionKey)));
                }}>
                  {selected.size === sessions.length ? "Clear all" : "Select all"}
                </button>
              ) : null}
            </div>
            {sessions && sessions.length > 0 ? (
              <input
                type="search"
                className="workspace-session-search"
                aria-label="Filter sessions"
                placeholder="Filter by title, folder, harness, model, or branch"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
              />
            ) : null}
            {scanning && !sessions ? <div className="workspace-session-loading" aria-hidden="true" /> : null}
            {sessions?.length === 0 && !scanning ? <p>No sessions from this folder or its subfolders were found.</p> : null}
            {sessions && sessions.length > 0 && visibleSessions.length === 0 ? <p>No sessions match this filter.</p> : null}
            {visibleSessions.length > 0 ? (
              <div className="workspace-session-list" aria-label="Sessions ordered newest first">
                {visibleSessions.map((session) => {
                  const key = selectionKey(session);
                  const harnessLabel = harnessLabels.get(session.harness) ?? session.harness;
                  return (
                    <label className="workspace-session-row" key={key}>
                      <input type="checkbox" checked={selected.has(key)} onChange={() => {
                        setSelected((current) => {
                          const next = new Set(current);
                          if (next.has(key)) next.delete(key);
                          else next.add(key);
                          return next;
                        });
                      }} />
                      <span className="workspace-session-row-content">
                        <span className="workspace-session-row-title">
                          <strong>{session.title || `${harnessLabel} session`}</strong>
                          <span className="workspace-session-harness-badge">{harnessLabel}</span>
                        </span>
                        <small className="workspace-session-row-workspace" title={session.workspace_path}>
                          {relativeWorkspace(canonicalWorkspace, session.workspace_path)}
                        </small>
                        <small>
                          {new Date(session.updated_at).toLocaleString("en-US")}
                          {session.model ? ` · ${session.model}` : ""}
                          {session.branch ? ` · ${session.branch}` : ""}
                        </small>
                      </span>
                    </label>
                  );
                })}
              </div>
            ) : null}
            {sessions && sessions.length > 0 ? <button
              type="button"
              className="server-manager-btn server-manager-btn--primary"
              disabled={importing || selected.size === 0}
              onClick={() => void importSelected()}
            >
              {importing ? "Importing…" : `Import ${selected.size} session${selected.size === 1 ? "" : "s"}`}
            </button> : null}
          </section>
        ) : null}
      </Modal>
      {showFolderPicker ? (
        <FolderPickerModal
          initialPath={workspacePath || undefined}
          onSelect={(path) => {
            setWorkspacePath(path);
            scanRequestRef.current?.abort();
            setScanning(false);
            setSessions(null);
            setCanonicalWorkspace(null);
            setSelected(new Set());
            setShowFolderPicker(false);
          }}
          onClose={() => setShowFolderPicker(false)}
        />
      ) : null}
    </>
  );
}
