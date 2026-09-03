"use client";

import { useState, useMemo, useCallback, type CSSProperties } from "react";
import { ChevronDown } from "lucide-react";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type Commit = {
  hash: string;
  shortHash: string;
  message: string;
  timestamp: string;
  filesChanged: number;
};

export type BranchInfo = {
  name: string;
  agentName: string;
  agentColor: string;
  state: "running" | "idle" | "disabled" | "merged";
  head: string;
  aheadMain: number;
  behindMain: number;
  lastCommitTime: string;
  lastCommitMessage: string;
  commits: Commit[];
};

export type GitGraphProps = {
  branches: BranchInfo[];
  mainCommits: Commit[];
  mainHead: string;
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function relTime(iso: string): string {
  const s = Math.round((Date.now() - new Date(iso).getTime()) / 1000);
  if (s < 60) return `${s}s ago`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h ago`;
  const d = Math.round(h / 24);
  if (d < 7) return `${d}d ago`;
  return `${Math.round(d / 7)}w ago`;
}

// ---------------------------------------------------------------------------
// Tooltip
// ---------------------------------------------------------------------------

type TipData = { x: number; y: number; node: React.ReactNode } | null;

function Tooltip({ data }: { data: TipData }) {
  if (!data) return null;
  return (
    <div className="git-graph-tooltip" style={{ left: data.x + 12, top: data.y + 12 }}>
      {data.node}
    </div>
  );
}

// ---------------------------------------------------------------------------
// MiniDAG — two-lane SVG (main + agent branch)
// ---------------------------------------------------------------------------

type LaneCommit = Commit & { lane: "main" | "agent" };
type GraphLine =
  | { kind: "segment"; x1: number; y1: number; x2: number; y2: number; color: string }
  | { kind: "curve"; path: string; color: string };

const LANE_M = 24; // main lane x
const LANE_A = 64; // agent lane x
const STEP = 26;
const R = 4;
const PAD = 14;

function MiniDAG({
  branch,
  mainCommits,
  setTip,
}: {
  branch: BranchInfo;
  mainCommits: Commit[];
  setTip: (t: TipData) => void;
}) {
  const graph = useMemo(() => {
    // Merge & sort all commits by time (oldest first = bottom)
    const mainSet = new Set(mainCommits.map((c) => c.hash));
    const all: LaneCommit[] = [
      ...mainCommits.map((c) => ({ ...c, lane: "main" as const })),
      ...branch.commits
        .filter((c) => !mainSet.has(c.hash))
        .map((c) => ({ ...c, lane: "agent" as const })),
    ].sort((a, b) => new Date(a.timestamp).getTime() - new Date(b.timestamp).getTime());

    // newest on top → reverse
    all.reverse();

    const nodes = all.map((c, i) => ({
      ...c,
      x: c.lane === "main" ? LANE_M : LANE_A,
      y: PAD + i * STEP,
    }));

    // Lines within each lane
    const lines: GraphLine[] = [];
    let lastMain: { x: number; y: number } | null = null;
    let lastAgent: { x: number; y: number } | null = null;

    // walk top→bottom (newest→oldest)
    for (const n of nodes) {
      if (n.lane === "main") {
        if (lastMain) lines.push({ kind: "segment", x1: lastMain.x, y1: lastMain.y, x2: n.x, y2: n.y, color: "var(--graph-main)" });
        lastMain = { x: n.x, y: n.y };
      } else {
        if (lastAgent) lines.push({ kind: "segment", x1: lastAgent.x, y1: lastAgent.y, x2: n.x, y2: n.y, color: branch.agentColor });
        lastAgent = { x: n.x, y: n.y };
      }
    }

    // Fork curve: from last agent node (oldest agent commit) back to nearest main node below it
    const agentNodes = nodes.filter((n) => n.lane === "agent");
    const oldestAgent = agentNodes[agentNodes.length - 1];
    if (oldestAgent) {
      const forkTarget = nodes.find((n) => n.lane === "main" && n.y >= oldestAgent.y);
      if (forkTarget) {
        const path = `M ${forkTarget.x} ${forkTarget.y} C ${forkTarget.x} ${forkTarget.y - STEP * 0.4}, ${oldestAgent.x} ${oldestAgent.y + STEP * 0.4}, ${oldestAgent.x} ${oldestAgent.y}`;
        lines.push({ kind: "curve", path, color: branch.agentColor });
      }
    }

    // Merge curve: if state is merged, newest agent → nearest main above
    if (branch.state === "merged" && agentNodes.length > 0) {
      const newestAgent = agentNodes[0];
      const mergeTarget = nodes.find((n) => n.lane === "main" && n.y <= newestAgent.y);
      if (mergeTarget) {
        const path = `M ${newestAgent.x} ${newestAgent.y} C ${newestAgent.x} ${newestAgent.y - STEP * 0.4}, ${mergeTarget.x} ${mergeTarget.y + STEP * 0.4}, ${mergeTarget.x} ${mergeTarget.y}`;
        lines.push({ kind: "curve", path, color: branch.agentColor });
      }
    }

    return { nodes, lines, h: PAD + nodes.length * STEP + PAD };
  }, [branch, mainCommits]);

  const tipFor = (c: LaneCommit, e: React.MouseEvent) => {
    setTip({
      x: e.clientX,
      y: e.clientY,
      node: (
        <>
          <div className="git-graph-tip-title">{c.message}</div>
          <div className="git-graph-tip-meta">
            {c.shortHash} &middot; {c.filesChanged} files &middot; {relTime(c.timestamp)}
          </div>
        </>
      ),
    });
  };

  return (
    <div className="git-graph-dag">
      {/* Lane labels */}
      <div className="git-graph-lanes">
        <span>
          <span className="git-graph-lane-dot is-main">{"●"}</span> main
        </span>
        <span>
          <span className="git-graph-lane-dot">{"●"}</span> {branch.agentName}
        </span>
      </div>
      <svg className="git-graph-svg" width="100%" height={graph.h}>
        {graph.lines.map((l, i) =>
          l.kind === "curve" ? (
            <path key={i} d={l.path} stroke={l.color} strokeWidth={2} fill="none" opacity={0.6} />
          ) : (
            <line key={i} x1={l.x1} y1={l.y1} x2={l.x2} y2={l.y2} stroke={l.color} strokeWidth={2} opacity={0.5} />
          ),
        )}
        {graph.nodes.map((n) => (
          <circle
            key={n.hash}
            className="git-graph-node"
            cx={n.x}
            cy={n.y}
            r={R}
            fill={n.lane === "main" ? "var(--graph-main)" : branch.agentColor}
            stroke="var(--bg-surface)"
            strokeWidth={2}
            onMouseOver={(e) => tipFor(n, e)}
            onMouseOut={() => setTip(null)}
          />
        ))}
        {/* HEAD labels */}
        {graph.nodes.length > 0 && graph.nodes[0].lane === "main" && (
          <text x={LANE_M + 10} y={graph.nodes[0].y + 4} fontSize={9} fill="var(--graph-main)" fontWeight={600}>HEAD</text>
        )}
      </svg>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Branch row
// ---------------------------------------------------------------------------

function BranchRow({
  branch,
  expanded,
  onToggle,
  mainCommits,
  setTip,
}: {
  branch: BranchInfo;
  expanded: boolean;
  onToggle: () => void;
  mainCommits: Commit[];
  setTip: (t: TipData) => void;
}) {
  const sparkDots = branch.commits.slice(0, 8);
  const even = branch.aheadMain === 0 && branch.behindMain === 0;

  return (
    <div
      className="git-graph-branch"
      style={{ "--agent-color": branch.agentColor } as CSSProperties}
    >
      {/* Overview row: the disclosure control for the detail below */}
      <button
        type="button"
        className="git-graph-branch-row"
        aria-expanded={expanded}
        onClick={onToggle}
      >
        {/* State + color dots */}
        <div className="git-graph-dots">
          <span className={`git-graph-state-dot is-${branch.state}`} />
          <span className="git-graph-agent-dot" />
        </div>

        {/* Name + message + sparkline */}
        <div className="git-graph-branch-main">
          <div className="git-graph-branch-name">{branch.agentName}</div>
          <div className="git-graph-branch-msg">{branch.lastCommitMessage}</div>
          <div className="git-graph-spark">
            {sparkDots.map((c, i) => (
              <span
                key={c.hash}
                className="git-graph-spark-dot"
                style={{ opacity: 1 - i * 0.1 }}
              />
            ))}
          </div>
        </div>

        {/* Ahead/behind + time */}
        <div className="git-graph-branch-meta">
          <div className="git-graph-delta">
            {branch.aheadMain > 0 && <span className="is-ahead">+{branch.aheadMain}</span>}
            {branch.behindMain > 0 && <span className="is-behind">-{branch.behindMain}</span>}
            {even && <span className="is-even">even</span>}
          </div>
          <div className="git-graph-time">{relTime(branch.lastCommitTime)}</div>
        </div>

        <ChevronDown size={14} className={`disclosure-caret${expanded ? " is-open" : ""}`} aria-hidden="true" />
      </button>

      {/* Expanded detail */}
      {expanded && (
        <div className="git-graph-branch-detail">
          <MiniDAG branch={branch} mainCommits={mainCommits} setTip={setTip} />
          <div className="git-graph-commits">
            {branch.commits.slice(0, 10).map((c) => (
              <div key={c.hash} className="git-graph-commit">
                <span className="git-graph-commit-hash">
                  {c.shortHash}
                </span>
                <span className="git-graph-commit-msg">{c.message}</span>
                <span className="git-graph-commit-files">{c.filesChanged}f</span>
                <span className="git-graph-commit-time">{relTime(c.timestamp)}</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// GitGraph — main export
// ---------------------------------------------------------------------------

export function GitGraph({ branches, mainCommits }: GitGraphProps) {
  const [expandedBranch, setExpandedBranch] = useState<string | null>(null);
  const [tip, setTip] = useState<TipData>(null);

  const sorted = useMemo(
    () => [...branches].sort((a, b) => new Date(b.lastCommitTime).getTime() - new Date(a.lastCommitTime).getTime()),
    [branches],
  );

  const toggle = useCallback(
    (name: string) => setExpandedBranch((prev) => (prev === name ? null : name)),
    [],
  );

  if (branches.length === 0) {
    return <div className="git-graph-empty">No agent branches</div>;
  }

  return (
    <div>
      <div className="git-graph-summary">
        <span>{branches.length} branches</span>
        <span>{branches.filter((b) => b.state === "running").length} active</span>
        <span>{branches.filter((b) => b.state === "merged").length} merged</span>
      </div>

      <div className="git-graph-list">
        {sorted.map((b) => (
          <BranchRow
            key={b.name}
            branch={b}
            expanded={expandedBranch === b.name}
            onToggle={() => toggle(b.name)}
            mainCommits={mainCommits}
            setTip={setTip}
          />
        ))}
      </div>

      <Tooltip data={tip} />
    </div>
  );
}
