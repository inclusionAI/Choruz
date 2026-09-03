import { promises as fs } from "fs";
import * as path from "path";

import { apiBaseUrl, type Company } from "../api/choruz-api";

type FetchLike = typeof fetch;

type WorkspacePathDecision =
  | { ok: true }
  | { ok: false; status: number; error: string };

type WorkspaceRootsDecision =
  | { ok: true; roots: string[] }
  | { ok: false; status: number; error: string };

async function policyPath(targetPath: string): Promise<string> {
  try {
    return await fs.realpath(targetPath);
  } catch {
    const parent = path.dirname(targetPath);
    const basename = path.basename(targetPath);
    try {
      return path.join(await fs.realpath(parent), basename);
    } catch {
      return path.resolve(targetPath);
    }
  }
}

export function isPathInsideRoot(candidatePath: string, rootPath: string): boolean {
  const candidate = path.resolve(candidatePath);
  const root = path.resolve(rootPath);
  const relative = path.relative(root, candidate);
  return relative === "" || (!!relative && !relative.startsWith("..") && !path.isAbsolute(relative));
}

export function workspaceRoots(
  companies: Pick<Company, "id" | "folder_path" | "deleted_at">[],
  workspaceId: string,
): string[] {
  return companies
    .filter((company) => company.id === workspaceId && !company.deleted_at)
    .map((company) => company.folder_path?.trim() ?? "")
    .filter(Boolean);
}

export async function loadWorkspaceRoots(
  token: string,
  workspaceId: string,
  fetchImpl: FetchLike = fetch,
): Promise<WorkspaceRootsDecision> {
  let response: Response;
  try {
    response = await fetchImpl(`${apiBaseUrl()}/v1/companies`, {
      headers: { Authorization: `Bearer ${token}` },
      cache: "no-store",
    });
  } catch {
    return { ok: false, status: 503, error: "Unable to verify workspace path access" };
  }

  if (!response.ok) {
    return { ok: false, status: response.status, error: "Unable to verify workspace path access" };
  }

  const companies = (await response.json()) as Pick<Company, "id" | "folder_path" | "deleted_at">[];
  return { ok: true, roots: workspaceRoots(companies, workspaceId) };
}

export async function requirePathInsideWorkspace(
  token: string,
  workspaceId: string,
  targetPath: string,
  fetchImpl: FetchLike = fetch,
): Promise<WorkspacePathDecision> {
  const rootResult = await loadWorkspaceRoots(token, workspaceId, fetchImpl);
  if (!rootResult.ok) return rootResult;

  if (rootResult.roots.length === 0) {
    return { ok: false, status: 403, error: "Workspace has no configured folder path" };
  }

  const [candidate, roots] = await Promise.all([
    policyPath(targetPath),
    Promise.all(rootResult.roots.map(policyPath)),
  ]);

  if (roots.some((root) => isPathInsideRoot(candidate, root))) {
    return { ok: true };
  }

  return { ok: false, status: 403, error: "Path is outside the requested workspace" };
}
