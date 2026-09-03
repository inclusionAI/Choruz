import { NextRequest, NextResponse } from "next/server";
import { requireAuth } from "../../../lib/api/api-auth";
import { instructionFileForDriver } from "../../../lib/agents/agent-provisioning";
import * as fs from "node:fs/promises";
import { constants as fsConstants } from "node:fs";
import * as path from "node:path";

function instructionFormat(content: string): "choruz" | "raw" {
  return content.includes("<!-- choruz-protocol:") ? "choruz" : "raw";
}

async function resolveAllowedWorkspace(filePath: string): Promise<string | null> {
  const home = process.env.HOME;
  if (!home) return null;
  try {
    const [resolved, resolvedHome, resolvedTmp] = await Promise.all([
      fs.realpath(filePath),
      fs.realpath(home),
      fs.realpath("/tmp"),
    ]);
    const isWithin = (candidate: string, root: string) =>
      candidate === root || candidate.startsWith(root + path.sep);
    return isWithin(resolved, resolvedHome) || isWithin(resolved, resolvedTmp) ? resolved : null;
  } catch {
    return null;
  }
}

function isMissingFile(error: unknown): boolean {
  return Boolean(error && typeof error === "object" && "code" in error && error.code === "ENOENT");
}

async function readInstructionFile(filePath: string): Promise<string> {
  const handle = await fs.open(filePath, fsConstants.O_RDONLY | fsConstants.O_NOFOLLOW);
  try {
    const stat = await handle.stat();
    if (!stat.isFile()) throw new Error("Instruction path is not a regular file");
    return await handle.readFile("utf-8");
  } finally {
    await handle.close();
  }
}

async function writeInstructionFile(
  filePath: string,
  content: string,
  exclusive = false,
): Promise<void> {
  const flags =
    fsConstants.O_WRONLY |
    fsConstants.O_CREAT |
    fsConstants.O_NOFOLLOW |
    (exclusive ? fsConstants.O_EXCL : fsConstants.O_TRUNC);
  const handle = await fs.open(filePath, flags, 0o600);
  try {
    const stat = await handle.stat();
    if (!stat.isFile()) throw new Error("Instruction path is not a regular file");
    await handle.writeFile(content, "utf-8");
  } finally {
    await handle.close();
  }
}

/** GET /api/agent-config?workspace_path=/path&driver_type=claude_terminal */
export async function GET(request: NextRequest) {
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;

  const { searchParams } = new URL(request.url);
  const workspacePath = searchParams.get("workspace_path");
  const driverType = searchParams.get("driver_type") || "";

  if (!workspacePath) {
    return NextResponse.json({ error: "workspace_path is required" }, { status: 400 });
  }
  const resolvedWorkspacePath = await resolveAllowedWorkspace(workspacePath);
  if (!resolvedWorkspacePath) {
    return NextResponse.json({ error: "Invalid workspace path" }, { status: 403 });
  }

  let primaryFilename: string;
  try {
    primaryFilename = instructionFileForDriver(driverType);
  } catch {
    return NextResponse.json({ error: "Unsupported driver_type" }, { status: 400 });
  }
  const filePath = path.join(resolvedWorkspacePath, primaryFilename);
  try {
    const content = await readInstructionFile(filePath);
    return NextResponse.json({
      filename: primaryFilename,
      content,
      path: filePath,
      format: instructionFormat(content),
    });
  } catch (error: unknown) {
    if (!isMissingFile(error)) {
      return NextResponse.json({ error: "Failed to read config file" }, { status: 500 });
    }
  }

  // An import must never move or rewrite an existing repository instruction
  // file. Read only the harness-native filename; saving creates that exact
  // filename only when the user asks it to.
  return NextResponse.json({
    filename: primaryFilename,
    content: "",
    path: filePath,
    exists: false,
    format: "raw",
  });
}

/** POST /api/agent-config { workspace_path, driver_type, content } */
export async function POST(request: NextRequest) {
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;

  let body: { workspace_path?: string; driver_type?: string; content?: string };
  try {
    body = await request.json();
  } catch {
    return NextResponse.json({ error: "Invalid JSON" }, { status: 400 });
  }

  const { workspace_path: workspacePath, driver_type: driverType = "", content } = body;

  if (!workspacePath) {
    return NextResponse.json({ error: "workspace_path is required" }, { status: 400 });
  }
  if (content === undefined || content === null) {
    return NextResponse.json({ error: "content is required" }, { status: 400 });
  }
  const resolvedWorkspacePath = await resolveAllowedWorkspace(workspacePath);
  if (!resolvedWorkspacePath) {
    return NextResponse.json({ error: "Invalid workspace path" }, { status: 403 });
  }

  let primaryFilename: string;
  try {
    primaryFilename = instructionFileForDriver(driverType);
  } catch {
    return NextResponse.json({ error: "Unsupported driver_type" }, { status: 400 });
  }
  const filePath = path.join(resolvedWorkspacePath, primaryFilename);
  try {
    await writeInstructionFile(filePath, content);
    return NextResponse.json({ ok: true, filename: primaryFilename, path: filePath });
  } catch {
    return NextResponse.json({ error: "Failed to write config file" }, { status: 500 });
  }
}
