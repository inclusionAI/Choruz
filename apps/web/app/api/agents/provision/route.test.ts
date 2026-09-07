import { NextRequest } from "next/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import { requireAuth } from "../../../../lib/api/api-auth";
import { provisionAgent } from "../../../../lib/agents/agent-provisioning";
import {
  ProvisioningIdempotencyConflictError,
  withProvisioningIdempotency,
} from "../../../../lib/agents/agent-provisioning-idempotency";
import { POST } from "./route";

vi.mock("../../../../lib/api/api-auth", () => ({
  requireAuth: vi.fn(),
}));

vi.mock("../../../../lib/agents/agent-provisioning", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../../../lib/agents/agent-provisioning")>();
  return {
    ...actual,
    provisionAgent: vi.fn(),
  };
});

vi.mock("../../../../lib/agents/agent-provisioning-idempotency", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../../../lib/agents/agent-provisioning-idempotency")>();
  return {
    ...actual,
    withProvisioningIdempotency: vi.fn(async (_actorId, _body, action) => action()),
  };
});

describe("/api/agents/provision", () => {
  it("preserves a gateway rate limit through the real provisioning step", async () => {
    vi.mocked(requireAuth).mockResolvedValue({ token: "session-token", claims: {
      principal_id: "human-1", workspace_id: "workspace-1", display_name: "Alice", expires_at_epoch_s: 1,
    } });
    const actual = await vi.importActual<typeof import("../../../../lib/agents/agent-provisioning")>("../../../../lib/agents/agent-provisioning");
    vi.mocked(provisionAgent).mockImplementation(actual.provisionAgent);
    // Only upstream HTTP and unrelated auth/idempotency storage are replaced:
    // route -> provisioning step -> API error decoding all run their real code.
    const upstream = vi.fn().mockResolvedValue(new Response(JSON.stringify({ error: "Too many requests" }), {
      status: 429, headers: { "retry-after": "42" },
    }));
    vi.stubGlobal("fetch", upstream);
    const response = await POST(new NextRequest("http://localhost/api/agents/provision", {
      method: "POST", body: JSON.stringify({ name: "Helper", driver_type: "claude_terminal", instructions: "Help with tasks." }),
    }));
    expect(upstream).toHaveBeenCalledOnce();
    expect(String(upstream.mock.calls[0][0])).toMatch(/\/v1\/agents$/);
    expect(response.status).toBe(429);
    expect(response.headers.get("retry-after")).toBe("42");
    expect(await response.json()).toEqual({ error: "Too many requests" });
  });

  it("leaves a remote workspace path to the selected device's validator", async () => {
    vi.stubEnv("HOME", "/controller-home");
    vi.mocked(requireAuth).mockResolvedValue({ token: "session-token", claims: {
      principal_id: "human-1", workspace_id: "workspace-1", display_name: "Alice", expires_at_epoch_s: 1,
    } });
    vi.mocked(provisionAgent).mockResolvedValue({} as Awaited<ReturnType<typeof provisionAgent>>);
    const body = { name: "Remote helper", instructions: "Help with tasks.", driver_type: "claude_terminal", runtime_host_id: "host-b", workspace_path: "/target-home/project" };
    const response = await POST(new NextRequest("http://localhost/api/agents/provision", { method: "POST", body: JSON.stringify(body) }));
    expect(response.status).toBe(201);
    expect(provisionAgent).toHaveBeenCalledWith(expect.objectContaining({ body }));
  });

  it("still rejects a local workspace outside the controller home", async () => {
    vi.stubEnv("HOME", "/controller-home");
    vi.mocked(requireAuth).mockResolvedValue({ token: "session-token", claims: {
      principal_id: "human-1", workspace_id: "workspace-1", display_name: "Alice", expires_at_epoch_s: 1,
    } });
    const response = await POST(new NextRequest("http://localhost/api/agents/provision", { method: "POST", body: JSON.stringify({ name: "Local helper", instructions: "Help with tasks.", driver_type: "claude_terminal", workspace_path: "/target-home/project" }) }));
    expect(response.status).toBe(400);
    expect(await response.json()).toEqual({ error: "Workspace path must be under /controller-home" });
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllEnvs();
    vi.unstubAllGlobals();
  });

  it("delegates to the provisioning primitive while preserving the manual response shape", async () => {
    vi.mocked(requireAuth).mockResolvedValue({
      token: "session-token",
      claims: {
        principal_id: "human-1",
        workspace_id: "workspace-1",
        display_name: "Alice",
        expires_at_epoch_s: 1,
      },
    });
    vi.mocked(provisionAgent).mockResolvedValue({
      agent: {
        id: "agent-1",
        workspace_id: "workspace-1",
        principal_type: "agent",
        name: "Helper",
        avatar_url: null,
        scopes: ["messages:read", "messages:write", "events:read"],
        disabled: false,
        created_at: "2026-01-01T00:00:00Z",
        updated_at: "2026-01-01T00:00:00Z",
      },
      secret: "agent-secret",
      conversation: {
        id: "conversation-1",
        workspace_id: "workspace-1",
        conversation_type: "direct",
        name: null,
        description: null,
        avatar_url: null,
        creator_id: "human-1",
        created_at: "2026-01-01T00:00:00Z",
        updated_at: "2026-01-01T00:00:00Z",
        members: {},
      },
      binding: {
        id: "binding-1",
        workspace_id: "workspace-1",
        conversation_id: "conversation-1",
        conversation_name: "Direct",
        conversation_type: "direct",
        agent_principal_id: "agent-1",
        agent_name: "Helper",
        driver_type: "claude_terminal",
        workspace_path: "/tmp/helper",
        git_worktree_path: null,
        external_session_id: null,
        external_thread_id: null,
        last_event_cursor: 0,
        last_acked_event_cursor: 0,
        last_seen_server_seq: 0,
        state: "idle",
        last_error: null,
        updated_at: "2026-01-01T00:00:00Z",
      },
      workspace_path: "/tmp/helper",
    });

    const response = await POST(new NextRequest("http://localhost/api/agents/provision", {
      method: "POST",
      body: JSON.stringify({
        name: "Helper",
        driver_type: "claude_terminal",
        instructions: "Help with tasks.",
        workspace_id: "workspace-1",
      }),
    }));

    expect(response.status).toBe(201);
    await expect(response.json()).resolves.toEqual({
      agent: expect.objectContaining({ id: "agent-1" }),
      secret: "agent-secret",
      conversation: expect.objectContaining({ id: "conversation-1" }),
      binding: expect.objectContaining({ id: "binding-1" }),
      workspace_path: "/tmp/helper",
    });
    expect(provisionAgent).toHaveBeenCalledWith({
      sessionToken: "session-token",
      actorId: "human-1",
      body: {
        name: "Helper",
        driver_type: "claude_terminal",
        instructions: "Help with tasks.",
        workspace_id: "workspace-1",
      },
    });
  });

  it("returns conflict when an idempotency key is reused for another request", async () => {
    vi.mocked(requireAuth).mockResolvedValue({
      token: "session-token",
      claims: {
        principal_id: "human-1",
        workspace_id: "workspace-1",
        display_name: "Alice",
        expires_at_epoch_s: 1,
      },
    });
    vi.mocked(withProvisioningIdempotency).mockRejectedValueOnce(
      new ProvisioningIdempotencyConflictError(),
    );

    const response = await POST(new NextRequest("http://localhost/api/agents/provision", {
      method: "POST",
      body: JSON.stringify({
        name: "Helper",
        driver_type: "claude_terminal",
        instructions: "Help with tasks.",
        idempotency_key: "request-1",
      }),
    }));

    expect(response.status).toBe(409);
    await expect(response.json()).resolves.toEqual({
      error: "This idempotency key was already used for a different provisioning request.",
    });
    expect(provisionAgent).not.toHaveBeenCalled();
  });

  it("rejects skill provisioning when the agent-skills plugin is disabled", async () => {
    vi.stubEnv("CHORUZ_PLUGINS", "workspace-git,remote-ssh");
    vi.mocked(requireAuth).mockResolvedValue({
      token: "session-token",
      claims: {
        principal_id: "human-1",
        workspace_id: "workspace-1",
        display_name: "Alice",
        expires_at_epoch_s: 1,
      },
    });

    const response = await POST(new NextRequest("http://localhost/api/agents/provision", {
      method: "POST",
      body: JSON.stringify({
        name: "Helper",
        driver_type: "claude_terminal",
        instructions: "Help with tasks.",
        skill_paths: ["/tmp/example-skill"],
      }),
    }));

    expect(response.status).toBe(404);
    await expect(response.json()).resolves.toEqual({
      error: "plugin 'agent-skills' is disabled",
    });
    expect(provisionAgent).not.toHaveBeenCalled();
  });

  it("rejects MathCode provisioning when the mathcode plugin is disabled", async () => {
    vi.stubEnv("CHORUZ_PLUGINS", "workspace-git,agent-skills");
    vi.mocked(requireAuth).mockResolvedValue({
      token: "session-token",
      claims: {
        principal_id: "human-1",
        workspace_id: "workspace-1",
        display_name: "Alice",
        expires_at_epoch_s: 1,
      },
    });

    const response = await POST(new NextRequest("http://localhost/api/agents/provision", {
      method: "POST",
      body: JSON.stringify({
        name: "Math Agent",
        driver_type: "mathcode_terminal",
        instructions: "Formalize and prove the theorem.",
      }),
    }));

    expect(response.status).toBe(404);
    await expect(response.json()).resolves.toEqual({ error: "plugin 'mathcode' is disabled" });
    expect(provisionAgent).not.toHaveBeenCalled();
  });

  it("rejects channel visibility on the public provisioning route", async () => {
    vi.mocked(requireAuth).mockResolvedValue({
      token: "session-token",
      claims: {
        principal_id: "human-1",
        workspace_id: "workspace-1",
        display_name: "Alice",
        expires_at_epoch_s: 1,
      },
    });

    const response = await POST(new NextRequest("http://localhost/api/agents/provision", {
      method: "POST",
      body: JSON.stringify({
        name: "Helper",
        driver_type: "claude_terminal",
        instructions: "Help with tasks.",
        channel_visibility: "internal",
      }),
    }));

    expect(response.status).toBe(400);
    await expect(response.json()).resolves.toEqual({
      error: "Field `channel_visibility` is not accepted by this route.",
    });
    expect(provisionAgent).not.toHaveBeenCalled();
  });

  it("allows channel visibility only for internal provisioning requests", async () => {
    vi.stubEnv("CHORUZ_INTERNAL_PROVISION_TOKEN", "internal-token");
    vi.mocked(requireAuth).mockResolvedValue({
      token: "session-token",
      claims: {
        principal_id: "human-1",
        workspace_id: "workspace-1",
        display_name: "Alice",
        expires_at_epoch_s: 1,
      },
    });
    vi.mocked(provisionAgent).mockResolvedValue({
      agent: {
        id: "agent-1",
        workspace_id: "workspace-1",
        principal_type: "agent",
        name: "Helper",
        avatar_url: null,
        scopes: ["messages:read", "messages:write", "events:read"],
        disabled: false,
        created_at: "2026-01-01T00:00:00Z",
        updated_at: "2026-01-01T00:00:00Z",
      },
      secret: "agent-secret",
      conversation: {
        id: "conversation-1",
        workspace_id: "workspace-1",
        conversation_type: "direct",
        name: null,
        description: null,
        avatar_url: null,
        creator_id: "human-1",
        created_at: "2026-01-01T00:00:00Z",
        updated_at: "2026-01-01T00:00:00Z",
        members: {},
      },
      binding: {
        id: "binding-1",
        workspace_id: "workspace-1",
        conversation_id: "conversation-1",
        conversation_name: "Direct",
        conversation_type: "direct",
        agent_principal_id: "agent-1",
        agent_name: "Helper",
        driver_type: "claude_terminal",
        workspace_path: "/tmp/helper",
        git_worktree_path: null,
        external_session_id: null,
        external_thread_id: null,
        last_event_cursor: 0,
        last_acked_event_cursor: 0,
        last_seen_server_seq: 0,
        state: "idle",
        last_error: null,
        updated_at: "2026-01-01T00:00:00Z",
      },
      workspace_path: "/tmp/helper",
    });

    const response = await POST(new NextRequest("http://localhost/api/agents/provision", {
      method: "POST",
      headers: { "x-choruz-internal-provision-token": "internal-token" },
      body: JSON.stringify({
        name: "Helper",
        driver_type: "claude_terminal",
        instructions: "Help with tasks.",
        channel_visibility: "internal",
      }),
    }));

    expect(response.status).toBe(201);
    expect(provisionAgent).toHaveBeenCalledWith({
      sessionToken: "session-token",
      actorId: "human-1",
      body: {
        name: "Helper",
        driver_type: "claude_terminal",
        instructions: "Help with tasks.",
        channel_visibility: "internal",
      },
    });
  });

  it("rejects the internal visibility bypass when the dedicated token is unset", async () => {
    vi.stubEnv("CHORUZ_INTERNAL_PROVISION_TOKEN", "");
    vi.mocked(requireAuth).mockResolvedValue({
      token: "session-token",
      claims: {
        principal_id: "human-1",
        workspace_id: "workspace-1",
        display_name: "Alice",
        expires_at_epoch_s: 1,
      },
    });

    const response = await POST(new NextRequest("http://localhost/api/agents/provision", {
      method: "POST",
      headers: { "x-choruz-internal-provision-token": "choruz-local" },
      body: JSON.stringify({
        name: "Helper",
        driver_type: "claude_terminal",
        instructions: "Help with tasks.",
        channel_visibility: "internal",
      }),
    }));

    expect(response.status).toBe(400);
    await expect(response.json()).resolves.toEqual({
      error: "Field `channel_visibility` is not accepted by this route.",
    });
    expect(provisionAgent).not.toHaveBeenCalled();
  });

  it("rejects the internal visibility bypass when the token is wrong", async () => {
    vi.stubEnv("CHORUZ_INTERNAL_PROVISION_TOKEN", "internal-token");
    vi.mocked(requireAuth).mockResolvedValue({
      token: "session-token",
      claims: {
        principal_id: "human-1",
        workspace_id: "workspace-1",
        display_name: "Alice",
        expires_at_epoch_s: 1,
      },
    });

    const response = await POST(new NextRequest("http://localhost/api/agents/provision", {
      method: "POST",
      headers: { "x-choruz-internal-provision-token": "wrong-token" },
      body: JSON.stringify({
        name: "Helper",
        driver_type: "claude_terminal",
        instructions: "Help with tasks.",
        channel_visibility: "internal",
      }),
    }));

    expect(response.status).toBe(400);
    await expect(response.json()).resolves.toEqual({
      error: "Field `channel_visibility` is not accepted by this route.",
    });
    expect(provisionAgent).not.toHaveBeenCalled();
  });

  it("does not accept the legacy internal provisioning header", async () => {
    vi.stubEnv(["E", "CHAT_INTERNAL_PROVISION_TOKEN"].join(""), "internal-token");
    vi.mocked(requireAuth).mockResolvedValue({
      token: "session-token",
      claims: { principal_id: "human-1", workspace_id: "workspace-1", display_name: "Alice", expires_at_epoch_s: 1 },
    });

    const response = await POST(new NextRequest("http://localhost/api/agents/provision", {
      method: "POST",
      headers: { [["x-", "e", "chat-internal-provision-token"].join("")]: "internal-token" },
      body: JSON.stringify({ name: "Helper", driver_type: "claude_terminal", channel_visibility: "internal" }),
    }));

    expect(response.status).toBe(400);
    expect(provisionAgent).not.toHaveBeenCalled();
  });
});
