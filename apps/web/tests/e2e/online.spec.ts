import { randomUUID } from "node:crypto";
import { createServer } from "node:http";
import { expect, test, type Page } from "@playwright/test";
import { API_BASE, gotoDashboard, signup } from "../fixtures/auth";
import { createAgent } from "../fixtures/api";

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
    await expect(b.getByLabel("Message to shared group")).toBeEnabled({ timeout: 30_000 });
    const guestLinksResponse = await b.request.get(`${API_BASE}/v1/online/groups`, { headers: auth(guest.token) });
    const guestLink = (await guestLinksResponse.json()).groups[0];
    const ownerLink = (await (await a.request.get(`${API_BASE}/v1/online/groups`, { headers: auth(owner.token) })).json()).groups[0];
    expect((await c.request.get(`${API_BASE}/v1/online/groups/${guestLink.id}/messages`, { headers: auth(outsider.token) })).status()).toBe(404);
    expect((await c.request.post(`${API_BASE}/v1/online/groups/join`, { headers: auth(outsider.token), data: { invitation } })).status()).toBe(400);
    const privateRead = await b.request.get(`${API_BASE}/v1/conversations/${group.id}/messages?principal_id=${guest.principal.id}`, { headers: auth(guest.token) });
    expect([403, 404]).toContain(privateRead.status());
    const ownerText = "Owner hello across the encrypted group link";
    expect((await a.request.post(`${API_BASE}/v1/messages`, { headers: auth(owner.token), data: { actor_id: owner.principal.id, conversation_id: group.id, content: ownerText, content_type: "text", metadata: {}, idempotency_key: randomUUID() } })).ok()).toBeTruthy();
    await expect(b.getByLabel("Shared messages").getByText(ownerText, { exact: true })).toBeVisible({ timeout: 30_000 });
    await expect(b.getByLabel("Shared messages").getByText("Online Owner", { exact: true })).toBeVisible();
    await expect(b.getByLabel("Shared messages").getByText("AI", { exact: true })).toHaveCount(0);
    const agentText = "Agent reply through the authenticated message API";
    expect((await a.request.post(`${API_BASE}/v1/messages`, { headers: auth(agent.secret), data: { actor_id: agent.principal.id, conversation_id: group.id, content: agentText, content_type: "text", metadata: {}, idempotency_key: randomUUID() } })).ok()).toBeTruthy();
    await expect(b.getByLabel("Shared messages").getByText(agentText, { exact: true })).toBeVisible({ timeout: 30_000 });
    await expect(b.getByLabel("Shared messages").getByText("AI", { exact: true })).toHaveCount(1);
    const registration = await a.request.post(`${API_BASE}/v1/principals/${agent.principal.id}/event-webhook`, { headers: auth(agent.secret), data: { actor_id: agent.principal.id, url: `http://127.0.0.1:${address.port}`, event_types: ["app_mention"] } });
    expect(registration.status()).toBe(200);
    const guestText = "@online-reply-agent Guest hello through the ordinary owner pipeline";
    await b.getByLabel("Message to shared group").fill(guestText);
    await b.getByRole("button", { name: "Send to group", exact: true }).click();
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
    await expect(b.getByLabel("Shared messages").getByText(guestText, { exact: true })).toBeVisible({ timeout: 30_000 });
    await b.reload(); await open(b);
    await b.getByRole("button", { name: "Open group", exact: true }).click();
    await expect(b.getByLabel("Shared messages").getByText(guestText, { exact: true })).toHaveCount(1);
    await b.screenshot({ path: test.info().outputPath("online-shared-group.png"), fullPage: true });
    expect((await a.request.delete(`${API_BASE}/v1/online/groups/${ownerLink.id}`, { headers: auth(owner.token) })).status()).toBe(204);
    await expect(b.getByLabel("Message to shared group")).toBeDisabled({ timeout: 20_000 });
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
