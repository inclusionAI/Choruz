import { expect, test } from "@playwright/test";
import { login, gotoDashboard, expandSidebarConversationSections, API_BASE } from "../fixtures/auth";
import { createCompany, createGroup, deleteCompany, sendMessage, uniqueName } from "../fixtures/api";
import { postgresQueryClient } from "../../lib/groups/group-provisioning-db";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { resolve } from "node:path";

test("the CLI exports every activity page and opt-in committed messages through the real API", async ({ page }) => {
  test.setTimeout(180_000);
  const exec = promisify(execFile);
  const root = resolve(process.cwd(), "../..");
  await exec("cargo", ["build", "-p", "choruz-cli"], { cwd: root, timeout: 120_000 });
  const { token, principal } = await login(page);
  const traceId = uniqueName("activity-export");
  const since = new Date(Date.now() - 60_000).toISOString();
  const until = new Date(Date.now() + 120_000).toISOString();
  const run = async (...args: string[]) => (await exec(resolve(root, "target/debug/choruz"), ["activity", ...args], {
    cwd: root, timeout: 30_000, env: { ...process.env, CHORUZ_API_BASE_URL: API_BASE, CHORUZ_SESSION_TOKEN: token },
  })).stdout;
  const company = await createCompany(page, token, principal.id, uniqueName("activity-export"));
  const db = await postgresQueryClient();
  try {
    for (let offset = 0; offset < 105; offset += 100) {
      const events = Array.from({ length: Math.min(100, 105 - offset) }, (_, index) => ({
        eventId: `${traceId}-${offset + index}`, schemaVersion: 1, traceId, spanId: "span", sessionId: "session",
        name: "ui_click", ts: new Date().toISOString(), durationMs: 10, data: { control: "export", password: "must-not-export" },
      }));
      const response = await page.request.post(`${API_BASE}/v1/telemetry`, { headers: { Authorization: `Bearer ${token}` }, data: { events } });
      expect(response.status()).toBe(204);
    }
    const filters = ["--since", since, "--until", until, "--trace-id", traceId];
    const firstPage = JSON.parse(await run("list", ...filters));
    expect(firstPage.records).toHaveLength(100);
    expect(typeof firstPage.next_cursor).toBe("string");
    const exported = await run("export", ...filters);
    const rows = exported.trim().split("\n").map(line => JSON.parse(line));
    expect(rows).toHaveLength(105);
    expect(new Set(rows.map(row => row.id)).size).toBe(105);
    expect(rows.every(row => row.trace_id === traceId)).toBe(true);
    expect(exported).not.toContain("must-not-export");
    const summary = JSON.parse(await run("summary", ...filters));
    expect(summary.groups).toEqual([{ name: "ui_click", count: 105, failed: 0, mean_duration_ms: 10 }]);
    expect(summary.truncated).toBe(false);
    const group = await createGroup(page, token, principal.id, uniqueName("export-messages"), [], company.id);
    const content = uniqueName("committed-export-content");
    await sendMessage(page, token, principal.id, group.id, content);
    const metadata = await run("messages", "--conversation", group.id);
    expect(metadata).not.toContain(content);
    const messages = (await run("messages", "--conversation", group.id, "--include-content")).trim().split("\n").map(line => JSON.parse(line));
    expect(messages.some(row => row.content === content)).toBe(true);
    const preview = JSON.parse(await run("prune", "--before", "1970-01-01T00:00:00Z"));
    expect(preview.applied).toBe(false);
    expect(preview.count).toBe(0);
  } finally {
    await db.query("DELETE FROM telemetry_event WHERE principal_id=$1 AND trace_id=$2", [principal.id, traceId]);
    await deleteCompany(page, token, company.id);
  }
});

