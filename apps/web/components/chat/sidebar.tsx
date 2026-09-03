"use client";

import { useCallback, useEffect, useMemo, useRef, useState, type MouseEvent, type CSSProperties } from "react";
import { useTheme } from "next-themes";
import { ChevronDown, ChevronRight, MoreHorizontal } from "lucide-react";
import { trace } from "../../lib/api/choruz-trace";
import { EmptyState } from "../ui/empty-state";
import type {
  Principal,
  Conversation,
  ChatMessage,
  PinnedConversation,
  ArchivedConversation,
  HiddenConversation,
  RuntimeBindingInfo,
  Company,
} from "../../lib/api/choruz-types";
import {
  buildSidebarConversationSections,
  type SidebarConversationSectionId,
} from "../../lib/messages/sidebar-conversations";
import { conversationDisplayName } from "../../lib/api/principals";
import type { UnreadEntry } from "../../lib/messages/thread-unreads";
import { FileTree } from "../workspace/file-tree";
import { FolderPickerModal } from "../workspace/folder-picker-modal";
import { Modal } from "../ui/modal";
import { ConversationListItem } from "./conversation-list-item";
import { Avatar } from "../ui/avatar";
import { transportFetch } from "../../lib/api/transport";

// ResetSessionsButton removed from UI — backend endpoint preserved at
// POST /v1/companies/{id}/reset-sessions for local console/API use.

/**
 * Light/dark toggle entry in the user dropdown. Resolves the current
 * theme via next-themes and flips it. Two `useTheme()` quirks:
 * - `theme` is undefined on first paint until the provider hydrates.
 *   We read `resolvedTheme` (always concrete after hydration) and fall
 *   back to "light" for the initial label so the menu doesn't flash.
 * - The label shows the *target* theme ("Switch to dark") since that's
 *   what clicking does, not the *current* theme.
 */
function ThemeMenuItem({ onSelect }: { onSelect: () => void }) {
  const { resolvedTheme, setTheme } = useTheme();
  const current = resolvedTheme ?? "light";
  const next = current === "dark" ? "light" : "dark";
  return (
    <button
      className="dropdown-menu-item"
      onClick={() => {
        trace.event("toggle_theme", { from: current, to: next });
        setTheme(next);
        onSelect();
      }}
    >
      Switch to {next} mode
    </button>
  );
}

