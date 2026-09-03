import { NextRequest, NextResponse } from "next/server";
import { promises as fs } from "node:fs";
import path from "node:path";
import { requireAuth } from "../../../../lib/api/api-auth";
import { serverPluginEnabled } from "../../../../plugins/server-plugin";

/**
 * POST /api/skills/scan
 * Scans a directory for skills. Each subdirectory containing a SKILL.md
 * is treated as a skill. Also picks up top-level .md files.
 *
 * Body: { path: string }
 * Returns: { skills: { name, dir, skill_md_path }[] }
 */
export async function POST(request: NextRequest) {
  if (!serverPluginEnabled("agent-skills")) {
    return NextResponse.json({ error: "plugin 'agent-skills' is disabled" }, { status: 404 });
  }
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;

  let body: { path: string };
  try {
    body = (await request.json()) as { path: string };
  } catch {
    return NextResponse.json({ error: "Invalid JSON" }, { status: 400 });
  }

  const dirPath = body.path?.trim();
  if (!dirPath) {
    return NextResponse.json({ error: "path is required" }, { status: 400 });
  }

  const resolved = path.resolve(dirPath);
  const home = process.env.HOME;
  if (!home || (resolved !== home && !resolved.startsWith(home + "/"))) {
    return NextResponse.json(
      { error: `Path must be under ${home}` },
      { status: 400 },
    );
  }

  try {
    const entries = await fs.readdir(resolved, { withFileTypes: true });
    const skills: { name: string; dir: string; skill_md_path: string }[] = [];

    for (const entry of entries) {
      if (entry.name.startsWith(".")) continue;

      if (entry.isDirectory()) {
        // Check for SKILL.md inside the subdirectory
        const skillMdPath = path.join(resolved, entry.name, "SKILL.md");
        try {
          await fs.access(skillMdPath);
          skills.push({
            name: entry.name,
            dir: path.join(resolved, entry.name),
            skill_md_path: skillMdPath,
          });
        } catch {
          // No SKILL.md, skip
        }
      } else if (entry.isFile() && entry.name.endsWith(".md")) {
        // Top-level .md files are also treated as skills
        skills.push({
          name: entry.name.replace(/\.md$/, ""),
          dir: resolved,
          skill_md_path: path.join(resolved, entry.name),
        });
      }
    }

    skills.sort((a, b) => a.name.localeCompare(b.name));

    return NextResponse.json({ skills });
  } catch (err) {
    return NextResponse.json(
      { error: `Cannot read directory: ${err instanceof Error ? err.message : err}` },
      { status: 400 },
    );
  }
}
