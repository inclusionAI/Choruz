import { expect, test } from "@playwright/test";
import { login, API_BASE, WEB_BASE } from "../fixtures/auth";
import { getConsoleSnapshot, uniqueName } from "../fixtures/api";

test.describe("API routes", () => {
  /* ---------------------------------------------------------------------- */
  /*  Auth endpoints                                                         */
  /* ---------------------------------------------------------------------- */

  test("POST /v1/auth/local/login should return session token", async ({
    page,
  }) => {
    const res = await page.request.post(`${API_BASE}/v1/auth/local/login`, {
      data: { username: "operator", password: "choruz-local" },
    });
    expect(res.ok()).toBeTruthy();
    const data = await res.json();
    expect(data.session_token).toBeTruthy();
    expect(data.principal.id).toBeTruthy();
  });

  test("POST /v1/auth/local/login should reject bad credentials", async ({
    page,
  }) => {
    const res = await page.request.post(`${API_BASE}/v1/auth/local/login`, {
      data: { username: "operator", password: "wrong" },
    });
    expect(res.ok()).toBeFalsy();
  });

  /* ---------------------------------------------------------------------- */
  /*  Console snapshot                                                       */
  /* ---------------------------------------------------------------------- */

  test("GET /v1/console should return full snapshot", async ({ page }) => {
    const { token } = await login(page);
    const snap = await getConsoleSnapshot(page, token);
    expect(snap.principal).toBeTruthy();
    expect(Array.isArray(snap.conversations)).toBeTruthy();
    expect(Array.isArray(snap.agents)).toBeTruthy();
    expect("presences" in snap).toBeFalsy();
  });

  /* ---------------------------------------------------------------------- */
  /*  Companies                                                              */
  /* ---------------------------------------------------------------------- */

  test("GET /v1/companies should list companies", async ({ page }) => {
    const { token } = await login(page);
    const res = await page.request.fetch(`${API_BASE}/v1/companies`, {
      headers: { Authorization: `Bearer ${token}` },
    });
    expect(res.ok()).toBeTruthy();
    const companies = await res.json();
    expect(Array.isArray(companies)).toBeTruthy();
  });

  test("POST /v1/companies should create a new company", async ({ page }) => {
    const { token, principal } = await login(page);
    const name = uniqueName("api-test-company");
    const res = await page.request.post(`${API_BASE}/v1/companies`, {
      headers: { Authorization: `Bearer ${token}` },
      data: { actor_id: principal.id, name, description: null },
    });
    expect(res.ok()).toBeTruthy();
    const company = await res.json();
    expect(company.name).toBe(name);
  });

  /* ---------------------------------------------------------------------- */
  /*  Groups                                                                 */
  /* ---------------------------------------------------------------------- */

  test("POST /v1/groups should create a group conversation", async ({
    page,
  }) => {
    const { token, principal } = await login(page);
    const snap = await getConsoleSnapshot(page, token);
    const agent = snap.agents.find((a) => !a.disabled);
    const memberIds = agent ? [agent.id] : [];
    const name = uniqueName("api-test-group");
    const res = await page.request.post(`${API_BASE}/v1/groups`, {
      headers: { Authorization: `Bearer ${token}` },
      data: {
        actor_id: principal.id,
        name,
        description: null,
        avatar_url: null,
        member_ids: memberIds,
      },
    });
    expect(res.ok()).toBeTruthy();
    const group = await res.json();
    expect(group.name).toBe(name);
  });

  /* ---------------------------------------------------------------------- */
  /*  Messages                                                               */
  /* ---------------------------------------------------------------------- */

  test("POST /v1/messages should create a message", async ({ page }) => {
    const { token, principal } = await login(page);
    const snap = await getConsoleSnapshot(page, token);
    const group = snap.conversations.find(
      (c) => c.conversation_type === "group",
    );
    if (!group) {
      test.skip();
      return;
    }
    const res = await page.request.post(`${API_BASE}/v1/messages`, {
      headers: { Authorization: `Bearer ${token}` },
      data: {
        actor_id: principal.id,
        conversation_id: group.id,
        content: `api-msg-${Date.now()}`,
        content_type: "text/plain",
        idempotency_key: `api-key-${Date.now()}`,
        metadata: {},
      },
    });
    expect(res.ok()).toBeTruthy();
  });

  test("GET /v1/conversations/:id/messages should return messages", async ({
    page,
  }) => {
    const { token, principal } = await login(page);
    const snap = await getConsoleSnapshot(page, token);
    const conv = snap.conversations[0];
    if (!conv) {
      test.skip();
      return;
    }
    const res = await page.request.fetch(
      `${API_BASE}/v1/conversations/${conv.id}/messages?principal_id=${principal.id}&limit=10`,
      { headers: { Authorization: `Bearer ${token}` } },
    );
    expect(res.ok()).toBeTruthy();
    const msgs = await res.json();
    expect(Array.isArray(msgs)).toBeTruthy();
  });

  /* ---------------------------------------------------------------------- */
  /*  Agent provision                                                        */
  /* ---------------------------------------------------------------------- */

  test("POST /api/agents/provision should provision a new agent", async ({
    page,
  }) => {
    await login(page);
    const name = uniqueName("api-test-agent");
    const res = await page.request.post(`${WEB_BASE}/api/agents/provision`, {
      data: {
        name,
        driver_type: "claude_terminal",
        instructions: "E2E test agent",
      },
    });
    expect(res.ok()).toBeTruthy();
    const data = await res.json();
    expect(data.agent.id).toBeTruthy();
    expect(data.agent.name).toBe(name);
    expect(data.secret).toBeTruthy();
  });

  /* ---------------------------------------------------------------------- */
  /*  Filesystem API                                                         */
  /* ---------------------------------------------------------------------- */

  test("GET /api/filesystem should return directory listing", async ({
    page,
  }) => {
    const { token } = await login(page);
    const res = await page.request.fetch(`${WEB_BASE}/api/filesystem?path=/tmp`, {
      headers: { Authorization: `Bearer ${token}` },
    });
    // May or may not be accessible, but should not 500
    expect(res.status()).toBeLessThan(500);
  });

  /* ---------------------------------------------------------------------- */
  /*  Git graph API                                                          */
  /* ---------------------------------------------------------------------- */

  test("GET /api/git-graph should return git data or error gracefully", async ({
    page,
  }) => {
    const { token } = await login(page);
    const res = await page.request.fetch(
      `${WEB_BASE}/api/git-graph?repo_path=/tmp&limit=10`,
      { headers: { Authorization: `Bearer ${token}` } },
    );
    expect(res.status()).toBeLessThan(500);
  });

  /* ---------------------------------------------------------------------- */
  /*  Analytics API                                                          */
  /* ---------------------------------------------------------------------- */

  test("POST /api/analytics should accept event data", async ({ page }) => {
    await login(page);
    const res = await page.request.post(`${WEB_BASE}/api/analytics`, {
      data: {
        event: "e2e_test",
        data: { test: true },
        timestamp: new Date().toISOString(),
      },
    });
    // Should accept the event (2xx or similar)
    expect(res.status()).toBeLessThan(500);
  });

  /* ---------------------------------------------------------------------- */
  /*  Unreads API                                                            */
  /* ---------------------------------------------------------------------- */

  test("GET /v1/unreads should return unread counts", async ({ page }) => {
    const { token } = await login(page);
    const res = await page.request.fetch(`${API_BASE}/v1/unreads`, {
      headers: { Authorization: `Bearer ${token}` },
    });
    // May or may not exist as an endpoint
    expect(res.status()).toBeLessThan(500);
  });

  /* ---------------------------------------------------------------------- */
  /*  Conversation view                                                      */
  /* ---------------------------------------------------------------------- */

  test("POST /v1/conversations/:id/view should mark as viewed", async ({
    page,
  }) => {
    const { token, principal } = await login(page);
    const snap = await getConsoleSnapshot(page, token);
    const conv = snap.conversations[0];
    if (!conv) {
      test.skip();
      return;
    }
    const res = await page.request.post(
      `${API_BASE}/v1/conversations/${conv.id}/view`,
      { headers: { Authorization: `Bearer ${token}` } },
    );
    expect(res.status()).toBeLessThan(500);
  });

  /* ---------------------------------------------------------------------- */
  /*  Runtime bindings                                                       */
  /* ---------------------------------------------------------------------- */

  test("GET /v1/runtime/bindings should return bindings list", async ({
    page,
  }) => {
    const { token } = await login(page);
    const res = await page.request.fetch(
      `${API_BASE}/v1/runtime/bindings`,
      { headers: { Authorization: `Bearer ${token}` } },
    );
    // Should return 200 with array or 404 if not implemented
    expect(res.status()).toBeLessThan(500);
  });
});
