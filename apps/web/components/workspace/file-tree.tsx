"use client";

import { useState, useCallback, useEffect, useRef, memo, type KeyboardEvent } from "react";
import { FolderPlus, RefreshCw } from "lucide-react";
import type { DirEntry } from "../../lib/api/choruz-types";
import { trace } from "../../lib/api/choruz-trace";
import { Spinner } from "../ui/spinner";
import { transportFetch } from "../../lib/api/transport";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface FileTreeProps {
  rootPath: string;
  workspaceId?: string;
  /** When true, the explorer section is initially collapsed */
  defaultCollapsed?: boolean;
  /** Override container style (e.g. maxHeight for drag-resize) */
  style?: React.CSSProperties;
  /** Called when a file node is clicked */
  onOpenFile?: (path: string) => void;
  /** Opens the owning Company's workspace picker. */
  onChangeRoot?: () => void;
}

interface TreeNodeData {
  name: string;
  path: string;
  type: "directory" | "file";
  children?: TreeNodeData[];
  loaded?: boolean;
}

// ---------------------------------------------------------------------------
// File icon helper
// ---------------------------------------------------------------------------

/** Lower-cased extension, used as `data-ext` so CSS can pick the icon colour. */
function fileExt(name: string): string {
  return name.split(".").pop()?.toLowerCase() || "";
}

/**
 * Roving focus: Tab enters the tree on the row focused last (the first row
 * before any), arrow keys walk the visible rows. Returns true when the key
 * was consumed.
 */
function moveTreeFocus(tree: HTMLElement, key: string): boolean {
  const rows = Array.from(tree.querySelectorAll<HTMLElement>(".file-tree-node"));
  if (rows.length === 0) return false;
  const current = rows.indexOf(document.activeElement as HTMLElement);
  let next: number;
  switch (key) {
    case "ArrowDown": next = Math.min(current + 1, rows.length - 1); break;
    case "ArrowUp": next = Math.max(current - 1, 0); break;
    case "Home": next = 0; break;
    case "End": next = rows.length - 1; break;
    default: return false;
  }
  rows[next].focus();
  return true;
}

/** Paths of the rows currently rendered, in document order. */
function visiblePaths(nodes: TreeNodeData[], expandedPaths: Set<string>, into: string[] = []): string[] {
  for (const node of nodes) {
    into.push(node.path);
    if (node.type === "directory" && expandedPaths.has(node.path) && node.children) {
      visiblePaths(node.children, expandedPaths, into);
    }
  }
  return into;
}

// ---------------------------------------------------------------------------
// TreeNode component (recursive)
// ---------------------------------------------------------------------------

