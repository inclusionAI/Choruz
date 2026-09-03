import { NextRequest, NextResponse } from "next/server";
import { requireAuth } from "../../../lib/api/api-auth";
import * as fs from "node:fs/promises";
import * as path from "node:path";
import { serverPluginEnabled } from "../../../plugins/server-plugin";

/** Extract description from SKILL.md YAML frontmatter. */
function parseDescription(content: string): string | null {
  const match = content.match(/^---\s*\n([\s\S]*?)\n---/);
  if (!match) return null;
  const descMatch = match[1].match(/description:\s*(.+)/);
  return descMatch ? descMatch[1].trim() : null;
}

function validatePath(filePath: string): boolean {
  const home = process.env.HOME;
  if (!home) return false;
  const resolved = path.resolve(filePath);
  return resolved === home || resolved.startsWith(home + "/") || resolved.startsWith("/tmp/");
}

/**
 * GET /api/agent-skills?workspace_path=/path — list skills
 * GET /api/agent-skills?workspace_path=/path&read=<skill_name> — read a skill's SKILL.md content
 */
export async function GET(request: NextRequest) {
  if (!serverPluginEnabled("agent-skills")) {
    return NextResponse.json({ error: "plugin 'agent-skills' is disabled" }, { status: 404 });
  }
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;

  const { searchParams } = new URL(request.url);
  const workspacePath = searchParams.get("workspace_path");
  const readSkill = searchParams.get("read");

  if (!workspacePath) {
    return NextResponse.json({ error: "workspace_path is required" }, { status: 400 });
  }
  if (!validatePath(workspacePath)) {
    return NextResponse.json({ error: "Invalid workspace path" }, { status: 403 });
  }

  // ---- Read single skill content ----
  if (readSkill) {
    const safeName = path.basename(readSkill);
    // Try .claude/skills/<name>/SKILL.md
    const skillMdPath = path.join(workspacePath, ".claude", "skills", safeName, "SKILL.md");
    try {
      const content = await fs.readFile(skillMdPath, "utf-8");
      return NextResponse.json({ name: safeName, content });
    } catch {
      // Try .claude/commands/<name> (already has .md)
    }
    const cmdName = safeName.endsWith(".md") ? safeName : `${safeName}.md`;
    const cmdPath = path.join(workspacePath, ".claude", "commands", cmdName);
    try {
      const content = await fs.readFile(cmdPath, "utf-8");
      return NextResponse.json({ name: safeName, content });
    } catch {
      return NextResponse.json({ error: "Skill not found" }, { status: 404 });
    }
  }

  // ---- List all skills ----
  const commandsDir = path.join(workspacePath, ".claude", "commands");
  const skillsDir = path.join(workspacePath, ".claude", "skills");

  const skills: { name: string; path: string; type: "command" | "skill"; description: string | null }[] = [];

  // 1. Scan .claude/commands/ for flat .md files (slash commands)
  try {
    const entries = await fs.readdir(commandsDir, { withFileTypes: true });
    for (const e of entries) {
      if (e.isFile() && e.name.endsWith(".md")) {
        let description: string | null = null;
        try {
          const content = await fs.readFile(path.join(commandsDir, e.name), "utf-8");
          description = parseDescription(content);
        } catch { /* ignore */ }
        skills.push({
          name: e.name,
          path: path.join(commandsDir, e.name),
          type: "command",
          description,
        });
      }
    }
  } catch {
    // directory may not exist
  }

  // 2. Scan .claude/skills/ for skill directories (subdir with SKILL.md)
  try {
    const entries = await fs.readdir(skillsDir, { withFileTypes: true });
    for (const e of entries) {
      if (!e.isDirectory() || e.name.startsWith(".")) continue;
      const skillMd = path.join(skillsDir, e.name, "SKILL.md");
      try {
        const content = await fs.readFile(skillMd, "utf-8");
        const description = parseDescription(content);
        skills.push({
          name: e.name,
          path: path.join(skillsDir, e.name),
          type: "skill",
          description,
        });
      } catch {
        // no SKILL.md, skip
      }
    }
  } catch {
    // directory may not exist
  }

  return NextResponse.json({ skills });
}