test("activity listeners survive a client-side remount without copying clipboard contents", async ({ page }) => {
  const { token, principal } = await login(page);
  const company = await createCompany(page, token, principal.id, uniqueName("clipboard-activity"));
  const db = await postgresQueryClient();
  try {
    const group = await createGroup(page, token, principal.id, uniqueName("clipboard-group"), [], company.id);
    await page.goto("/docs");
    await page.evaluate(() => { document.documentElement.dataset.activityDocument = "same-document"; });
    await page.getByRole("link", { name: "Dashboard", exact: true }).click();
    await expect(page.locator(".company-selector-btn")).toBeVisible();
    await page.goBack();
    await expect(page.getByRole("link", { name: "Dashboard", exact: true })).toBeVisible();
    await page.goForward();
    await expect(page.locator(".company-selector-btn")).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.dataset.activityDocument)).toBe("same-document");
    await expandSidebarConversationSections(page);
    await page.locator(".company-selector-btn").click();
    await page.locator(".company-dropdown-item-name").filter({ hasText: company.name }).click();
    await page.locator(`[data-conversation-id="${group.id}"]`).click();
    const secret = uniqueName("private-clipboard");
    await page.locator("textarea").first().evaluate((element, value) => {
      const clipboardData = new DataTransfer();
      clipboardData.setData("text/plain", value);
      element.dispatchEvent(new ClipboardEvent("paste", { bubbles: true, clipboardData }));
    }, secret);
    const records = () => db.query("SELECT data FROM telemetry_event WHERE principal_id=$1 AND name='ui_paste' AND data->>'company_id'=$2", [principal.id, company.id]);
    await expect.poll(async () => (await records()).rowCount).toBe(1);
    const activity = await db.query("SELECT data FROM telemetry_event WHERE principal_id=$1 AND data->>'company_id'=$2", [principal.id, company.id]);
    expect(JSON.stringify(activity.rows)).not.toContain(secret);
  } finally { await deleteCompany(page, token, company.id); }
});

test("import choices are recorded as controls, not input values", async ({ page }) => {
  const { token, principal } = await login(page);
  const company = await createCompany(page, token, principal.id, uniqueName("choice-activity"));
  const db = await postgresQueryClient();
  try {
    await gotoDashboard(page);
    await page.locator(".company-selector-btn").click();
    await page.locator(".company-dropdown-item-name").filter({ hasText: company.name }).click();
    await page.getByRole("button", { name: "Actions menu", exact: true }).click();
    await page.getByText("Import Sessions", { exact: true }).click();
    const checkbox = page.locator('[data-activity="import_harness_claude"]');
    await checkbox.uncheck();
    await checkbox.check();
    const rows = () => db.query("SELECT data FROM telemetry_event WHERE principal_id=$1 AND name='ui_toggle' AND data->>'company_id'=$2 AND data->>'control'='import_harness_claude' ORDER BY occurred_at,id", [principal.id, company.id]);
    await expect.poll(async () => (await rows()).rowCount).toBe(2);
    expect((await rows()).rows.map(row => row.data.checked)).toEqual([false, true]);
  } finally { await deleteCompany(page, token, company.id); }
});

test("a user's send click is correlated with failure and retry without collecting the draft", async ({ page }) => {
  const { token, principal } = await login(page);
  const company = await createCompany(page, token, principal.id, uniqueName("activity-flow"));
  const db = await postgresQueryClient();
  try {
    const group = await createGroup(page, token, principal.id, uniqueName("flow-group"), [], company.id);
    await gotoDashboard(page);
    await page.locator(".company-selector-btn").click();
    await page.locator(".company-dropdown-item-name").filter({ hasText: company.name }).click();
    await page.locator(`[data-conversation-id="${group.id}"]`).click();
    const traceIds: string[] = [];
    await page.route("**/api/v1/messages", async route => {
      traceIds.push(route.request().headers()["x-trace-id"]);
      if (traceIds.length === 1) await route.fulfill({ status: 503, contentType: "application/json", body: JSON.stringify({ error: "temporary failure" }) });
      else await route.continue();
    });
    const draft = uniqueName("private-draft");
    const textarea = page.locator("textarea").first();
    await textarea.fill(draft);
    await page.getByTitle("Send message", { exact: true }).click();
    await expect(page.getByRole("alert").filter({ hasText: "Message not sent" })).toBeVisible();
    await expect(textarea).toHaveValue(draft);
    await page.getByTitle("Send message", { exact: true }).click();
    await expect(page.locator(".messages-area").getByText(draft, { exact: true })).toBeVisible();
    const records = () => db.query("SELECT trace_id, span_id, data FROM telemetry_event WHERE principal_id=$1 AND trace_id=ANY($2::text[]) AND name='http_request' AND data->>'resource'='/api/v1/messages' ORDER BY occurred_at, id", [principal.id, traceIds]);
    await expect.poll(async () => (await records()).rowCount).toBe(4);
    const rows = (await records()).rows;
    for (const [index, outcome] of ["failed", "succeeded"].entries()) {
      const pair = rows.filter(row => row.trace_id === traceIds[index]);
      expect(pair).toHaveLength(2);
      expect(pair[0].span_id).toBe(pair[1].span_id);
      expect(pair[0].data.phase).toBe("started");
      expect(pair[1].data).toMatchObject({ phase: "finished", outcome, company_id: company.id, conversation_id: group.id, status: index === 0 ? 503 : 201 });
    }
    const clicks = await db.query("SELECT data FROM telemetry_event WHERE principal_id=$1 AND trace_id=ANY($2::text[]) AND name='ui_click'", [principal.id, traceIds]);
    expect(clicks.rows).toHaveLength(2);
    expect(clicks.rows.every(row => row.data.label === "Send message")).toBe(true);
    const interactionsUrl = `${API_BASE}/v1/conversations/${group.id}/interactions`;
    const headers = { Authorization: `Bearer ${token}` };
    const summary = await page.request.get(interactionsUrl, { headers });
    expect(summary.ok()).toBe(true);
    const summaryRows = (await summary.json()).records;
    expect(summaryRows).toHaveLength(1);
    expect(summaryRows[0].content).toBeUndefined();
    expect(summaryRows[0].trace_id).toBe(traceIds[1]);
    const content = await page.request.get(`${interactionsUrl}?include_content=true`, { headers });
    expect(content.ok()).toBe(true);
    expect((await content.json()).records[0].content).toBe(draft);
    const activity = await db.query("SELECT data FROM telemetry_event WHERE principal_id=$1 AND data->>'company_id'=$2", [principal.id, company.id]);
    expect(JSON.stringify(activity.rows)).not.toContain(draft);
    const privateName = uniqueName("private-file") + ".txt";
    await page.locator(".chat-input-bar input[type=file]").setInputFiles({ name: privateName, mimeType: "text/plain", buffer: Buffer.from("private-file-bytes") });
    await page.getByRole("button", { name: `Remove ${privateName}`, exact: true }).click();
    await expect.poll(async () => (await db.query("SELECT data FROM telemetry_event WHERE principal_id=$1 AND name='ui_files_selected' AND data->>'company_id'=$2", [principal.id, company.id])).rows.map(row => row.data.count)).toEqual([1]);
    const withFile = await db.query("SELECT data FROM telemetry_event WHERE principal_id=$1 AND data->>'company_id'=$2", [principal.id, company.id]);
    expect(JSON.stringify(withFile.rows)).not.toContain(privateName);
    expect(JSON.stringify(withFile.rows)).not.toContain("private-file-bytes");
  } finally { await deleteCompany(page, token, company.id); }
});