const TreeNode = memo(function TreeNode({
  node,
  depth,
  tabStopPath,
  onToggle,
  expandedPaths,
  onOpenFile,
}: {
  node: TreeNodeData;
  depth: number;
  /** The one row that is in the tab order. */
  tabStopPath: string | null;
  onToggle: (path: string) => void;
  expandedPaths: Set<string>;
  onOpenFile?: (path: string) => void;
}) {
  const isDir = node.type === "directory";
  const isExpanded = expandedPaths.has(node.path);
  const paddingLeft = 12 + depth * 16;
  const activate = () => {
    if (isDir) onToggle(node.path);
    else onOpenFile?.(node.path);
  };
  const handleKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const expands = isDir && e.key === "ArrowRight" && !isExpanded;
    const collapses = isDir && e.key === "ArrowLeft" && isExpanded;
    if (e.key !== "Enter" && e.key !== " " && !expands && !collapses) return;
    e.preventDefault();
    activate();
  };

  return (
    <>
      <div
        className={`file-tree-node${isDir ? " is-dir" : " is-file"}`}
        style={{ paddingLeft }}
        onClick={activate}
        onKeyDown={handleKeyDown}
        tabIndex={node.path === tabStopPath ? 0 : -1}
        data-path={node.path}
        role="treeitem"
        aria-expanded={isDir ? isExpanded : undefined}
      >
        {isDir ? (
          <>
            <span className={`file-tree-chevron${isExpanded ? " expanded" : ""}`}>
              <svg width="10" height="10" viewBox="0 0 10 10" fill="currentColor">
                <path d="M3 2l4 3-4 3z" />
              </svg>
            </span>
            <span className="file-tree-icon file-tree-folder-icon">
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
                <path d="M2 4.5C2 3.67 2.67 3 3.5 3h2.59a1 1 0 01.7.29L8 4.5h4.5c.83 0 1.5.67 1.5 1.5v5.5c0 .83-.67 1.5-1.5 1.5h-9A1.5 1.5 0 012 11.5V4.5z"/>
              </svg>
            </span>
          </>
        ) : (
          <>
            <span className="file-tree-chevron-placeholder" />
            <span className="file-tree-icon" data-ext={fileExt(node.name)}>
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
                <path d="M4 2h5l4 4v7a1 1 0 01-1 1H4a1 1 0 01-1-1V3a1 1 0 011-1z"/>
                <path d="M9 2v4h4"/>
              </svg>
            </span>
          </>
        )}
        <span className="file-tree-name">{node.name}</span>
      </div>
      {isDir && isExpanded && node.children && (
        <div className="file-tree-children" role="group">
          {node.children.length === 0 && node.loaded && (
            <div
              className="file-tree-empty"
              style={{ paddingLeft: paddingLeft + 16 }}
            >
              (empty)
            </div>
          )}
          {node.children.map((child) => (
            <TreeNode
              key={child.path}
              node={child}
              depth={depth + 1}
              tabStopPath={tabStopPath}
              onToggle={onToggle}
              expandedPaths={expandedPaths}
              onOpenFile={onOpenFile}
            />
          ))}
        </div>
      )}
    </>
  );
});

// ---------------------------------------------------------------------------
// FileTree main component
// ---------------------------------------------------------------------------

