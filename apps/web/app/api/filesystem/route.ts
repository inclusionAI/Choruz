import { NextRequest, NextResponse } from "next/server";
import { apiBaseUrl } from "../../../lib/api/choruz-api";
import { requireAuth } from "../../../lib/api/api-auth";
import { requirePathInsideWorkspace } from "../../../lib/workspace/workspace-path-guard";

const API_BASE = apiBaseUrl();

export async function GET(request: NextRequest) {
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;
  const { token: sessionToken } = auth;

  const { searchParams } = new URL(request.url);
  const action = searchParams.get("action") || "list";
  const path = searchParams.get("path") || "";
  const showHidden = searchParams.get("show_hidden") || "false";
  const includeFiles = searchParams.get("include_files") || "false";
  const workspaceId = searchParams.get("workspace_id");

  if (action === "read" && !workspaceId) {
    return NextResponse.json({ error: "workspace_id is required" }, { status: 400 });
  }

  if (workspaceId && action !== "home") {
    const decision = await requirePathInsideWorkspace(sessionToken, workspaceId, path);
    if (!decision.ok) {
      return NextResponse.json({ error: decision.error }, { status: decision.status });
    }
  }

  let endpoint: string;
  if (action === "home") {
    endpoint = `${API_BASE}/v1/filesystem/home`;
  } else if (action === "stat") {
    endpoint = `${API_BASE}/v1/filesystem/stat?path=${encodeURIComponent(path)}`;
  } else if (action === "read") {
    endpoint = `${API_BASE}/v1/filesystem/read?path=${encodeURIComponent(path)}`;
  } else {
    endpoint = `${API_BASE}/v1/filesystem/list?path=${encodeURIComponent(path)}&show_hidden=${showHidden}&include_files=${includeFiles}`;
  }

  const res = await fetch(endpoint, {
    headers: { Authorization: `Bearer ${sessionToken}` },
    cache: "no-store",
  });

  let data: unknown;
  try {
    data = await res.json();
  } catch {
    data = { error: "Backend returned non-JSON response" };
  }

  return NextResponse.json(data, { status: res.status });
}

export async function POST(request: NextRequest) {
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;
  const { token: sessionToken } = auth;

  let body: unknown;
  try {
    body = await request.json();
  } catch {
    return NextResponse.json({ error: "Invalid JSON body" }, { status: 400 });
  }

  const payload = body as { path?: unknown; workspace_id?: unknown };
  if (typeof payload.path !== "string") {
    return NextResponse.json({ error: "path is required" }, { status: 400 });
  }
  if (typeof payload.workspace_id !== "string") {
    return NextResponse.json({ error: "workspace_id is required" }, { status: 400 });
  }
  const decision = await requirePathInsideWorkspace(sessionToken, payload.workspace_id, payload.path);
  if (!decision.ok) {
    return NextResponse.json({ error: decision.error }, { status: decision.status });
  }

  const res = await fetch(`${API_BASE}/v1/filesystem/write`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${sessionToken}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify(body),
    cache: "no-store",
  });

  let data: unknown;
  try {
    data = await res.json();
  } catch {
    data = { error: "Backend returned non-JSON response" };
  }

  return NextResponse.json(data, { status: res.status });
}
