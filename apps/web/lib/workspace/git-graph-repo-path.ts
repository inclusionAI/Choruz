import type { RuntimeBindingInfo } from "../api/choruz-types";

export function selectGitGraphRepoPath(
  runtimeBindings: RuntimeBindingInfo[],
  workspaceId: string,
): string | null {
  const binding = runtimeBindings.find(
    (candidate) => candidate.workspace_id === workspaceId && candidate.workspace_path,
  );
  return binding?.workspace_path ?? null;
}
