import { NextRequest, NextResponse } from "next/server";
import { requireAuth } from "../../../lib/api/api-auth";
import { execFile } from "child_process";
import { promisify } from "util";
import * as os from "os";
import * as path from "path";
import { requirePathInsideWorkspace } from "../../../lib/workspace/workspace-path-guard";
import { serverPluginEnabled } from "../../../plugins/server-plugin";

const exec = promisify(execFile);

/**
 * GET /api/git-graph?repo_path=/path/to/repo&limit=200
 *
 * Returns git branch info + recent commits for the git-graph visualization.
 * Requires the repo path (typically the company folder_path or the Choruz repo root).
 */
export async function GET(request: NextRequest) {
  if (!serverPluginEnabled("workspace-git")) {
    return NextResponse.json({ error: "plugin 'workspace-git' is disabled" }, { status: 404 });
  }
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;
  const { token: sessionToken, claims } = auth;

  const { searchParams } = new URL(request.url);
  const repoPath = searchParams.get("repo_path");
  const limit = parseInt(searchParams.get("limit") || "200", 10);
  const workspaceId = searchParams.get("workspace_id") || claims.workspace_id;

  if (!repoPath) {
    return NextResponse.json({ error: "repo_path is required" }, { status: 400 });
  }

  const decision = await requirePathInsideWorkspace(sessionToken, workspaceId, repoPath);
  if (!decision.ok) {
    return NextResponse.json({ error: decision.error }, { status: decision.status });
  }

  // Sanity: resolve + confine to $HOME to prevent traversal into system dirs.
  const home = os.homedir();
  const resolved = path.resolve(repoPath);
  const homeReal = path.resolve(home);
  if (!resolved.startsWith(homeReal + path.sep) && resolved !== homeReal) {
    return NextResponse.json(
      { error: "repo_path must be under the user's home directory" },
      { status: 400 },
    );
  }

  try {
    // 1. Get all branches with their HEADs
    const { stdout: branchOut } = await exec(
      "git",
      ["branch", "-a", "--format=%(refname:short) %(objectname:short) %(committerdate:iso-strict)"],
      { cwd: resolved, maxBuffer: 1024 * 1024 },
    );

    // 2. Get main branch HEAD
    let mainBranch = "main";
    try {
      const { stdout: mainRef } = await exec(
        "git",
        ["symbolic-ref", "--short", "HEAD"],
        { cwd: resolved },
      );
      mainBranch = mainRef.trim() || "main";
    } catch {
      // default to main
    }

    // 3. Get recent commits on main
    const { stdout: mainLogOut } = await exec(
      "git",
      [
        "log",
        mainBranch,
        `--max-count=${limit}`,
        "--format=%H|%h|%s|%cI",
        "--",
      ],
      { cwd: resolved, maxBuffer: 1024 * 1024 },
    );

    const mainCommits = mainLogOut
      .trim()
      .split("\n")
      .filter(Boolean)
      .map((line) => {
        const [hash, shortHash, message, timestamp] = line.split("|");
        return { hash, shortHash, message, timestamp, filesChanged: 0 };
      });

    const mainHead = mainCommits[0]?.hash || "";

    // 4. Get choruz/* branches (agent branches)
    const choruzBranches = branchOut
      .trim()
      .split("\n")
      .filter((l) => l.startsWith("choruz/"))
      .map((line) => {
        const parts = line.trim().split(" ");
        return {
          name: parts[0],
          head: parts[1],
          lastTime: parts[2] || "",
        };
      });

    // 5. For each Choruz branch, get commits and ahead/behind
    const branches = await Promise.all(
      choruzBranches.map(async (b) => {
        // Ahead/behind
        let aheadMain = 0;
        let behindMain = 0;
        try {
          const { stdout: revList } = await exec(
            "git",
            ["rev-list", "--left-right", "--count", `${mainBranch}...${b.name}`],
            { cwd: resolved },
          );
          const [behind, ahead] = revList.trim().split("\t").map(Number);
          aheadMain = ahead || 0;
          behindMain = behind || 0;
        } catch {
          // branch may be gone
        }

        // Recent commits on this branch (not on main)
        let commits: { hash: string; shortHash: string; message: string; timestamp: string; filesChanged: number }[] = [];
        try {
          const { stdout: logOut } = await exec(
            "git",
            [
              "log",
              b.name,
              `--not`,
              mainBranch,
              "--max-count=20",
              "--format=%H|%h|%s|%cI",
            ],
            { cwd: resolved, maxBuffer: 512 * 1024 },
          );
          commits = logOut
            .trim()
            .split("\n")
            .filter(Boolean)
            .map((line) => {
              const [hash, shortHash, message, timestamp] = line.split("|");
              return { hash, shortHash, message, timestamp, filesChanged: 0 };
            });
        } catch {
          // ignore
        }

        // Get file change counts per commit (shortstat)
        for (const c of commits.slice(0, 10)) {
          try {
            const { stdout: stat } = await exec(
              "git",
              ["diff", "--shortstat", `${c.hash}^`, c.hash],
              { cwd: resolved },
            );
            const match = stat.match(/(\d+) file/);
            c.filesChanged = match ? parseInt(match[1], 10) : 0;
          } catch {
            // first commit or error
          }
        }

        // Extract agent name from branch: choruz/{agent}-{conv} → agent part
        const branchSuffix = b.name.replace("choruz/", "");
        const agentName = branchSuffix.replace(/-[a-f0-9]{8,}$/, "").replace(/-[a-z0-9]{4}$/, "");

        return {
          name: b.name,
          agentName: agentName || branchSuffix,
          agentColor: "", // frontend will assign based on agent name hash
          state: "idle" as const,
          head: b.head,
          aheadMain,
          behindMain,
          lastCommitTime: commits[0]?.timestamp || b.lastTime,
          lastCommitMessage: commits[0]?.message || "",
          commits,
        };
      }),
    );

    return NextResponse.json({
      branches: branches.filter((b) => b.commits.length > 0 || b.aheadMain > 0),
      mainCommits: mainCommits.slice(0, 50),
      mainHead,
    });
  } catch (err) {
    return NextResponse.json(
      { error: `Git error: ${err instanceof Error ? err.message : String(err)}` },
      { status: 500 },
    );
  }
}
