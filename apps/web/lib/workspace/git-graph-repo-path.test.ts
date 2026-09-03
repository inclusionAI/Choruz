import { describe, expect, it } from "vitest";

import type { RuntimeBindingInfo } from "../api/choruz-types";
import { selectGitGraphRepoPath } from "./git-graph-repo-path";

function binding(workspaceId: string, workspacePath: string): RuntimeBindingInfo {
  return {
    id: `binding-${workspaceId}`,
    workspace_id: workspaceId,
    conversation_id: `conversation-${workspaceId}`,
    agent_principal_id: `agent-${workspaceId}`,
    driver_type: "codex_terminal",
    workspace_path: workspacePath,
    state: "idle",
  };
}

describe("selectGitGraphRepoPath", () => {
  it("selects a repo path from the active workspace instead of the first binding", () => {
    expect(
      selectGitGraphRepoPath(
        [
          binding("ws-other", "/work/other"),
          binding("ws-active", "/work/active"),
        ],
        "ws-active",
      ),
    ).toBe("/work/active");
  });

  it("returns null when the active workspace has no binding path", () => {
    expect(selectGitGraphRepoPath([binding("ws-other", "/work/other")], "ws-active")).toBeNull();
  });
});
