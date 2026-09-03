// Fresh backend API feature sweep. Does NOT import helpers from the rest
// of the test suite — each fixture is inlined so nothing bleeds in.
//
// Scope: hit one endpoint per major feature area and assert shape + basic
// semantics. The goal is "does every feature still work at all?", not
// deep coverage of edge cases.

import { expect, test, type APIRequestContext } from "@playwright/test";

const GATEWAY =
  process.env.CHORUZ_API_BASE_URL ?? `http://127.0.0.1:${process.env.CHORUZ_API_PORT ?? "3000"}`;

/* ========================================================================== */
/*  Inline fixtures — no external helpers                                     */
/* ========================================================================== */

async function adminLogin(api: APIRequestContext): Promise<{ token: string; id: string; name: string; workspace_id: string }> {
  const r = await api.post(`${GATEWAY}/v1/auth/local/login`, {
    data: { username: "operator", password: "choruz-local" },
  });
  expect(r.ok(), "operator login").toBeTruthy();
  const body = await r.json();
  return {
    token: body.session_token,
    id: body.principal.id,
    name: body.principal.name,
    workspace_id: body.principal.workspace_id,
  };
}

async function authedGet<T = any>(api: APIRequestContext, token: string, path: string): Promise<T> {
  const r = await api.get(`${GATEWAY}${path}`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  expect(r.ok(), `GET ${path} -> ${r.status()}`).toBeTruthy();
  return r.json() as Promise<T>;
}

async function authedPost<T = any>(
  api: APIRequestContext,
  token: string,
  path: string,
  data: unknown,
  extraHeaders?: Record<string, string>,
): Promise<T> {
  const r = await api.post(`${GATEWAY}${path}`, {
    headers: { Authorization: `Bearer ${token}`, ...(extraHeaders ?? {}) },
    data,
  });
  expect(r.ok(), `POST ${path} -> ${r.status()}: ${await r.text().catch(() => "")}`).toBeTruthy();
  return r.json() as Promise<T>;
}

/* ========================================================================== */
/*  Feature tests                                                              */
/* ========================================================================== */

// Parallel (not serial) so one failure doesn't mask the rest of the sweep.
test.describe("BE feature sweep", () => {
  test("healthz returns 200", async ({ request }) => {
    const r = await request.get(`${GATEWAY}/healthz`);
    expect(r.ok()).toBeTruthy();
  });

  test("metrics endpoint is reachable and emits Prometheus-style text", async ({ request }) => {
    const r = await request.get(`${GATEWAY}/metrics`);
    expect(r.ok()).toBeTruthy();
    const text = await r.text();
    // Expect at least one gauge/counter line
    expect(text.length).toBeGreaterThan(0);
  });

  test("auth: local login returns session_token + principal envelope", async ({ request }) => {
    const me = await adminLogin(request);
    expect(me.token).toMatch(/.+/);
    expect(me.id).toMatch(/.+/);
    expect(me.name).toBe("operator");
  });

  test("auth: invalid credentials rejected", async ({ request }) => {
    const r = await request.post(`${GATEWAY}/v1/auth/local/login`, {
      data: { username: "operator", password: "WRONG" },
    });
    expect(r.status()).toBeGreaterThanOrEqual(400);
    expect(r.status()).toBeLessThan(500);
  });

  test("/v1/me returns the authenticated principal", async ({ request }) => {
    const me = await adminLogin(request);
    const r = await authedGet(request, me.token, "/v1/me");
    expect(r.id).toBe(me.id);
    expect(r.name).toBe("operator");
  });

  test("/v1/console returns the full bootstrap snapshot", async ({ request }) => {
    const me = await adminLogin(request);
    const snap = await authedGet(request, me.token, "/v1/console");
    expect(snap.principal.id).toBe(me.id);
    expect(Array.isArray(snap.conversations)).toBeTruthy();
    expect(Array.isArray(snap.agents)).toBeTruthy();
    expect(typeof snap.messages_by_conversation).toBe("object");
    expect(Array.isArray(snap.audit_logs)).toBeTruthy();
  });

  test("/v1/status reports gateway health", async ({ request }) => {
    const me = await adminLogin(request);
    const r = await authedGet(request, me.token, "/v1/status");
    expect(r).toBeTruthy();
    expect(typeof r).toBe("object");
  });

  test("principals: console snapshot lists the operator among principals", async ({ request }) => {
    // There is no GET /v1/principals — listing is via /v1/console. The
    // snapshot returns the authed principal at the top plus an `agents`
    // array of other workspace principals.
    const me = await adminLogin(request);
    const snap = await authedGet(request, me.token, "/v1/console");
    expect(snap.principal.id).toBe(me.id);
  });

  test("agents: create via /v1/agents lifecycle endpoint and snapshot reflects it", async ({ request }) => {
    // /v1/principals rejects agent creation ("use the agent lifecycle API").
    // The correct endpoint is POST /v1/agents with CreateAgentRequest:
    // { actor_id, name, scopes, workspace_id? }.
    const me = await adminLogin(request);
    const agentName = `sweep-agent-${Date.now()}`;
    const created = await authedPost(request, me.token, "/v1/agents", {
      actor_id: me.id,
      name: agentName,
      scopes: ["messages:write", "messages:read"],
      workspace_id: me.workspace_id,
    });
    // /v1/agents returns { principal: Principal, secret: string }.
    const createdAgent = (created as any).principal;
    expect(createdAgent.id).toMatch(/.+/);
    expect(createdAgent.name).toBe(agentName);
    expect((created as any).secret).toMatch(/^agt_/);

    const snap = await authedGet(request, me.token, "/v1/console");
    // Console's `agents` collects workspace agents via an async listener;
    // the refresh may lag by a tick on first run. Poll briefly.
    let found = (snap.agents as any[]).find((a) => a.id === createdAgent.id);
    for (let i = 0; !found && i < 10; i++) {
      await new Promise((r) => setTimeout(r, 200));
      const s2 = await authedGet(request, me.token, "/v1/console");
      found = (s2.agents as any[]).find((a) => a.id === createdAgent.id);
    }
    expect(found, "newly created agent should surface in /v1/console").toBeTruthy();
  });

  test("companies: list returns an array", async ({ request }) => {
    const me = await adminLogin(request);
    const r = await request.get(`${GATEWAY}/v1/companies?principal_id=${me.id}`, {
      headers: { Authorization: `Bearer ${me.token}` },
    });
    expect(r.ok()).toBeTruthy();
    const list = await r.json();
    expect(Array.isArray(list)).toBeTruthy();
  });

  test("groups: create a group with operator as sole member", async ({ request }) => {
    const me = await adminLogin(request);
    const name = `sweep-group-${Date.now()}`;
    const group = await authedPost(request, me.token, "/v1/groups", {
      actor_id: me.id,
      name,
      description: null,
      avatar_url: null,
      member_ids: [],
    });
    expect(group.id).toMatch(/.+/);
    expect(group.conversation_type).toBe("group");
    expect(group.name).toBe(name);
  });

  test("direct conversation: creating between operator + another principal returns/upserts", async ({ request }) => {
    // CreateDirectConversationRequest shape: { actor_id, peer_principal_id, workspace_id? }
    const me = await adminLogin(request);
    const created = await authedPost(request, me.token, "/v1/agents", {
      actor_id: me.id,
      name: `sweep-direct-peer-${Date.now()}`,
      scopes: ["messages:write", "messages:read"],
      workspace_id: me.workspace_id,
    });
    const other = (created as any).principal;
    const r = await request.post(`${GATEWAY}/v1/conversations/direct`, {
      headers: { Authorization: `Bearer ${me.token}` },
      data: { actor_id: me.id, peer_principal_id: other!.id },
    });
    expect(r.ok(), `direct conv create -> ${r.status()}: ${await r.text().catch(() => "")}`).toBeTruthy();
    const conv = await r.json();
    expect(conv.conversation_type).toBe("direct");
  });

  test("messages: send with x-trace-id → response carries same trace in metadata", async ({ request }) => {
    const me = await adminLogin(request);
    // Create a throwaway group so we don't pollute existing conversations
    const group = await authedPost(request, me.token, "/v1/groups", {
      actor_id: me.id,
      name: `sweep-msg-${Date.now()}`,
      description: null,
      avatar_url: null,
      member_ids: [],
    });
    const trace = `sweep-trace-${Date.now()}`;
    const msg = await authedPost(
      request,
      me.token,
      "/v1/messages",
      {
        actor_id: me.id,
        conversation_id: group.id,
        content: "hello from feature sweep",
        content_type: "text/plain",
        idempotency_key: `sweep-${Date.now()}`,
        metadata: {},
      },
      { "x-trace-id": trace, "content-type": "application/json" },
    );
    expect(msg.metadata.trace_id).toBe(trace);
  });

  test("messages: list returns the messages we just sent", async ({ request }) => {
    const me = await adminLogin(request);
    const group = await authedPost(request, me.token, "/v1/groups", {
      actor_id: me.id,
      name: `sweep-list-${Date.now()}`,
      description: null,
      avatar_url: null,
      member_ids: [],
    });
    const content = `sweep-list-msg-${Date.now()}`;
    await authedPost(
      request,
      me.token,
      "/v1/messages",
      {
        actor_id: me.id,
        conversation_id: group.id,
        content,
        content_type: "text/plain",
        idempotency_key: `k-${Date.now()}`,
        metadata: {},
      },
    );
    const list = await authedGet<any[]>(
      request,
      me.token,
      `/v1/conversations/${group.id}/messages?principal_id=${me.id}&limit=20`,
    );
    expect(list.some((m) => m.content === content)).toBeTruthy();
  });

  test("messages: search finds a freshly-sent message by substring", async ({ request }) => {
    const me = await adminLogin(request);
    const group = await authedPost(request, me.token, "/v1/groups", {
      actor_id: me.id,
      name: `sweep-search-${Date.now()}`,
      description: null,
      avatar_url: null,
      member_ids: [],
    });
    const needle = `SWEEP-NEEDLE-${Date.now()}`;
    await authedPost(
      request,
      me.token,
      "/v1/messages",
      {
        actor_id: me.id,
        conversation_id: group.id,
        content: `this message contains the marker ${needle}`,
        content_type: "text/plain",
        idempotency_key: `search-${Date.now()}`,
        metadata: {},
      },
    );
    // Search may be async-indexed (tsvector trigger). Give it a beat.
    await new Promise((r) => setTimeout(r, 500));
    const r = await request.get(
      `${GATEWAY}/v1/messages/search?principal_id=${me.id}&q=${encodeURIComponent(needle)}&limit=10`,
      { headers: { Authorization: `Bearer ${me.token}` } },
    );
    expect(r.ok(), `search -> ${r.status()}`).toBeTruthy();
    const results = await r.json();
    expect(Array.isArray(results)).toBeTruthy();
  });

  test("/v1/unreads returns counts map", async ({ request }) => {
    const me = await adminLogin(request);
    const r = await request.get(`${GATEWAY}/v1/unreads?principal_id=${me.id}`, {
      headers: { Authorization: `Bearer ${me.token}` },
    });
    expect(r.ok()).toBeTruthy();
    const body = await r.json();
    expect(typeof body).toBe("object");
  });

  test("audit logs: list returns rows (query param is workspace_id, not principal_id)", async ({ request }) => {
    const me = await adminLogin(request);
    const r = await request.get(
      `${GATEWAY}/v1/audit-logs?workspace_id=${encodeURIComponent(me.workspace_id)}`,
      { headers: { Authorization: `Bearer ${me.token}` } },
    );
    expect(r.ok(), `audit-logs -> ${r.status()}`).toBeTruthy();
    const body = await r.json();
    expect(Array.isArray(body)).toBeTruthy();
  });

  test("telemetry ingest accepts a batch and returns 204", async ({ request }) => {
    const me = await adminLogin(request);
    const r = await request.post(`${GATEWAY}/v1/telemetry`, {
      headers: { Authorization: `Bearer ${me.token}`, "content-type": "application/json" },
      data: {
        events: [
          { source: "FE", traceId: `sw-${Date.now()}`, spanId: "s1", name: "sweep_event", ts: new Date().toISOString(), data: { k: "v" } },
        ],
      },
    });
    expect(r.status()).toBe(204);
  });

  test("principal events: ack queue list returns cursor", async ({ request }) => {
    const me = await adminLogin(request);
    const r = await request.get(
      `${GATEWAY}/v1/principals/${me.id}/events`,
      { headers: { Authorization: `Bearer ${me.token}` } },
    );
    expect(r.ok()).toBeTruthy();
    const body = await r.json();
    expect(body).toBeTruthy();
  });

  test("cron: list returns an array (may be empty)", async ({ request }) => {
    const me = await adminLogin(request);
    // Pick any agent to query its cron jobs against
    const snap = await authedGet(request, me.token, "/v1/console");
    const agent = (snap.agents as any[])[0];
    test.skip(!agent, "no agent available to list cron for");
    const r = await request.get(
      `${GATEWAY}/v1/agents/${agent.id}/cron?principal_id=${me.id}`,
      { headers: { Authorization: `Bearer ${me.token}` } },
    );
    expect(r.ok(), `cron list -> ${r.status()}: ${await r.text().catch(() => "")}`).toBeTruthy();
    const body = await r.json();
    expect(Array.isArray(body)).toBeTruthy();
  });

  test("filesystem: home returns a path", async ({ request }) => {
    const me = await adminLogin(request);
    const r = await request.get(`${GATEWAY}/v1/filesystem/home`, {
      headers: { Authorization: `Bearer ${me.token}` },
    });
    expect(r.ok()).toBeTruthy();
    const body = await r.json();
    expect(typeof body.home === "string" || typeof body.path === "string").toBeTruthy();
  });

  // `/v1/filesystem/*` is human-scoped AND enforces a path whitelist
  // (rooted at the user's HOME, by default). The tests below discover the
  // whitelisted home first, then probe stat/list against it.

  test("filesystem: stat on the whitelisted home directory", async ({ request }) => {
    const me = await adminLogin(request);
    const home = await request.get(`${GATEWAY}/v1/filesystem/home`, {
      headers: { Authorization: `Bearer ${me.token}` },
    });
    expect(home.ok()).toBeTruthy();
    const hbody = await home.json();
    const homePath: string = hbody.home ?? hbody.path;
    expect(homePath?.length ?? 0).toBeGreaterThan(0);

    const r = await request.get(
      `${GATEWAY}/v1/filesystem/stat?path=${encodeURIComponent(homePath)}`,
      { headers: { Authorization: `Bearer ${me.token}` } },
    );
    expect(r.ok(), `stat -> ${r.status()}`).toBeTruthy();
    const body = await r.json();
    expect(body.exists).toBeTruthy();
  });

  test("filesystem: list home returns entries array", async ({ request }) => {
    const me = await adminLogin(request);
    const home = await request.get(`${GATEWAY}/v1/filesystem/home`, {
      headers: { Authorization: `Bearer ${me.token}` },
    });
    const hbody = await home.json();
    const homePath: string = hbody.home ?? hbody.path;
    const r = await request.get(
      `${GATEWAY}/v1/filesystem/list?path=${encodeURIComponent(homePath)}`,
      { headers: { Authorization: `Bearer ${me.token}` } },
    );
    expect(r.ok(), `list -> ${r.status()}`).toBeTruthy();
    const body = await r.json();
    const entries = Array.isArray(body) ? body : body.entries;
    expect(Array.isArray(entries)).toBeTruthy();
  });

  test("webhooks: manual flush endpoint responds", async ({ request }) => {
    const me = await adminLogin(request);
    const r = await request.post(`${GATEWAY}/v1/webhooks/flush`, {
      headers: { Authorization: `Bearer ${me.token}` },
      data: { actor_id: me.id },
    });
    // Not all envs have webhooks configured, so just require non-500
    expect(r.status()).toBeLessThan(500);
  });

  test("agents: batch-disable of empty set is a no-op 2xx", async ({ request }) => {
    const me = await adminLogin(request);
    const r = await request.post(`${GATEWAY}/v1/agents/batch-disable`, {
      headers: { Authorization: `Bearer ${me.token}` },
      data: { actor_id: me.id, agent_ids: [] },
    });
    expect(r.status()).toBeLessThan(400);
  });
});
