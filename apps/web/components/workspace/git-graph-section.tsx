"use client";

import { useCallback, useMemo, useState } from "react";
import { RefreshCw } from "lucide-react";
import { Spinner } from "../ui/spinner";
import type { RuntimeBindingInfo } from "../../lib/api/choruz-types";
import { GitGraph, type GitGraphProps } from "./git-graph";
import { avatarColor } from "../../lib/avatar";
import { selectGitGraphRepoPath } from "../../lib/workspace/git-graph-repo-path";
import { transportFetch } from "../../lib/api/transport";

export function GitGraphSection({
  runtimeBindings,
  workspaceId,
}: {
  runtimeBindings: RuntimeBindingInfo[];
  workspaceId: string;
}) {
  const [graphData, setGraphData] = useState<GitGraphProps | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [show, setShow] = useState(false);

  // Derive repo path from first binding's workspace_path (go up to repo root)
  const repoPath = useMemo(() => {
    return selectGitGraphRepoPath(runtimeBindings, workspaceId);
  }, [runtimeBindings, workspaceId]);

  const fetchGraph = useCallback(async () => {
    if (!repoPath) return;
    setLoading(true);
    setError(null);
    try {
      const mainRepo = repoPath.replace(/\/.runtime\/workspaces\/[^/]+\/workspace$/, "");
      const params = new URLSearchParams({
        repo_path: mainRepo,
        limit: "100",
        workspace_id: workspaceId,
      });
      const res = await transportFetch(`/api/git-graph?${params.toString()}`);
      if (!res.ok) throw new Error(`${res.status}`);
      const data = await res.json();

      // Enrich with agent colors and state from runtime bindings
      for (const b of data.branches) {
        b.agentColor = avatarColor(b.agentName);

        const binding = runtimeBindings.find(
          (rb) => b.name.includes(rb.agent_principal_id?.slice(0, 8) || "____"),
        );
        if (binding) {
          b.state = binding.state === "running" ? "running" : binding.state === "disabled" ? "disabled" : "idle";
        }
      }

      setGraphData(data);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }, [repoPath, runtimeBindings, workspaceId]);

  return (
    <div className="detail-section">
      <div className="detail-section-header">
        <h4>Git Graph</h4>
        <button
          type="button"
          className="detail-section-action"
          aria-expanded={show}
          onClick={() => {
            if (!show && !graphData) fetchGraph();
            setShow(!show);
          }}
        >
          {show ? "Hide" : "Show"}
        </button>
      </div>
      {show && (
        <>
          {loading && <div className="detail-inline-empty"><Spinner label="Loading git data…" /></div>}
          {error && <div className="detail-inline-empty is-error">Error: {error}</div>}
          {graphData && (
            <>
              <button type="button" className="detail-section-action git-graph-refresh" onClick={fetchGraph}>
                <RefreshCw size={12} aria-hidden="true" /> Refresh
              </button>
              <GitGraph {...graphData} />
            </>
          )}
          {!loading && !error && !graphData && !repoPath && (
            <div className="detail-inline-empty">No workspace path available</div>
          )}
        </>
      )}
    </div>
  );
}