/** POST /api/agent-skills — install a skill from a local Markdown file. */
export async function POST(request: NextRequest) {
  if (!serverPluginEnabled("agent-skills")) {
    return NextResponse.json({ error: "plugin 'agent-skills' is disabled" }, { status: 404 });
  }
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;

  let body: {
    workspace_path?: string;
    source: "local";
    local_path?: string;
  };
  try {
    body = await request.json();
  } catch {
    return NextResponse.json({ error: "Invalid JSON" }, { status: 400 });
  }

  const { workspace_path: workspacePath, source } = body;

  if (!workspacePath) {
    return NextResponse.json({ error: "workspace_path is required" }, { status: 400 });
  }
  if (!validatePath(workspacePath)) {
    return NextResponse.json({ error: "Invalid workspace path" }, { status: 403 });
  }
  if (source !== "local") {
    return NextResponse.json({ error: "source must be 'local'" }, { status: 400 });
  }

  const commandsDir = path.join(workspacePath, ".claude", "commands");
  await fs.mkdir(commandsDir, { recursive: true });

  const localPath = body.local_path;
  if (!localPath) {
    return NextResponse.json({ error: "local_path is required" }, { status: 400 });
  }
  if (!validatePath(localPath)) {
    return NextResponse.json({ error: "Invalid local path" }, { status: 403 });
  }
  const resolved = path.resolve(localPath);
  if (!resolved.endsWith(".md")) {
    return NextResponse.json({ error: "Only .md files are supported" }, { status: 400 });
  }
  try {
    const content = await fs.readFile(resolved, "utf-8");
    const filename = path.basename(resolved);
    await fs.writeFile(path.join(commandsDir, filename), content, "utf-8");
    return NextResponse.json({ ok: true, installed: filename });
  } catch {
    return NextResponse.json({ error: "Failed to read local file" }, { status: 400 });
  }
}

/** DELETE /api/agent-skills { workspace_path, skill_name } — delete a skill file */
export async function DELETE(request: NextRequest) {
  if (!serverPluginEnabled("agent-skills")) {
    return NextResponse.json({ error: "plugin 'agent-skills' is disabled" }, { status: 404 });
  }
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;

  let body: { workspace_path?: string; skill_name?: string };
  try {
    body = await request.json();
  } catch {
    return NextResponse.json({ error: "Invalid JSON" }, { status: 400 });
  }

  const { workspace_path: workspacePath, skill_name: skillName } = body;

  if (!workspacePath || !skillName) {
    return NextResponse.json({ error: "workspace_path and skill_name are required" }, { status: 400 });
  }
  if (!validatePath(workspacePath)) {
    return NextResponse.json({ error: "Invalid workspace path" }, { status: 403 });
  }

  const safeName = path.basename(skillName);

  // Try deleting as a skill directory (.claude/skills/<name>/)
  const skillDirPath = path.join(workspacePath, ".claude", "skills", safeName);
  try {
    const stat = await fs.stat(skillDirPath);
    if (stat.isDirectory()) {
      await fs.rm(skillDirPath, { recursive: true });
      return NextResponse.json({ ok: true, deleted: safeName });
    }
  } catch {
    // not a skill directory, try as command file
  }

  // Try deleting as a command file (.claude/commands/<name>.md)
  const cmdName = safeName.endsWith(".md") ? safeName : `${safeName}.md`;
  const filePath = path.join(workspacePath, ".claude", "commands", cmdName);
  try {
    await fs.unlink(filePath);
    return NextResponse.json({ ok: true, deleted: cmdName });
  } catch (err: unknown) {
    if (err && typeof err === "object" && "code" in err && (err as { code: string }).code === "ENOENT") {
      return NextResponse.json({ error: "Skill not found" }, { status: 404 });
    }
    return NextResponse.json({ error: "Failed to delete skill" }, { status: 500 });
  }
}