export type SidebarProps = {
  open: boolean;
  principal: Principal;
  conversations: Conversation[];
  agents: Principal[];
  messagesByConv: Record<string, ChatMessage[]>;
  pinnedConversations: PinnedConversation[];
  archivedConversations: ArchivedConversation[];
  hiddenConversations: HiddenConversation[];
  runtimeBindings?: RuntimeBindingInfo[];
  pinPendingConversationIds: Set<string>;
  archivePendingConversationIds: Set<string>;
  hidePendingConversationIds: Set<string>;
  activeConvId: string | null;
  onTogglePin: (conversationId: string, nextPinned: boolean) => void;
  onToggleArchive: (conversationId: string, nextArchived: boolean) => void;
  onRestoreHiddenSession: (conversationId: string) => void;
  onHideSession: (conversationId: string) => void;
  onSelectConversation: (convId: string) => void;
  onCreateAgent: () => void;
  onManageHarnessAccounts?: () => void;
  onCreateGroup: () => void;
  sessionToken: string;
  refreshSnapshot: () => void;
  hasMoreConversations?: boolean;
  loadingMoreConversations?: boolean;
  onLoadMoreConversations?: () => void;
  trackEvent?: (event: string, data?: Record<string, unknown>) => void;
  companies?: Company[];
  activeCompanyId?: string | null;
  onSelectCompany?: (companyId: string) => void;
  onCreateCompany?: () => void;
  onToggleAgentsActive?: (companyId: string, active: boolean) => Promise<void>;
  pluginActions?: ReadonlyArray<{ id: string; label: string; onSelect: () => void }>;
  onArchiveCompany?: (companyId: string) => Promise<void>;
  onUnarchiveCompany?: (companyId: string) => Promise<void>;
  onDeleteCompany?: (companyId: string) => Promise<void>;
  onRenameCompany?: (companyId: string, newName: string) => Promise<void>;
  onChangeCompanyWorkspace?: (companyId: string, folderPath: string) => Promise<void>;
  onOpenCompanyMachines?: (companyId: string) => void;
  unreads?: Record<string, UnreadEntry>;
  style?: CSSProperties;
  extraClassName?: string;
  onOpenFile?: (path: string) => void;
};

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function Sidebar({
  open,
  principal,
  conversations,
  agents,
  messagesByConv,
  pinnedConversations,
  archivedConversations,
  hiddenConversations,
  runtimeBindings = [],
  pinPendingConversationIds,
  archivePendingConversationIds,
  hidePendingConversationIds,
  activeConvId,
  onTogglePin,
  onToggleArchive,
  onRestoreHiddenSession,
  onHideSession,
  onSelectConversation,
  onCreateAgent,
  onManageHarnessAccounts,
  onCreateGroup,
  sessionToken,
  refreshSnapshot,
  hasMoreConversations = false,
  loadingMoreConversations = false,
  onLoadMoreConversations,
  trackEvent,
  companies = [],
  activeCompanyId,
  onSelectCompany,
  onCreateCompany,
  onToggleAgentsActive,
  pluginActions = [],
  onArchiveCompany,
  onUnarchiveCompany,
  onDeleteCompany,
  onRenameCompany,
  onChangeCompanyWorkspace,
  onOpenCompanyMachines,
  unreads = {},
  style,
  extraClassName,
  onOpenFile,
}: SidebarProps) {
  const [searchFilter, setSearchFilter] = useState("");
  const searchDebounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [manageMode, setManageMode] = useState(false);
  const [expandedSections, setExpandedSections] = useState<
    Partial<Record<SidebarConversationSectionId, boolean>>
  >({});
  const [showPlusMenu, setShowPlusMenu] = useState(false);
  const [showUserMenu, setShowUserMenu] = useState(false);
  const actionsMenuButtonRef = useRef<HTMLButtonElement>(null);
  const userMenuButtonRef = useRef<HTMLButtonElement>(null);
  const [selectedConvIds, setSelectedConvIds] = useState<Set<string>>(new Set());
  const [deleting, setDeleting] = useState(false);
  const [showHiddenSessions, setShowHiddenSessions] = useState(false);

  useEffect(() => {
    if (!showPlusMenu && !showUserMenu) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      if (showPlusMenu) {
        setShowPlusMenu(false);
        actionsMenuButtonRef.current?.focus();
      }
      if (showUserMenu) {
        setShowUserMenu(false);
        userMenuButtonRef.current?.focus();
      }
    };
    document.addEventListener("keydown", closeOnEscape);
    return () => document.removeEventListener("keydown", closeOnEscape);
  }, [showPlusMenu, showUserMenu]);

  // Company management state
  const [companyDropdownOpen, setCompanyDropdownOpen] = useState(false);
  const [workspacePickerCompanyId, setWorkspacePickerCompanyId] = useState<string | null>(null);
  const [workspaceUpdateError, setWorkspaceUpdateError] = useState<string | null>(null);
  const [contextMenuId, setContextMenuId] = useState<string | null>(null);
  const [renameId, setRenameId] = useState<string | null>(null);
  const [renameText, setRenameText] = useState("");
  const [hiddenCompanies, setHiddenCompanies] = useState<string[]>([]);
  const [showHiddenInDropdown, setShowHiddenInDropdown] = useState(false);
  useEffect(() => {
    try {
      const saved = JSON.parse(localStorage.getItem("choruz_hidden_companies") ?? "[]");
      if (Array.isArray(saved)) setHiddenCompanies(saved);
    } catch {}
  }, []);
  const [deleteConfirmId, setDeleteConfirmId] = useState<string | null>(null);
  const [deleteConfirmText, setDeleteConfirmText] = useState("");
  const [companyActionLoading, setCompanyActionLoading] = useState<string | null>(null);
  const [companySelectMode, setCompanySelectMode] = useState(false);
  const [selectedCompanyIds, setSelectedCompanyIds] = useState<Set<string>>(new Set());

  // Explorer / conversation list vertical resize
  const [explorerHeight, setExplorerHeight] = useState<number | null>(null);
  const explorerResizing = useMemo(() => ({ active: false, startY: 0, startH: 0 }), []);
  const onExplorerResizeMouseDown = useCallback((e: MouseEvent) => {
    e.preventDefault();
    explorerResizing.active = true;
    explorerResizing.startY = e.clientY;
    explorerResizing.startH = explorerHeight ?? 200;
    const onMove = (ev: globalThis.MouseEvent) => {
      if (!explorerResizing.active) return;
      const delta = ev.clientY - explorerResizing.startY;
      const newH = Math.max(60, Math.min(explorerResizing.startH + delta, window.innerHeight - 200));
      setExplorerHeight(newH);
    };
    const onUp = () => {
      explorerResizing.active = false;
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
    };
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
  }, [explorerHeight, explorerResizing]);

  // Persist hidden companies to localStorage
  const updateHiddenCompanies = useCallback((next: string[]) => {
    setHiddenCompanies(next);
    try { localStorage.setItem("choruz_hidden_companies", JSON.stringify(next)); } catch {}
  }, []);

  // Filter companies: hide hidden ones from dropdown unless toggled
  const visibleCompanies = useMemo(() =>
    companies.filter(c => !hiddenCompanies.includes(c.id)),
    [companies, hiddenCompanies],
  );

  const activeCompany = useMemo(() =>
    companies.find(c => c.id === activeCompanyId),
    [companies, activeCompanyId],
  );

  // Companies shown in dropdown: visible + optionally hidden
  const dropdownCompanies = useMemo(() => {
    const list = showHiddenInDropdown ? companies : visibleCompanies;
    return [...list].sort((a, b) => {
      const aArchived = !!a.archived_at;
      const bArchived = !!b.archived_at;
      if (aArchived !== bArchived) return aArchived ? 1 : -1;
      return a.name.localeCompare(b.name);
    });
  }, [companies, visibleCompanies, showHiddenInDropdown]);

  const sidebarSections = useMemo(() =>
    buildSidebarConversationSections({
      conversations,
      agents,
      principal,
      messagesByConv,
      searchQuery: searchFilter,
      pinnedConversations,
      archivedConversations,
      hiddenConversations,
      activeConvId,
      runtimeBindings,
    }),
    [activeConvId, agents, archivedConversations, conversations, hiddenConversations, messagesByConv, pinnedConversations, principal, runtimeBindings, searchFilter],
  );

  const toggleSection = useCallback((
    sectionId: SidebarConversationSectionId,
    isExpanded: boolean,
  ) => {
    setExpandedSections((prev) => ({
      ...prev,
      [sectionId]: !isExpanded,
    }));
  }, []);
  const allFilteredConversationIds = sidebarSections.allFilteredConversationIds;
  const allFilteredSelected =
    allFilteredConversationIds.length > 0 &&
    allFilteredConversationIds.every((convId) => selectedConvIds.has(convId));

  const toggleSelection = useCallback((convId: string) => {
    setSelectedConvIds((prev) => {
      const next = new Set(prev);
      if (next.has(convId)) {
        next.delete(convId);
      } else {
        next.add(convId);
      }
      return next;
    });
  }, []);

  const selectAll = useCallback(() => {
    setSelectedConvIds(new Set(allFilteredConversationIds));
  }, [allFilteredConversationIds]);

  const deselectAll = useCallback(() => {
    setSelectedConvIds(new Set());
  }, []);

  const handleBatchDelete = useCallback(async () => {
    if (selectedConvIds.size === 0) return;
    const selectedConvs = conversations.filter((c) => selectedConvIds.has(c.id));

    // Collect agent IDs and conversation IDs to delete
    const agentIdsToDisable: string[] = [];
    const conversationIdsToDelete: string[] = [];
    for (const conv of selectedConvs) {
      conversationIdsToDelete.push(conv.id);
      // Only disable agents for DIRECT conversations (deleting the agent itself)
      // Group conversations: just delete the conversation, keep agents alive
      if (conv.conversation_type === "direct") {
        const otherMember = Object.keys(conv.members).find(
          (id) => id !== principal.id,
        );
        if (otherMember) {
          agentIdsToDisable.push(otherMember);
        }
      }
      // Groups: conversation_id is already added, agents stay alive
    }

    setDeleting(true);
    // Inherit the click's trace id — the global click tracker started one
    // when the "Delete" button was pressed.
    const span = trace.start("batch_delete", { count: selectedConvIds.size, agents: agentIdsToDisable.length, conversations: conversationIdsToDelete.length });
    try {
      const res = await transportFetch("/api/agents/batch-disable", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          actor_id: principal.id,
          agent_ids: [...new Set(agentIdsToDisable)],
          conversation_ids: conversationIdsToDelete,
        }),
      });
      if (!res.ok) {
        const body = await res.json().catch(() => ({}));
        span.end({ error: `HTTP ${res.status}`, body });
      } else {
        span.end({ status: res.status });
      }
      trackEvent?.("batch_delete", { count: selectedConvIds.size });
      await refreshSnapshot();
      setManageMode(false);
      setSelectedConvIds(new Set());
    } catch (err) {
      span.end({ error: String(err) });
    } finally {
      setDeleting(false);
    }
  }, [selectedConvIds, conversations, principal, agents, sessionToken, refreshSnapshot, trackEvent]);

  const exitManageMode = useCallback(() => {
    setManageMode(false);
    setSelectedConvIds(new Set());
  }, []);

  // Count how many selected items are deletable (direct convos with agents)
  const selectedDeleteCount = selectedConvIds.size;

  return (
    <aside className={`chat-sidebar${open ? " open" : ""}${extraClassName ? " " + extraClassName : ""}`} style={style}>
      <div className="sidebar-brand" aria-label="Choruz">
        <img
          className="sidebar-brand-lockup sidebar-brand-lockup-light"
          src="/brand/choruz-lockup.svg"
          alt="Choruz"
        />
        <img
          className="sidebar-brand-lockup sidebar-brand-lockup-dark"
          src="/brand/choruz-lockup-dark.svg"
          alt=""
          aria-hidden="true"
        />
      </div>
      {/* Company selector — ChatGPT-style dropdown */}
      {companies.length > 0 && (
        <div className="company-selector">
          <button
            className="company-selector-btn"
            onClick={() => setCompanyDropdownOpen(!companyDropdownOpen)}
            aria-label="Select company"
            aria-expanded={companyDropdownOpen}
          >
            <span className="company-selector-name">
              {activeCompany?.name ?? "Select company"}
            </span>
            <span className="company-selector-arrow">{companyDropdownOpen ? "\u25B4" : "\u25BE"}</span>
          </button>

          {companyDropdownOpen && (
            <>
              <div
                className="dropdown-backdrop"
                onClick={() => {
                  setCompanyDropdownOpen(false);
                  setContextMenuId(null);
                  setRenameId(null);
                  setDeleteConfirmId(null);
                  setDeleteConfirmText("");
                }}
              />
              <div className="company-dropdown">
                {dropdownCompanies.map((c) => {
                  const isHidden = hiddenCompanies.includes(c.id);
                  return (
                    <div
                      key={c.id}
                      className={`company-dropdown-item${c.id === activeCompanyId ? " active" : ""}${c.archived_at ? " archived" : ""}`}
                    >
                      <>
                          {companySelectMode && (
                            <input
                              type="checkbox"
                              checked={selectedCompanyIds.has(c.id)}
                              onChange={() => {
                                setSelectedCompanyIds(prev => {
                                  const next = new Set(prev);
                                  if (next.has(c.id)) next.delete(c.id); else next.add(c.id);
                                  return next;
                                });
                              }}
                              className="company-select-checkbox"
                            />
                          )}
                          {renameId === c.id ? (
                              <input
                                autoFocus
                                className="company-rename-input"
                                value={renameText}
                                onChange={(e) => setRenameText(e.target.value)}
                                onKeyDown={async (e) => {
                                  if (e.key === "Enter" && renameText.trim() && renameText !== c.name) {
                                    e.preventDefault();
                                    const span = trace.start("rename_company", { companyId: c.id, oldName: c.name, newName: renameText.trim() });
                                    setCompanyActionLoading(c.id);
                                    try {
                                      await onRenameCompany?.(c.id, renameText.trim());
                                      span.end({ status: "ok" });
                                    } catch (err) {
                                      span.end({ error: String(err) });
                                    } finally {
                                      setCompanyActionLoading(null);
                                      setRenameId(null);
                                    }
                                  }
                                  if (e.key === "Escape") { setRenameId(null); }
                                }}
                                onBlur={async () => {
                                  if (renameText.trim() && renameText !== c.name) {
                                    const span = trace.start("rename_company", { companyId: c.id, oldName: c.name, newName: renameText.trim() });
                                    setCompanyActionLoading(c.id);
                                    try {
                                      await onRenameCompany?.(c.id, renameText.trim());
                                      span.end({ status: "ok" });
                                    } catch (err) {
                                      span.end({ error: String(err) });
                                    } finally {
                                      setCompanyActionLoading(null);
                                      setRenameId(null);
                                    }
                                  } else {
                                    setRenameId(null);
                                  }
                                }}
                              />
                          ) : (
                          <button
                            type="button"
                            className="company-dropdown-item-name"
                            onClick={() => {
                              if (companySelectMode) {
                                setSelectedCompanyIds(prev => {
                                  const next = new Set(prev);
                                  if (next.has(c.id)) next.delete(c.id); else next.add(c.id);
                                  return next;
                                });
                                return;
                              }
                              trace.event("company_switch", { companyId: c.id, companyName: c.name });
                              onSelectCompany?.(c.id);
                              setCompanyDropdownOpen(false);
                              setContextMenuId(null);
                            }}
                          >
                            {!companySelectMode && c.id === activeCompanyId && <span className="company-check">{"✓"}</span>}
                            <span>{c.name}</span>
                            {c.archived_at && <span className="company-archived-badge">archived</span>}
                            {isHidden && <span className="company-hidden-badge">hidden</span>}
                          </button>
                          )}
                          <button
                            className="company-dots-btn"
                            onClick={(e) => {
                              e.stopPropagation();
                              setContextMenuId(contextMenuId === c.id ? null : c.id);
                            }}
                            aria-label={`Actions for ${c.name}`}
                          >
                            <MoreHorizontal size={16} aria-hidden="true" />
                          </button>

                          {/* Context menu */}
                          {contextMenuId === c.id && (
                            <div className="company-context-menu">
                              {onOpenCompanyMachines ? (
                                <button onClick={() => {
                                  trace.event("open_company_machines", { companyId: c.id, companyName: c.name });
                                  onOpenCompanyMachines(c.id);
                                  setContextMenuId(null);
                                  setCompanyDropdownOpen(false);
                                }}>
                                  Machines
                                </button>
                              ) : null}
                              <button onClick={() => {
                                trace.event("rename_company_start", { companyId: c.id, companyName: c.name });
                                setRenameId(c.id);
                                setRenameText(c.name);
                                setContextMenuId(null);
                              }}>
                                Rename
                              </button>
                              <button onClick={() => {
                                trace.event("hide_company", { companyId: c.id, companyName: c.name, hidden: !isHidden });
                                if (isHidden) {
                                  updateHiddenCompanies(hiddenCompanies.filter(id => id !== c.id));
                                } else {
                                  updateHiddenCompanies([...hiddenCompanies, c.id]);
                                }
                                setContextMenuId(null);
                              }}>
                                {isHidden ? "Show" : "Hide"}
                              </button>
                              <button
                                disabled={companyActionLoading === c.id}
                                onClick={async () => {
                                  const action = c.archived_at ? "unarchive_company" : "archive_company";
                                  const span = trace.start(action, { companyId: c.id, companyName: c.name });
                                  setCompanyActionLoading(c.id);
                                  try {
                                    if (c.archived_at) {
                                      await onUnarchiveCompany?.(c.id);
                                    } else {
                                      await onArchiveCompany?.(c.id);
                                    }
                                    span.end({ status: "ok" });
                                  } catch (err) {
                                    span.end({ error: String(err) });
                                  } finally {
                                    setCompanyActionLoading(null);
                                    setContextMenuId(null);
                                  }
                                }}
                              >
                                {c.archived_at ? "Unarchive" : "Archive"}
                              </button>
                              <div className="dropdown-menu-divider" role="separator" />
                              <button
                                className="company-context-danger"
                                disabled={companyActionLoading === c.id}
                                onClick={async () => {
                                  if (!window.confirm(`Delete "${c.name}"? This can be recovered within 30 days.`)) return;
                                  const span = trace.start("delete_company", { companyId: c.id, companyName: c.name });
                                  setCompanyActionLoading(c.id);
                                  try {
                                    await onDeleteCompany?.(c.id);
                                    span.end({ status: "ok" });
                                  } catch (err) {
                                    span.end({ error: String(err) });
                                  } finally {
                                    setCompanyActionLoading(null);
                                    setContextMenuId(null);
                                  }
                                }}
                              >
                                Delete
                              </button>
                            </div>
                          )}
                      </>
                    </div>
                  );
                })}

                {/* Show/hide hidden companies toggle */}
                {hiddenCompanies.length > 0 && (
                  <button
                    className="company-show-hidden"
                    onClick={() => setShowHiddenInDropdown(!showHiddenInDropdown)}
                  >
                    {showHiddenInDropdown
                      ? "Hide hidden companies"
                      : `Show ${hiddenCompanies.length} hidden`}
                  </button>
                )}

                {/* Select mode toggle + batch actions */}
                {!companySelectMode ? (
                  <button
                    className="company-show-hidden"
                    onClick={() => { setCompanySelectMode(true); setSelectedCompanyIds(new Set()); }}
                  >
                    Select
                  </button>
                ) : (
                  <div className="company-batch-bar">
                    <span className="company-batch-count">{selectedCompanyIds.size} selected</span>
                    <button
                      className="company-batch-btn"
                      onClick={() => setSelectedCompanyIds(new Set(dropdownCompanies.map(c => c.id)))}
                    >All</button>
                    <button
                      className="company-batch-btn"
                      disabled={selectedCompanyIds.size === 0}
                      onClick={() => {
                        trace.event("batch_company_action", { action: "hide", count: selectedCompanyIds.size });
                        const next = [...hiddenCompanies, ...selectedCompanyIds];
                        updateHiddenCompanies([...new Set(next)]);
                        setCompanySelectMode(false);
                      }}
                    >Hide</button>
                    <button
                      className="company-batch-btn"
                      disabled={selectedCompanyIds.size === 0 || companyActionLoading !== null}
                      onClick={async () => {
                        const span = trace.start("batch_company_action", { action: "archive", count: selectedCompanyIds.size });
                        try {
                          for (const id of selectedCompanyIds) {
                            setCompanyActionLoading(id);
                            try { await onArchiveCompany?.(id); } catch {}
                          }
                          span.end({ status: "ok" });
                        } catch (err) {
                          span.end({ error: String(err) });
                        } finally {
                          setCompanyActionLoading(null);
                          setCompanySelectMode(false);
                        }
                      }}
                    >Archive</button>
                    <button
                      className="company-batch-btn is-danger"
                      disabled={selectedCompanyIds.size === 0 || companyActionLoading !== null}
                      onClick={async () => {
                        if (!window.confirm(`Delete ${selectedCompanyIds.size} companies?`)) return;
                        const span = trace.start("batch_company_action", { action: "delete", count: selectedCompanyIds.size });
                        try {
                          for (const id of selectedCompanyIds) {
                            setCompanyActionLoading(id);
                            try { await onDeleteCompany?.(id); } catch {}
                          }
                          span.end({ status: "ok" });
                        } catch (err) {
                          span.end({ error: String(err) });
                        } finally {
                          setCompanyActionLoading(null);
                          setCompanySelectMode(false);
                        }
                      }}
                    >Delete</button>
                    <button
                      className="company-batch-btn"
                      onClick={() => { setCompanySelectMode(false); setSelectedCompanyIds(new Set()); }}
                    >Cancel</button>
                  </div>
                )}
              </div>
            </>
          )}
        </div>
      )}
      {/* File tree explorer — shown when active company has a folder_path */}
      {(() => {
        const activeCompany = companies.find((c) => c.id === activeCompanyId);
        return activeCompany?.folder_path ? (
          <>
            <FileTree
              rootPath={activeCompany.folder_path}
              workspaceId={activeCompany.id}
              style={explorerHeight != null ? { maxHeight: explorerHeight } : undefined}
              onOpenFile={onOpenFile}
              onChangeRoot={onChangeCompanyWorkspace ? () => {
                setWorkspaceUpdateError(null);
                setWorkspacePickerCompanyId(activeCompany.id);
              } : undefined}
            />
            <div
              className="explorer-resize-handle"
              onMouseDown={onExplorerResizeMouseDown}
              role="separator"
              aria-orientation="horizontal"
              aria-label="Resize file explorer"
            />
          </>
        ) : null;
      })()}

      {workspacePickerCompanyId && (() => {
        const company = companies.find((candidate) => candidate.id === workspacePickerCompanyId);
        if (!company) return null;
        return (
          <FolderPickerModal
            initialPath={company.folder_path ?? undefined}
            onClose={() => {
              setWorkspaceUpdateError(null);
              setWorkspacePickerCompanyId(null);
            }}
            onSelect={(folderPath) => {
              if (!onChangeCompanyWorkspace) return Promise.resolve();
              return onChangeCompanyWorkspace(company.id, folderPath)
                .then(() => {
                  setWorkspaceUpdateError(null);
                  setWorkspacePickerCompanyId(null);
                })
                .catch((error) => {
                  setWorkspaceUpdateError(
                    error instanceof Error ? error.message : "Could not update workspace folder.",
                  );
                  throw error;
                });
            }}
            onClearFolder={() => {
              if (!onChangeCompanyWorkspace) return Promise.resolve();
              return onChangeCompanyWorkspace(company.id, "")
                .then(() => {
                  setWorkspaceUpdateError(null);
                  setWorkspacePickerCompanyId(null);
                })
                .catch((error) => {
                  setWorkspaceUpdateError(
                    error instanceof Error ? error.message : "Could not remove workspace folder.",
                  );
                  throw error;
                });
            }}
            selectionError={workspaceUpdateError}
          />
        );
      })()}

      <div className="sidebar-header">
        <div className="user-info menu-anchor">
          <button
            ref={userMenuButtonRef}
            type="button"
            className="user-menu-trigger"
            onClick={() => setShowUserMenu(!showUserMenu)}
            aria-label="User menu"
            aria-expanded={showUserMenu}
          >
            <Avatar name={principal.name} />
          </button>
          <span>{principal.name}</span>
          {showUserMenu && (
            <>
              <div
                className="dropdown-backdrop"
                onClick={() => setShowUserMenu(false)}
              />
              <div className="dropdown-menu align-left">
                <ThemeMenuItem onSelect={() => setShowUserMenu(false)} />
                <a
                  href="/docs"
                  target="_blank"
                  rel="noopener"
                  onClick={() => setShowUserMenu(false)}
                  className="dropdown-menu-item"
                >
                  Documentation
                </a>
              </div>
            </>
          )}
        </div>
        <div className="sidebar-actions">
          {!manageMode ? (
            <div className="menu-anchor">
              <button
                ref={actionsMenuButtonRef}
                title="Actions"
                data-modal-return-focus
                onClick={() => { if (!showPlusMenu) trace.event("create_menu_open"); setShowPlusMenu(!showPlusMenu); }}
                aria-label="Actions menu"
                className="sidebar-action-btn"
              >
                +
              </button>
              {showPlusMenu && (
                <>
                  <div
                    className="dropdown-backdrop"
                    onClick={() => setShowPlusMenu(false)}
                  />
                  <div className="dropdown-menu align-right">
                    <button onClick={() => { onCreateAgent(); setShowPlusMenu(false); }} className="dropdown-menu-item">
                      Create Agent
                    </button>
                    {onManageHarnessAccounts ? (
                      <button onClick={() => { onManageHarnessAccounts(); setShowPlusMenu(false); }} className="dropdown-menu-item">
                        Harness Accounts
                      </button>
                    ) : null}
                    <button onClick={() => { onCreateGroup(); setShowPlusMenu(false); }} className="dropdown-menu-item">
                      New Group
                    </button>
                    <button onClick={() => { onCreateCompany?.(); setShowPlusMenu(false); }} className="dropdown-menu-item">
                      New Company
                    </button>
                    <div className="dropdown-menu-divider" role="separator" />
                    <button onClick={() => { trace.event("manage_chats_toggle", { enabled: true }); setManageMode(true); setShowPlusMenu(false); }} className="dropdown-menu-item">
                      Manage Chats
                    </button>
                    {hiddenConversations.length > 0 && (
                      <button onClick={() => { setShowHiddenSessions(true); setShowPlusMenu(false); }} className="dropdown-menu-item">
                        Restore hidden sessions
                      </button>
                    )}
                    {pluginActions.map((action) => (
                      <button
                        key={action.id}
                        onClick={() => { action.onSelect(); setShowPlusMenu(false); }}
                        className="dropdown-menu-item"
                      >
                        {action.label}
                      </button>
                    ))}
                  </div>
                </>
              )}
            </div>
          ) : (
            <>
              <button
                title="Select all"
                className="sidebar-action-btn is-text"
                onClick={allFilteredSelected ? deselectAll : selectAll}
              >
                {allFilteredSelected ? "None" : "All"}
              </button>
              <button
                title="Cancel"
                className="sidebar-action-btn is-text"
                onClick={exitManageMode}
              >
                Cancel
              </button>
            </>
          )}
        </div>
      </div>

      <div className="sidebar-search">
        <input
          placeholder="Search conversations…"
          value={searchFilter}
          onChange={(e) => {
            const val = e.target.value;
            setSearchFilter(val);
            if (searchDebounceRef.current) clearTimeout(searchDebounceRef.current);
            searchDebounceRef.current = setTimeout(() => {
              if (val.trim()) trace.event("search_filter", { query: val.trim() });
            }, 500);
          }}
        />
      </div>

      <div
        className="conversation-list"
        aria-label="Conversations"
        onKeyDown={(event) => {
          if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
          const items = Array.from(
            event.currentTarget.querySelectorAll<HTMLElement>(".conv-item-main"),
          ).filter((item) => item.offsetParent !== null);
          const current = items.indexOf(document.activeElement as HTMLElement);
          if (current < 0 || items.length === 0) return;
          event.preventDefault();
          const delta = event.key === "ArrowDown" ? 1 : -1;
          items[(current + delta + items.length) % items.length]?.focus();
        }}
      >
        {sidebarSections.sections.filter((section) => section.shouldRender).map((section) => {
          const initialExpanded = section.defaultExpanded || section.forceExpandedByActive;
          const isExpanded = section.forceExpandedBySearch
            || (expandedSections[section.id] ?? initialExpanded);
          return (
            <section
              key={section.id}
              className={`conversation-section conversation-section--${section.id}`}
              role="group"
              aria-label={section.title}
            >
              <button
                type="button"
                className="conversation-section-header"
                onClick={() => toggleSection(section.id, isExpanded)}
                aria-expanded={isExpanded}
                aria-controls={`conversation-section-${section.id}`}
              >
                {isExpanded ? (
                  <ChevronDown size={15} aria-hidden="true" />
                ) : (
                  <ChevronRight size={15} aria-hidden="true" />
                )}
                <span className="conversation-section-title">{section.title}</span>
                <span className="conversation-section-count">{section.conversations.length}</span>
              </button>
              {isExpanded && (
                <div
                  id={`conversation-section-${section.id}`}
                  className="conversation-section-body"
                  role={section.conversations.length > 0 ? "list" : undefined}
                  aria-label={`${section.title} conversations`}
                >
                  {section.conversations.length === 0 ? (
                    <EmptyState
                      inline
                      className="conversation-section-empty"
                      description={
                        sidebarSections.hasSearchQuery
                          ? "No matching conversations"
                          : section.id === "direct"
                            ? "No direct messages"
                            : section.id === "group"
                              ? "No group conversations"
                              : "No archived conversations"
                      }
                    />
                  ) : (
                    section.conversations.map((item) => (
                      <ConversationListItem
                        key={item.id}
                        conv={item.conversation}
                        principal={principal}
                        agents={agents}
                        messages={messagesByConv[item.id] ?? []}
                        isActive={item.id === activeConvId}
                        isSelected={selectedConvIds.has(item.id)}
                        isPinned={item.isPinned}
                        isArchived={item.isArchived}
                        hidePending={hidePendingConversationIds.has(item.id)}
                        pinPending={pinPendingConversationIds.has(item.id)}
                        archivePending={archivePendingConversationIds.has(item.id)}
                        manageMode={manageMode}
                        unreadCount={unreads[item.id]?.unread ?? 0}
                        mentionCount={unreads[item.id]?.mentions ?? 0}
                        threadUnreadCount={unreads[item.id]?.threadUnread ?? 0}
                        hideEmptyPreview={item.isTerminalDirectMessage}
                        onSelect={() => onSelectConversation(item.id)}
                        onToggleSelection={() => toggleSelection(item.id)}
                        onTogglePin={(nextPinned) => onTogglePin(item.id, nextPinned)}
                        onToggleArchive={(nextArchived) => onToggleArchive(item.id, nextArchived)}
                        onHide={() => onHideSession(item.id)}
                      />
                    ))
                  )}
                </div>
              )}
            </section>
          );
        })}
        {hasMoreConversations && !sidebarSections.hasSearchQuery && (
          <button
            type="button"
            className="conversation-load-more"
            onClick={onLoadMoreConversations}
            disabled={loadingMoreConversations}
          >
            {loadingMoreConversations ? "Loading…" : "Load more conversations"}
          </button>
        )}
      </div>

      {showHiddenSessions && (
        <Modal
          title="Restore hidden sessions"
          description="Hidden sessions stay out of chats and search until you restore one."
          onClose={() => setShowHiddenSessions(false)}
          layout="flush"
          className="folder-picker-modal"
        >
          <div className="folder-picker-list" role="list" aria-label="Hidden sessions">
            {hiddenConversations.map((hidden) => {
              const conversation = conversations.find((candidate) => candidate.id === hidden.conversation_id);
              if (!conversation) return null;
              const name = conversationDisplayName(conversation, principal, agents);
              return (
                <div key={hidden.conversation_id} className="folder-picker-item" role="listitem">
                  <span className="folder-item-name">{name}</span>
                  <button
                    type="button"
                    className="company-show-hidden"
                    disabled={hidePendingConversationIds.has(hidden.conversation_id)}
                    onClick={() => onRestoreHiddenSession(hidden.conversation_id)}
                  >
                    Restore
                  </button>
                </div>
              );
            })}
          </div>
        </Modal>
      )}

      {manageMode && selectedConvIds.size > 0 && (
        <div
          className="sidebar-batch-bar"
        >
          <span className="sidebar-batch-count">
            {selectedConvIds.size} selected
            {selectedDeleteCount > 0 && ` (${selectedDeleteCount} agents)`}
          </span>
          <button
            onClick={handleBatchDelete}
            disabled={deleting || selectedDeleteCount === 0}
            title={`Delete ${selectedDeleteCount} conversation(s)`
            }
            className={`batch-delete-btn${selectedDeleteCount > 0 ? "" : " is-empty"}${deleting ? " is-loading" : ""}`}
          >
            {deleting ? "Deleting…" : `Delete (${selectedDeleteCount})`}
          </button>
        </div>
      )}
    </aside>
  );
}
