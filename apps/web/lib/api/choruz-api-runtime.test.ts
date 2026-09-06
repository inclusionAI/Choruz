import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  fetchConversationRuntimeStatus,
  fetchRuntimeBinding,
  fetchRuntimeBindings,
  executeRuntimeHostOperation,
  importWorkspaceSessions,
  onboardRuntimeHost,
  rebindRuntimeBinding,
} from "./choruz-api";

describe("runtime binding API helpers", () => {
  let previousBaseUrl: string | undefined;
  let previousApiUrl: string | undefined;
  let previousApiPort: string | undefined;

  beforeEach(() => {
    previousBaseUrl = process.env.CHORUZ_API_BASE_URL;
    previousApiUrl = process.env.CHORUZ_API_URL;
    previousApiPort = process.env.CHORUZ_API_PORT;
    vi.stubGlobal("window", undefined);
    delete process.env.CHORUZ_API_BASE_URL;
    delete process.env.CHORUZ_API_URL;
    delete process.env.CHORUZ_API_PORT;
  });

  afterEach(() => {
    if (previousBaseUrl === undefined) delete process.env.CHORUZ_API_BASE_URL;
    else process.env.CHORUZ_API_BASE_URL = previousBaseUrl;
    if (previousApiUrl === undefined) delete process.env.CHORUZ_API_URL;
    else process.env.CHORUZ_API_URL = previousApiUrl;
    if (previousApiPort === undefined) delete process.env.CHORUZ_API_PORT;
    else process.env.CHORUZ_API_PORT = previousApiPort;
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("loads the runtime binding list with the session token", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => [],
    });
    vi.stubGlobal("fetch", fetchMock);

    await fetchRuntimeBindings("session-token");

    expect(fetchMock).toHaveBeenCalledWith(
      "http://127.0.0.1:3000/v1/runtime/bindings",
      expect.objectContaining({
        cache: "no-store",
        headers: expect.any(Headers),
      }),
    );
    const [, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(new Headers(init.headers).get("authorization")).toBe("Bearer session-token");
  });

  it("loads conversation runtime status with auth and no-store", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => [],
    });
    vi.stubGlobal("fetch", fetchMock);

    await fetchConversationRuntimeStatus("session-token", "conversation 1");

    expect(fetchMock).toHaveBeenCalledWith(
      "http://127.0.0.1:3000/v1/conversations/conversation%201/runtime-status",
      expect.objectContaining({
        cache: "no-store",
        headers: expect.any(Headers),
      }),
    );
    const [, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(new Headers(init.headers).get("authorization")).toBe("Bearer session-token");
  });

  it("targets the correct runtime binding endpoints for detail and actions", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ id: "binding-1" }),
    });
    vi.stubGlobal("fetch", fetchMock);

    await fetchRuntimeBinding("session-token", "binding-1");
    await rebindRuntimeBinding("session-token", "binding-1", "/worktrees/next");

    expect(fetchMock.mock.calls[0]?.[0]).toBe(
      "http://127.0.0.1:3000/v1/runtime/bindings/binding-1",
    );
    expect(fetchMock.mock.calls[1]?.[0]).toBe(
      "http://127.0.0.1:3000/v1/runtime/bindings/binding-1/rebind",
    );

    const rebindInit = fetchMock.mock.calls[1]?.[1] as RequestInit;
    expect(rebindInit.method).toBe("POST");
    expect(rebindInit.body).toBe(JSON.stringify({ workspace_path: "/worktrees/next" }));
  });

  it("preserves each native session workspace in recursive imports", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ imported: [] }),
    });
    vi.stubGlobal("fetch", fetchMock);

    await importWorkspaceSessions("session-token", "company-1", "/projects", [
      {
        harness: "claude",
        native_session_id: "claude-1",
        workspace_path: "/projects/frontend",
      },
      {
        harness: "codex",
        native_session_id: "codex-1",
        workspace_path: "/projects/backend",
      },
      {
        harness: "pi",
        native_session_id: "pi-1",
        workspace_path: "/projects/research",
      },
      {
        harness: "grok",
        native_session_id: "grok-1",
        workspace_path: "/projects/infra",
      },
      {
        harness: "open_code",
        native_session_id: "opencode-1",
        workspace_path: "/projects/tools",
      },
    ]);

    const [, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(JSON.parse(init.body as string)).toEqual({
      company_id: "company-1",
      workspace_path: "/projects",
      sessions: [
        { harness: "claude", native_session_id: "claude-1", workspace_path: "/projects/frontend" },
        { harness: "codex", native_session_id: "codex-1", workspace_path: "/projects/backend" },
        { harness: "pi", native_session_id: "pi-1", workspace_path: "/projects/research" },
        { harness: "grok", native_session_id: "grok-1", workspace_path: "/projects/infra" },
        { harness: "open_code", native_session_id: "opencode-1", workspace_path: "/projects/tools" },
      ],
    });
  });

  it("targets a selected device for remote filesystem work and session imports", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ entries: [], imported: [] }),
    });
    vi.stubGlobal("fetch", fetchMock);

    await executeRuntimeHostOperation(
      "session-token",
      "host west",
      "filesystem.list",
      { path: "/srv/projects", include_files: false },
    );
    await importWorkspaceSessions(
      "session-token",
      "company-1",
      "/srv/projects",
      [{
        harness: "claude",
        native_session_id: "session-1",
        workspace_path: "/srv/projects/app",
      }],
      "host west",
    );

    expect(fetchMock.mock.calls[0]?.[0]).toBe(
      "http://127.0.0.1:3000/v1/runtime-hosts/host%20west/operations",
    );
    expect(JSON.parse((fetchMock.mock.calls[0]?.[1] as RequestInit).body as string)).toEqual({
      kind: "filesystem.list",
      request: { path: "/srv/projects", include_files: false },
    });
    expect(JSON.parse((fetchMock.mock.calls[1]?.[1] as RequestInit).body as string)).toMatchObject({
      company_id: "company-1",
      runtime_host_id: "host west",
    });
  });

  it("onboards a runtime host only through the encrypted transport body", async () => {
    const relayFetch = vi.fn().mockResolvedValue(new Response(JSON.stringify({
      host_id: "host-remote",
      host_name: "GPU server",
    }), { status: 200, headers: { "content-type": "application/json" } }));
    const payload = {
      controller_gateway_url: "https://gateway.example",
      controller_credential: "v1.AAAAAAAAAAAAAAAAAAAAAA.BBBBBBBBBBBBBBBBBBBBBB",
      runtime_host_code: "12345678",
      name: "GPU server",
    };

    await expect(onboardRuntimeHost({ fetch: relayFetch, socket: vi.fn() }, payload)).resolves.toEqual({
      host_id: "host-remote",
      host_name: "GPU server",
    });

    const [path, init] = relayFetch.mock.calls[0] as [string, RequestInit];
    expect(path).toBe("/api/v1/runtime-host-onboarding");
    expect(new Headers(init.headers).has("authorization")).toBe(false);
    expect(JSON.parse(init.body as string)).toEqual(payload);
  });

  it("shows the remote onboarding diagnostic instead of stringifying its error envelope", async () => {
    const relayFetch = vi.fn().mockResolvedValue(new Response(JSON.stringify({
      error: { detail: "connector pairing failed" },
    }), { status: 500, headers: { "content-type": "application/json" } }));

    await expect(onboardRuntimeHost(
      { fetch: relayFetch, socket: vi.fn() },
      {
        controller_gateway_url: "https://gateway.example",
        controller_credential: "v1.AAAAAAAAAAAAAAAAAAAAAA.BBBBBBBBBBBBBBBBBBBBBB",
        runtime_host_code: "12345678",
        name: "GPU server",
      },
    )).rejects.toThrow("connector pairing failed");
  });
});
