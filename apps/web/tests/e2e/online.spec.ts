import { randomUUID } from "node:crypto";
import { createServer } from "node:http";
import { expect, test, type Page } from "@playwright/test";
import { API_BASE, gotoDashboard, signup } from "../fixtures/auth";
import { createAgent } from "../fixtures/api";
import { postgresQueryClient } from "../../lib/groups/group-provisioning-db";

async function submitOnline(page: Page, action: "sign-up" | "sign-in", password: string) {
  for (let attempt = 0; attempt < 6; attempt++) {
    await page.getByLabel("Password", { exact: true }).fill(password);
    const responsePromise = page.waitForResponse(response => response.url().endsWith(`/v1/online/${action}`) && response.request().method() === "POST");
    await page.getByRole("button", { name: action === "sign-up" ? "Create account and sign in" : "Sign in", exact: true }).click();
    const response = await responsePromise;
    if (response.status() !== 429) {
      expect(response.status()).toBe(200);
      return;
    }
    await expect(page.getByRole("dialog").getByRole("alert")).toContainText("Too many requests");
    const delay = Number(response.headers()["retry-after"]);
    expect(delay).toBeGreaterThan(0);
    expect(delay).toBeLessThanOrEqual(60);
    // Parallel accounts share the fixture's IP. Honor the real service's limit.
    await new Promise(resolve => setTimeout(resolve, delay * 1000));
  }
  throw new Error("Online account service remained rate limited");
}