export function FileTree({ rootPath, workspaceId, defaultCollapsed = false, style, onOpenFile, onChangeRoot }: FileTreeProps) {
  const [collapsed, setCollapsed] = useState(defaultCollapsed);
  const [tree, setTree] = useState<TreeNodeData[]>([]);
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(new Set());
  const [focusedPath, setFocusedPath] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const loadedPathsRef = useRef<Set<string>>(new Set());

  // Get the display name (last segment of the root path)
  const rootName = rootPath.split("/").filter(Boolean).pop() || rootPath;

  // Load root directory contents
  const fetchDirectory = useCallback(
    async (dirPath: string): Promise<TreeNodeData[]> => {
      try {
        const params = new URLSearchParams({
          action: "list",
          path: dirPath,
          include_files: "true",
        });
        if (workspaceId) params.set("workspace_id", workspaceId);
        const res = await transportFetch(`/api/filesystem?${params.toString()}`);
        if (!res.ok) return [];
        const data = (await res.json()) as { entries?: DirEntry[] };
        const entries = data.entries || [];

        // Sort: directories first, then files, both alphabetical
        entries.sort((a, b) => {
          if (a.type === "directory" && b.type !== "directory") return -1;
          if (a.type !== "directory" && b.type === "directory") return 1;
          return a.name.toLowerCase().localeCompare(b.name.toLowerCase());
        });

        return entries.map((e) => ({
          name: e.name,
          path: e.path,
          type: e.type as "directory" | "file",
          loaded: e.type !== "directory",
        }));
      } catch {
        return [];
      }
    },
    [workspaceId],
  );

  // Initial load
  useEffect(() => {
    if (!rootPath) return;
    let cancelled = false;
    setLoading(true);
    setError(null);
    fetchDirectory(rootPath)
      .then((nodes) => {
        if (cancelled) return;
        setTree(nodes);
        loadedPathsRef.current.add(rootPath);
      })
      .catch(() => {
        if (!cancelled) setError("Failed to load files");
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [rootPath, fetchDirectory]);

  // Toggle expand/collapse of a directory
  const handleToggle = useCallback(
    async (path: string) => {
      setExpandedPaths((prev) => {
        const next = new Set(prev);
        if (next.has(path)) {
          next.delete(path);
        } else {
          next.add(path);
        }
        return next;
      });

      // Lazy load: if not yet loaded, fetch children
      if (!loadedPathsRef.current.has(path)) {
        const children = await fetchDirectory(path);
        loadedPathsRef.current.add(path);

        setTree((prev) => {
          const updateChildren = (nodes: TreeNodeData[]): TreeNodeData[] =>
            nodes.map((n) => {
              if (n.path === path) {
                return { ...n, children, loaded: true };
              }
              if (n.children) {
                return { ...n, children: updateChildren(n.children) };
              }
              return n;
            });
          return updateChildren(prev);
        });
      }
    },
    [fetchDirectory],
  );

  // Refresh the entire tree
  const handleRefresh = useCallback(async () => {
    trace.event("refresh_files", { rootPath });
    setLoading(true);
    loadedPathsRef.current.clear();
    loadedPathsRef.current.add(rootPath);

    const nodes = await fetchDirectory(rootPath);
    setTree(nodes);

    // Re-load all expanded paths
    const expanded = new Set(expandedPaths);
    for (const p of expanded) {
      const children = await fetchDirectory(p);
      loadedPathsRef.current.add(p);
      // Update tree with the loaded children
      setTree((prev) => {
        const updateChildren = (ns: TreeNodeData[]): TreeNodeData[] =>
          ns.map((n) => {
            if (n.path === p) {
              return { ...n, children, loaded: true };
            }
            if (n.children) {
              return { ...n, children: updateChildren(n.children) };
            }
            return n;
          });
        return updateChildren(prev);
      });
    }

    setLoading(false);
  }, [rootPath, expandedPaths, fetchDirectory]);

  // The tab stop follows the last focused row; if that row was hidden by a
  // collapse (or never set), it falls back to the first row.
  const visible = visiblePaths(tree, expandedPaths);
  const tabStopPath = focusedPath && visible.includes(focusedPath) ? focusedPath : visible[0] ?? null;

  return (
    <div className="file-tree-container" style={style}>
      {/* Section header: the collapse toggle plus sibling action buttons */}
      <div className="file-tree-section-header">
        <button
          type="button"
          className="file-tree-section-toggle"
          aria-expanded={!collapsed}
          onClick={() => setCollapsed((c) => !c)}
        >
          <span className={`file-tree-section-chevron${collapsed ? "" : " expanded"}`}>
            <svg width="10" height="10" viewBox="0 0 10 10" fill="currentColor">
              <path d="M3 2l4 3-4 3z" />
            </svg>
          </span>
          <span className="file-tree-section-title">EXPLORER</span>
          <span className="file-tree-section-subtitle" title={rootPath}>
            {rootName}
          </span>
        </button>
        {!collapsed && (
          <span className="file-tree-actions">
            {onChangeRoot && (
              <button
                type="button"
                className="file-tree-refresh-btn"
                onClick={onChangeRoot}
                title="Change workspace folder"
                aria-label="Change workspace folder"
              >
                <FolderPlus size={14} aria-hidden="true" />
              </button>
            )}
            <button
              type="button"
              className="file-tree-refresh-btn"
              data-trace="refresh_files"
              onClick={handleRefresh}
              title="Refresh file tree"
              aria-label="Refresh file tree"
            >
              <RefreshCw size={14} aria-hidden="true" />
            </button>
          </span>
        )}
      </div>

      {/* Tree contents */}
      {!collapsed && (
        <div
          className="file-tree-content"
          role="tree"
          aria-label="File explorer"
          onKeyDown={(e) => {
            if (moveTreeFocus(e.currentTarget, e.key)) e.preventDefault();
          }}
          onFocus={(e) => {
            const path = (e.target as HTMLElement).closest<HTMLElement>(".file-tree-node")?.dataset.path;
            if (path) setFocusedPath(path);
          }}
        >
          {loading && tree.length === 0 && (
            <div className="file-tree-loading"><Spinner label="Loading…" /></div>
          )}
          {error && (
            <div className="file-tree-error">{error}</div>
          )}
          {tree.map((node) => (
            <TreeNode
              key={node.path}
              node={node}
              depth={0}
              tabStopPath={tabStopPath}
              onToggle={handleToggle}
              expandedPaths={expandedPaths}
              onOpenFile={onOpenFile}
            />
          ))}
        </div>
      )}
    </div>
  );
}
