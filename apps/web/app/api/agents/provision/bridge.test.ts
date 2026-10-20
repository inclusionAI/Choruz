// @vitest-environment node
import { NextRequest } from "next/server";
import { afterEach, expect, it, vi } from "vitest";
import { ChoruzClient } from "../../../../../../services/choruz-bridge/src/choruz-client";
import { provisionAgent } from "../../../../lib/agents/agent-provisioning";
import { POST } from "./route";

vi.mock("../../../../lib/agents/agent-provisioning", async (original) => ({
  ...await original<typeof import("../../../../lib/agents/agent-provisioning")>(),
  provisionAgent: vi.fn(),
}));
vi.mock("../../../../lib/agents/agent-provisioning-idempotency", () => ({
  ProvisioningIdempotencyConflictError: class extends Error {},
  withProvisioningIdempotency: (_actor: unknown, _body: unknown, action: () => unknown) => action(),
}));

afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.unstubAllEnvs(); });

it("provisions through the web route's real cookie auth and body validation", async () => {
  const gateway = "http://gateway.invalid";
  const web = "http://web.invalid";
  vi.stubEnv("CHORUZ_API_BASE_URL", gateway);
  const token = `${Buffer.from(JSON.stringify({ principal_id: "owner", workspace_id: "workspace", principal_type: "human" })).toString("base64url")}.test-signature`;
  vi.mocked(provisionAgent).mockResolvedValue({ agent: { id: "created-agent" } } as Awaited<ReturnType<typeof provisionAgent>>);
  const requests: string[] = [];
  // Only gateway authentication and durable provisioning are replaced. The
  // bridge client, Next route, requireAuth and request validation run unchanged.
  vi.stubGlobal("fetch", async (url: string, init?: RequestInit) => {
    requests.push(url);
    if (url === `${gateway}/v1/auth/local/login`) {
      return Response.json({ session_token: token, principal: { id: "owner" } });
    }
    if (url === `${gateway}/v1/me`) {
      expect(new Headers(init?.headers).get("authorization")).toBe(`Bearer ${token}`);
      return Response.json({ id: "owner" });
    }
    if (url !== `${web}/api/agents/provision`) return new Response("Not found", { status: 404 });
    expect(new Headers(init?.headers).get("authorization")).toBeNull();
    return POST(new NextRequest(url, { ...init, signal: init?.signal ?? undefined }));
  });
  const client = new ChoruzClient({ api_url: gateway, web_url: web, username: "owner", password: "fixture" });
  await expect(client.provisionAgent({ name: "Helper", driver: "codex_terminal" })).resolves.toEqual({ principal_id: "created-agent" });
  expect(provisionAgent).toHaveBeenCalledWith(expect.objectContaining({
    sessionToken: token, actorId: "owner",
    body: { name: "Helper", driver_type: "codex_terminal", instructions: "" },
  }));
  expect(requests).toEqual([`${gateway}/v1/auth/local/login`, `${web}/api/agents/provision`, `${gateway}/v1/me`]);
  const rejected = await POST(new NextRequest(`${web}/api/agents/provision`, {
    method: "POST", headers: { authorization: `Bearer ${token}` },
    body: JSON.stringify({ name: "Helper", driver_type: "codex_terminal", instructions: "" }),
  }));
  expect(rejected.status).toBe(401);
  expect(provisionAgent).toHaveBeenCalledTimes(1);
});