test("Online group invitation exchanges real encrypted messages without granting device access", async ({ browser }) => {
  test.setTimeout(120_000);
  const contexts = await Promise.all([browser.newContext(), browser.newContext(), browser.newContext()]);
  const pages = await Promise.all(contexts.map(context => context.newPage()));
  const [owner, guest, outsider] = await Promise.all(pages.map((page, i) => signup(page, `online-group-${i}-${randomUUID().slice(0, 12)}`, "local-password-for-test")));
  const [a, b, c] = pages;
  const received: string[] = [];
  const webhook = createServer((request, response) => {
    let body = "";
    request.setEncoding("utf8");
    request.on("data", chunk => { body += chunk; });
    request.on("end", () => {
      received.push(JSON.parse(body).payload?.content ?? "");
      response.writeHead(200).end();
    });
  });
  await new Promise<void>((resolve, reject) => { webhook.once("error", reject); webhook.listen(0, "127.0.0.1", resolve); });
  const address = webhook.address();
  if (!address || typeof address === "string") throw new Error("Online test webhook did not bind");
  const auth = (token: string) => ({ Authorization: `Bearer ${token}` });
  const open = async (page: typeof a) => {
    await gotoDashboard(page);
    await page.getByRole("button", { name: "Actions menu" }).click();
    await page.getByRole("button", { name: "Online", exact: true }).click();
  };
  try {
    const agent = await createAgent(a, owner.token, owner.principal.id, "online-reply-agent");
    const groupResponse = await a.request.post(`${API_BASE}/v1/groups`, { headers: auth(owner.token), data: { actor_id: owner.principal.id, name: "Online shared acceptance", member_ids: [agent.principal.id] } });
    expect(groupResponse.ok()).toBeTruthy();
    const group = await groupResponse.json();
    const localGroup = await b.request.post(`${API_BASE}/v1/groups`, { headers: auth(guest.token), data: { actor_id: guest.principal.id, name: "Local planning", member_ids: [] } });
    expect(localGroup.ok()).toBeTruthy();
    for (const page of pages) {
      await open(page);
      await page.getByRole("button", { name: "Create account", exact: true }).click();
      await page.getByLabel("Display name", { exact: true }).fill(page === a ? "Online Owner" : page === b ? "Online Guest" : "Online Outsider");
      await page.getByLabel("Email", { exact: true }).fill(`${randomUUID()}@example.test`);
      await submitOnline(page, "sign-up", `online-password-${randomUUID()}`);
      await expect(page.getByText("Connection: connected", { exact: true })).toBeVisible({ timeout: 20_000 });
    }
    await a.getByLabel("Group to share").selectOption(group.id);
    await a.getByRole("button", { name: "Create group invitation" }).click();
    const invitation = await a.getByLabel("Group invitation", { exact: true }).inputValue();
    expect(invitation).toMatch(/^choruz-online\./);
    await a.keyboard.press("Escape");
    await expect(a.getByRole("dialog")).toHaveCount(0);
    await a.locator(".conv-item").filter({ hasText: "Online shared acceptance" }).first().click();
    await b.getByLabel("Invitation to join").fill(invitation);
    await b.getByRole("button", { name: "Join group", exact: true }).click();
    await expect(b.getByPlaceholder("Message Online shared acceptance...")).toBeEnabled({ timeout: 30_000 });
    await expect(b.getByRole("dialog")).toHaveCount(0);
    await expect(b.locator(".chat-sidebar .conv-item").filter({ hasText: "Online shared acceptance" })).toBeVisible();
    await expect(b.getByRole("button", { name: "Attach file", exact: true })).toHaveCount(0);
    const guestLinksResponse = await b.request.get(`${API_BASE}/v1/online/groups`, { headers: auth(guest.token) });
    const guestLink = (await guestLinksResponse.json()).groups[0];
    const ownerLink = (await (await a.request.get(`${API_BASE}/v1/online/groups`, { headers: auth(owner.token) })).json()).groups[0];
    expect((await c.request.get(`${API_BASE}/v1/online/groups/${guestLink.id}/messages`, { headers: auth(outsider.token) })).status()).toBe(404);
    expect((await c.request.post(`${API_BASE}/v1/online/groups/join`, { headers: auth(outsider.token), data: { invitation } })).status()).toBe(400);
    // The outsider's permission checks are complete; stop its unrelated session polling.
    await c.goto("about:blank");
    const privateRead = await b.request.get(`${API_BASE}/v1/conversations/${group.id}/messages?principal_id=${guest.principal.id}`, { headers: auth(guest.token) });
    expect([403, 404]).toContain(privateRead.status());
    const ownerText = "Owner hello across the encrypted group link";
    await b.getByPlaceholder("Message Online shared acceptance...").fill("Shared group draft");
    await b.locator(".chat-sidebar .conv-item").filter({ hasText: "Local planning" }).click();
    await b.getByPlaceholder("Message Local planning...").fill("Local group draft");
    expect((await a.request.post(`${API_BASE}/v1/messages`, { headers: auth(owner.token), data: { actor_id: owner.principal.id, conversation_id: group.id, content: ownerText, content_type: "text", metadata: {}, idempotency_key: randomUUID() } })).ok()).toBeTruthy();
    const sharedRow = b.locator(".chat-sidebar .conv-item").filter({ hasText: "Online shared acceptance" });
    await expect(sharedRow.locator(".conv-unread")).toHaveText("1", { timeout: 30_000 });
    await expect(sharedRow.locator(".conv-preview")).toContainText("Owner hello");
    await sharedRow.click();
    await expect(b.getByPlaceholder("Message Online shared acceptance...")).toHaveValue("Shared group draft");
    await expect(sharedRow.locator(".conv-unread")).toHaveCount(0);
    await b.getByRole("button", { name: "Close Online shared acceptance", exact: true }).click();
    await expect(b.getByPlaceholder("Message Local planning...")).toHaveValue("Local group draft");
    await sharedRow.click();
    await expect(b.getByPlaceholder("Message Online shared acceptance...")).toHaveValue("Shared group draft");
    await b.getByPlaceholder("Message Online shared acceptance...").fill("");
    await expect(b.locator(".chat-main .messages-area").getByText(ownerText, { exact: true })).toBeVisible({ timeout: 30_000 });
    await expect(b.locator(".chat-main .messages-area").getByText("Online Owner", { exact: true })).toBeVisible();
    await expect(b.locator(".chat-main .messages-area").getByText("AI", { exact: true })).toHaveCount(0);
    const agentText = "Agent reply through the authenticated message API";
    const db = await postgresQueryClient();
    const bindingId = randomUUID();
    // Fixture the runtime identity, not the shared messages or their UI. No CLI is launched.
    await db.query("INSERT INTO agent_runtime_bindings (id, conversation_id, agent_principal_id, driver_type, workspace_path, state, config_json) VALUES ($1,$2,$3,'claude_terminal',$4,'paused',$5)", [bindingId, group.id, agent.principal.id, process.cwd(), JSON.stringify({ harness_account_name: "Research account" })]);
    expect((await a.request.post(`${API_BASE}/v1/messages`, { headers: auth(agent.secret), data: { actor_id: agent.principal.id, conversation_id: group.id, content: agentText, content_type: "text", metadata: {}, idempotency_key: randomUUID() } })).ok()).toBeTruthy();
    await expect(b.locator(".chat-main .messages-area").getByText(agentText, { exact: true })).toBeVisible({ timeout: 30_000 });
    await expect(b.locator(".chat-main .messages-area").getByText("AI", { exact: true })).toHaveCount(1);
    await expect(b.locator(".chat-main .messages-area .msg-runtime-host").filter({ hasText: "Research account" })).toHaveText("Group owner's device · Research account");
    await b.getByTitle("Toggle details", { exact: true }).click();
    const details = b.getByRole("dialog", { name: "Online shared acceptance", exact: true });
    await expect(details.getByText("online-reply-agent", { exact: true })).toBeVisible();
    await expect(details.getByText(/Choruz user/)).toBeVisible();
    await expect(details.getByText(/Agent owner: Online Owner/)).toBeVisible();
    await expect(details.getByText(/Harness account: Research account/)).toBeVisible();
    await expect(details.getByText(/Group owner's device/)).toBeVisible();
    await b.keyboard.press("Escape");
    const guestAgent = await createAgent(b, guest.token, guest.principal.id, "guest-online-assistant");
    const guestBinding = randomUUID();
    const guestDirect = await b.request.post(`${API_BASE}/v1/conversations/direct`, { headers: auth(guest.token), data: { actor_id: guest.principal.id, peer_principal_id: guestAgent.principal.id } });
    expect(guestDirect.status()).toBe(201);
    const guestDirectId = (await guestDirect.json()).id;
    await db.query("INSERT INTO agent_runtime_bindings (id, conversation_id, agent_principal_id, driver_type, workspace_path, state, config_json) VALUES ($1,$2,$3,'claude_terminal',$4,'paused',$5)", [guestBinding, guestDirectId, guestAgent.principal.id, process.cwd(), JSON.stringify({ harness_account_name: "Guest account" })]);
    await b.getByTitle("Toggle details", { exact: true }).click();
    const myAgents = b.getByRole("region", { name: "Your Agents in this group" });
    await myAgents.getByRole("button", { name: "Add guest-online-assistant", exact: true }).click();
    await expect(myAgents.getByText("In this group", { exact: true })).toBeVisible({ timeout: 30_000 });
    await b.keyboard.press("Escape");
    const toGuest = "@guest-online-assistant Check the guest execution path";
    expect((await a.request.post(`${API_BASE}/v1/messages`, { headers: auth(owner.token), data: { actor_id: owner.principal.id, conversation_id: group.id, content: toGuest, content_type: "text", metadata: {}, idempotency_key: randomUUID() } })).ok()).toBeTruthy();
    const guestMirror = `online-group:${guestLink.id}:${guest.principal.workspace_id}`;
    await expect.poll(async () => {
      const response = await b.request.get(`${API_BASE}/v1/conversations/${guestMirror}/messages?principal_id=${guestAgent.principal.id}`, { headers: auth(guestAgent.secret) });
      return response.status() === 200 ? await response.text() : "";
    }, { timeout: 30_000 }).toContain(toGuest);
    // Real authenticated Agent write and mailbox transport; a paused binding replaces model execution only.
    const guestReply = "Guest Agent response with its own identity";
    expect((await b.request.post(`${API_BASE}/v1/messages`, { headers: auth(guestAgent.secret), data: { actor_id: guestAgent.principal.id, conversation_id: guestMirror, content: guestReply, content_type: "text", metadata: {}, idempotency_key: randomUUID() } })).ok()).toBeTruthy();
    await expect(a.locator(".chat-primary").getByText(guestReply, { exact: true })).toBeVisible({ timeout: 30_000 });
    const hostGuestReply = a.locator(".chat-primary .msg-group").filter({ hasText: guestReply });
    await expect(hostGuestReply.getByText("Guest's device · Guest account", { exact: true })).toBeVisible();
    await expect(hostGuestReply.getByText("Agent owner: Online Guest", { exact: true })).toBeVisible();
    await expect(b.locator(".messages-area").getByText(guestReply, { exact: true })).toHaveCount(1, { timeout: 30_000 });
    await expect(b.locator(".messages-area .msg-runtime-host").filter({ hasText: "Guest account" })).toContainText("Guest's device");
    await expect(b.locator(".messages-area").getByText("Agent owner: Online Guest", { exact: true })).toBeVisible();
    await b.reload();
    await b.getByTitle("Toggle details", { exact: true }).click();
    await expect(b.getByRole("region", { name: "Your Agents in this group" }).getByText("In this group", { exact: true })).toBeVisible();
    await b.getByRole("button", { name: "Remove guest-online-assistant", exact: true }).click();
    await expect(b.getByRole("button", { name: "Add guest-online-assistant", exact: true })).toBeVisible();
    await b.keyboard.press("Escape");
    const lateReply = await b.request.post(`${API_BASE}/v1/messages`, { headers: auth(guestAgent.secret), data: { actor_id: guestAgent.principal.id, conversation_id: guestMirror, content: "Removed Agent must not auto-join", content_type: "text", metadata: {}, idempotency_key: randomUUID() } });
    expect(lateReply.status()).toBe(403);
    await db.query("DELETE FROM agent_runtime_bindings WHERE id=$1", [guestBinding]);
    await db.query("DELETE FROM agent_runtime_bindings WHERE id=$1 AND agent_principal_id=$2", [bindingId, agent.principal.id]);
    const registration = await a.request.post(`${API_BASE}/v1/principals/${agent.principal.id}/event-webhook`, { headers: auth(agent.secret), data: { actor_id: agent.principal.id, url: `http://127.0.0.1:${address.port}`, event_types: ["app_mention"] } });
    expect(registration.status()).toBe(200);
    const guestText = "@online-reply-agent Guest hello through the ordinary owner pipeline";
    let interruptResponse = true;
    await b.route("**/v1/online/groups/*/messages", async route => {
      if (route.request().method() === "POST" && interruptResponse) {
        interruptResponse = false;
        const accepted = await route.fetch();
        expect(accepted.ok()).toBeTruthy();
        await route.abort("failed");
      } else await route.continue();
    });
    await b.getByPlaceholder("Message Online shared acceptance...").fill(guestText);
    await b.getByRole("button", { name: "Send message", exact: true }).click();
    await expect(b.getByText("Message not sent", { exact: true })).toBeVisible();
    await expect(b.getByPlaceholder("Message Online shared acceptance...")).toHaveValue(guestText);
    await b.reload();
    await expect(b.getByPlaceholder("Message Online shared acceptance...")).toHaveValue(guestText);
    await b.getByRole("button", { name: "Send message", exact: true }).click();
    await expect.poll(async () => {
      const response = await a.request.get(`${API_BASE}/v1/conversations/${group.id}/messages?principal_id=${owner.principal.id}`, { headers: auth(owner.token) });
      return await response.text();
    }, { timeout: 30_000 }).toContain(guestText);
    await expect.poll(() => received, { timeout: 15_000 }).toContain(guestText);
    await expect(a.getByRole("tabpanel", { name: "Chat", exact: true }).getByText("Online Guest", { exact: true })).toBeVisible();
    const guestMessage = a.locator(".chat-primary .msg-group").filter({ hasText: guestText });
    await guestMessage.hover();
    await guestMessage.getByTitle("More actions", { exact: true }).click();
    await guestMessage.getByText("Reply in thread", { exact: true }).click();
    await expect(a.locator(".thread-panel").getByText("Online Guest", { exact: true })).toBeVisible();
    await expect(a.locator(".thread-panel .agent-badge")).toHaveCount(0);
    await a.locator(".thread-panel-close").click();
    await expect(b.locator(".chat-main .messages-area").getByText(guestText, { exact: true })).toBeVisible({ timeout: 30_000 });
    await b.getByPlaceholder("Message Online shared acceptance...").fill("Draft survives navigation");
    await b.getByRole("button", { name: "Actions menu" }).click();
    await b.getByRole("button", { name: "Online", exact: true }).click();
    await b.keyboard.press("Escape");
    await expect(b.getByPlaceholder("Message Online shared acceptance...")).toHaveValue("Draft survives navigation");
    // Account verification may be throttled; local history is not an auth probe.
    let verificationRequests = 0;
    await b.route("**/v1/online/session", async route => {
      if (route.request().method() !== "GET") return route.continue();
      verificationRequests++;
      await route.fulfill({ status: 429, headers: { "Retry-After": "60" }, json: { error: { detail: "Too many requests" } } });
    });
    await b.reload();
    await expect(b.getByRole("dialog")).toHaveCount(0);
    await expect(b.getByPlaceholder("Message Online shared acceptance...")).toHaveValue("Draft survives navigation");
    await b.waitForResponse(response => response.url().endsWith("/v1/online/groups") && response.ok(), { timeout: 30_000 });
    expect(verificationRequests).toBe(0);
    await b.getByPlaceholder("Message Online shared acceptance...").fill("");
    await expect(b.locator(".chat-main .messages-area").getByText(guestText, { exact: true })).toHaveCount(1);
    await b.screenshot({ path: test.info().outputPath("online-shared-group.png"), fullPage: true });
    expect((await a.request.delete(`${API_BASE}/v1/online/groups/${ownerLink.id}`, { headers: auth(owner.token) })).status()).toBe(204);
    await expect(b.getByPlaceholder("Message Online shared acceptance...")).toBeDisabled({ timeout: 20_000 });
    expect((await b.request.post(`${API_BASE}/v1/online/groups/${guestLink.id}/messages`, { headers: auth(guest.token), data: { id: randomUUID(), content: "Must be denied" } })).status()).toBe(409);
    await a.reload();
    await a.locator(".conv-item").filter({ hasText: "Online shared acceptance" }).first().click();
    await expect(a.getByRole("tabpanel", { name: "Chat", exact: true }).getByText("Online Guest", { exact: true })).toBeVisible();
    expect(received).toEqual([guestText]);
    for (const [i, local] of [owner, guest, outsider].entries()) await pages[i].request.delete(`${API_BASE}/v1/online/session`, { headers: auth(local.token) });
  } finally {
    await new Promise<void>((resolve, reject) => webhook.close(error => error ? reject(error) : resolve()));
    await Promise.all(contexts.map(context => context.close()));
  }
});

test("local-only dashboards stop polling unavailable Online groups", async ({ page }) => {
  await signup(page, `local-online-${randomUUID().slice(0, 16)}`, "local-test-password");
  await page.clock.install();
  let requests = 0;
  page.on("request", request => {
    if (new URL(request.url()).pathname === "/api/v1/online/groups") requests += 1;
  });
  const unavailable = page.waitForResponse(response =>
    new URL(response.url()).pathname === "/api/v1/online/groups" && response.status() === 403);
  await gotoDashboard(page);
  await unavailable;
  await expect(page.getByRole("button", { name: "Actions menu" })).toBeVisible();
  await page.clock.runFor(15_000);
  expect(requests).toBe(1);
});

test("Online sidebar resumes on account changes and preserves groups during bounded retries", async ({ page }) => {
  await signup(page, `poll-online-${randomUUID().slice(0, 16)}`, "local-test-password");
  await page.clock.install();
  let signedIn = false;
  let requests = 0;
  let failure = false;
  // Only this UI lifecycle scenario replaces Online responses; the neighboring
  // journeys exercise the real Worker, identity storage and encrypted groups.
  await page.route("**/api/v1/online/session", route => {
    if (route.request().method() === "DELETE") signedIn = false;
    return route.fulfill({ json: { state: signedIn ? "signed_in" : "signed_out" } });
  });
  await page.route("**/api/v1/online/sign-in", route => {
    signedIn = true;
    return route.fulfill({ json: { state: "signed_in", display_name: "Polling user" } });
  });
  await page.route("**/api/v1/online/groups", route => {
    requests += 1;
    if (!signedIn) return route.fulfill({ status: 403, json: { error: "Sign in to Online first" } });
    if (failure) return route.fulfill({ status: 429, headers: { "Retry-After": "10" }, json: { error: "Try later" } });
    return route.fulfill({ json: { groups: [{ id: "polling-link", role: "guest", status: "active", name: "Polling design group", conversation_id: "host-conversation" }] } });
  });
  await gotoDashboard(page);
  await expect.poll(() => requests).toBe(1);
  await page.getByRole("button", { name: "Actions menu" }).click();
  await page.getByRole("button", { name: "Online", exact: true }).click();
  await page.getByLabel("Email", { exact: true }).fill("poll@example.test");
  await page.getByLabel("Password", { exact: true }).fill("test-password");
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  const item = page.locator('[data-conversation-id="online:polling-link"]');
  await expect(item).toBeVisible();
  await page.getByRole("dialog").getByRole("button", { name: "Close", exact: true }).click();
  await expect.poll(() => requests).toBeGreaterThan(1);
  // Let the explicit dialog-close refresh settle before measuring its timer.
  await page.clock.runFor(0);
  failure = true;
  const throttled = page.waitForResponse(response => response.url().endsWith("/api/v1/online/groups") && response.status() === 429);
  await page.clock.runFor(3000);
  await throttled;
  const beforeRetry = requests;
  await page.clock.runFor(9000);
  expect(requests).toBe(beforeRetry);
  await expect(item).toBeVisible();
  failure = false;
  await page.clock.runFor(1000);
  await expect.poll(() => requests).toBe(beforeRetry + 1);
  await page.context().setOffline(true);
  const beforeOffline = requests;
  await page.clock.runFor(15_000);
  expect(requests).toBe(beforeOffline);
  await expect(item).toBeVisible();
  await page.context().setOffline(false);
  await expect.poll(() => requests).toBe(beforeOffline + 1);
  await page.evaluate(() => {
    Object.defineProperty(document, "visibilityState", { configurable: true, value: "hidden" });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  const beforeHidden = requests;
  await page.clock.runFor(15_000);
  expect(requests).toBe(beforeHidden);
  await page.evaluate(() => {
    delete (document as unknown as { visibilityState?: string }).visibilityState;
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await expect.poll(() => requests).toBe(beforeHidden + 1);
  await page.getByRole("button", { name: "Actions menu" }).click();
  await page.getByRole("button", { name: "Online", exact: true }).click();
  await page.getByRole("button", { name: "Sign out of Online" }).click();
  await expect(item).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Sign in", exact: true })).toBeVisible();
  const stopped = requests;
  await page.clock.runFor(15_000);
  expect(requests).toBe(stopped);
});

test("Online signs in through the real Worker, persists locally and revokes on sign-out", async ({ page }) => {
  test.setTimeout(120_000);
  const id = randomUUID();
  const local = await signup(page, `online-${id.slice(0, 20)}`, "local-test-password");
  const email = `${id}@example.test`;
  const password = `online-password-${id}`;
  await gotoDashboard(page);
  const open = async () => {
    await page.getByRole("button", { name: "Actions menu" }).click();
    await page.getByRole("button", { name: "Online", exact: true }).click();
  };
  await open();
  await page.getByRole("button", { name: "Create account", exact: true }).click();
  await page.getByLabel("Display name", { exact: true }).fill("Online Test Person");
  await page.getByLabel("Email", { exact: true }).fill(email);
  await submitOnline(page, "sign-up", password);
  await expect(page.getByRole("status")).toHaveText("Signed in as Online Test Person");
  await page.screenshot({ path: test.info().outputPath("online-signed-in.png"), fullPage: true });
  const headers = { Authorization: `Bearer ${local.token}` };
  const identity = await (await page.request.get(`${API_BASE}/v1/online/session`, { headers })).json();
  expect(identity.state).toBe("signed_in");
  expect(identity.device_id).toBeTruthy();
  expect(identity.session_token).toBeUndefined();
  expect(identity.token).toBeUndefined();
  await page.reload();
  await open();
  await expect(page.getByRole("status")).toHaveText("Signed in as Online Test Person");
  await page.getByRole("button", { name: "Sign out of Online" }).click();
  await expect(page.getByRole("button", { name: "Sign in", exact: true })).toBeVisible();
  expect(await (await page.request.get(`${API_BASE}/v1/online/session`, { headers })).json()).toEqual({ state: "signed_out" });
  await page.getByLabel("Email", { exact: true }).fill(email);
  await page.getByLabel("Password", { exact: true }).fill("incorrect-password");
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByRole("alert")).toBeVisible();
  await expect(page.getByLabel("Password", { exact: true })).toHaveValue("");
  await submitOnline(page, "sign-in", password);
  await expect(page.getByRole("status")).toHaveText("Signed in as Online Test Person");
  const storage = await page.evaluate(() => JSON.stringify({ ...localStorage }));
  expect(storage).not.toContain(password);
  const range = new URLSearchParams({ source: "audit", since: new Date(Date.now() - 60_000).toISOString(), until: new Date(Date.now() + 60_000).toISOString(), limit: "100" });
  const activityResponse = await page.request.get(`${API_BASE}/v1/activity?${range}`, { headers });
  expect(activityResponse.ok()).toBeTruthy();
  const activity = await activityResponse.text();
  expect(activity).toContain("online.signed_in");
  expect(activity).not.toContain(password);
  await page.getByRole("button", { name: "Sign out of Online" }).click();
});