test("activity survives lost acknowledgement and reload without duplicate persistence", async ({ page }) => {
  const { token, principal } = await login(page);
  const company = await createCompany(page, token, principal.id, uniqueName("activity"));
  const db = await postgresQueryClient();
  try {
    const group = await createGroup(page, token, principal.id, uniqueName("activity-group"), [], company.id);
    let captured: { eventId: string; ts: string; name: string } | undefined;
    let body: unknown;
    await page.route("**/api/v1/telemetry", async route => {
      const payload = route.request().postDataJSON();
      const owned = payload.events.find((event: { name: string; data?: { conversation_id?: string } }) => event.name === "switch_conversation" && event.data?.conversation_id === group.id);
      if (!captured && owned) {
        body = payload;
        // The production gateway commits, but the browser loses its acknowledgement.
        const response = await route.fetch();
        expect(response.status()).toBe(204);
        await route.abort();
        captured = owned;
      } else await route.continue();
    });
    await gotoDashboard(page);
    await page.locator(".company-selector-btn").click();
    await page.locator(".company-dropdown-item-name").filter({ hasText: company.name }).click();
    await page.locator('[data-conversation-id="' + group.id + '"]').click();
    await expect.poll(() => captured?.eventId).toBeTruthy();
    await page.reload();
    const records = () => db.query("SELECT occurred_at, workspace_id FROM telemetry_event WHERE principal_id=$1 AND event_id=$2", [principal.id, captured!.eventId]);
    await expect.poll(async () => (await records()).rowCount).toBe(1);
    const replay = await page.request.post(API_BASE + "/v1/telemetry", { headers: { Authorization: "Bearer " + token }, data: body });
    expect(replay.status()).toBe(204);
    const result = await records();
    expect(result.rows).toHaveLength(1);
    expect(result.rows[0].workspace_id).toBe(principal.workspace_id);
    expect(new Date(result.rows[0].occurred_at).toISOString()).toBe(captured!.ts);
    await expect.poll(async () => page.evaluate(async eventId => {
      const db = await new Promise<IDBDatabase>((resolve, reject) => {
        const request = indexedDB.open("choruz-activity");
        request.onsuccess = () => resolve(request.result); request.onerror = () => reject(request.error);
      });
      try { return await new Promise<boolean>((resolve, reject) => {
        const request = db.transaction("pending").objectStore("pending").get(eventId);
        request.onsuccess = () => resolve(request.result === undefined); request.onerror = () => reject(request.error);
      }); } finally { db.close(); }
    }, captured!.eventId)).toBe(true);
  } finally { await deleteCompany(page, token, company.id); }
});
