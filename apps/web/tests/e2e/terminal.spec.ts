import { expect, test, request } from "@playwright/test";
import { randomUUID } from "node:crypto";
import { mkdir, writeFile, readFile, realpath } from "node:fs/promises";
import path from "node:path";
import { postgresQueryClient } from "../../lib/groups/group-provisioning-db";
import { API_BASE, WEB_BASE, login, gotoDashboard } from "../fixtures/auth";
import { createGroup, provisionAgent, sendMessage, uniqueName } from "../fixtures/api";
import { runtimeTest } from "../fixtures/runtime-device";

for (const driver of ["claude_terminal", "codex_terminal"]) {
  for (const remote of [false, true]) {
    runtimeTest(`structured ${driver} on ${remote ? "device B" : "this computer"} preserves approvals across reload`, async ({ page, device }) => {
      const { company, host, headers, home } = device;
      const name = uniqueName("structured-dm");
      const fixture = path.join(home, `structured-${driver}`);
      await writeFile(fixture, await readFile(path.resolve("tests/fixtures/structured-cli.py")), { mode: 0o700 });
      const created = await page.request.post(`${WEB_BASE}/api/agents/provision`, { data: {
        name, instructions: "Verify the selected device workspace.", driver_type: driver,
        workspace_id: company.id, ...(remote ? { runtime_host_id: host.id } : {}),
      } });
      expect(created.status(), await created.text()).toBe(201);
      const agent = await created.json();
      const db = await postgresQueryClient();
      await db.query("UPDATE agent_runtime_bindings SET config_json = (config_json - 'model') || $1::jsonb WHERE id=$2", [JSON.stringify({ binary_path: fixture }), agent.binding.id]);
      await gotoDashboard(page);
      await page.getByRole("button", { name: "Select company" }).click();
      await page.locator(".company-dropdown-item").filter({ hasText: company.name }).locator(".company-dropdown-item-name").click();
      const open = async () => {
        const conversation = page.locator(".conv-item").filter({ hasText: name }).first();
        await expect(conversation).toBeVisible();
        await conversation.click();
        await expect(page.getByRole("region", { name: "Agent session" })).toBeVisible();
      };
      await open();
      const session = page.getByRole("region", { name: "Agent session" });
      await session.getByLabel("Message Agent").fill("Inspect the workspace");
      await expect(session.getByRole("button", { name: "Send", exact: true })).toBeEnabled();
      await session.getByRole("button", { name: "Send", exact: true }).click();
      await expect(session.getByRole("button", { name: "Allow once" })).toBeVisible();
      await expect(readFile(path.join(agent.workspace_path, "approved-on-device"))).rejects.toThrow();
      await page.reload();
      await open();
      await session.getByRole("button", { name: "Allow once" }).click();
      await expect(session.getByText("Verified workspace on selected device", { exact: true })).toBeVisible();
      await expect.poll(() => readFile(path.join(agent.workspace_path, "approved-on-device"), "utf8").catch(() => "")).toBe(agent.workspace_path);
      await session.locator("details summary").first().click();
      await expect(session.locator("details pre").first()).toContainText(agent.workspace_path);
      await session.getByLabel("Message Agent").fill("wait");
      await session.getByRole("button", { name: "Send", exact: true }).click();
      await session.getByRole("button", { name: "Stop", exact: true }).click();
      await expect(session.getByRole("status")).toHaveText("ready");
      await expect(session.getByRole("button", { name: "Stop", exact: true })).toHaveCount(0);
      await expect(session.getByRole("button", { name: "Reconnect", exact: true })).toHaveCount(0);
      const reopened = page.waitForResponse((response) =>
        new URL(response.url()).pathname.endsWith(`/runtime/bindings/${agent.binding.id}/session`)
        && response.request().method() === "POST",
      );
      await page.reload();
      await open();
      // The region mounts before its reopen request returns. Verify replay
      // after that owned request, not against the temporary connecting view.
      expect((await reopened).ok()).toBeTruthy();
      await expect(session.getByText("Verified workspace on selected device", { exact: true })).toHaveCount(1);
      const beforeTerminal = await (await page.request.get(`${API_BASE}/v1/runtime/bindings/${agent.binding.id}/session`, { headers })).json();
      await session.getByRole("button", { name:"Terminal", exact:true }).click();
      await expect(session.locator(".terminal-container > .xterm")).toHaveCount(1);
      await session.getByRole("button", { name:"Conversation", exact:true }).click();
      await expect(session.getByRole("status")).toHaveText("ready");
      await expect(session.getByText("Verified workspace on selected device", { exact: true })).toHaveCount(1);
      const afterTerminal = await (await page.request.get(`${API_BASE}/v1/runtime/bindings/${agent.binding.id}/session`, { headers })).json();
      expect(afterTerminal.session_id).toBe(beforeTerminal.session_id);
      const anonymous = await request.newContext();
      try {
        const unauthorized = await anonymous.get(`${API_BASE}/v1/runtime/bindings/${agent.binding.id}/session`);
        expect(unauthorized.status()).toBe(401);
      } finally {
        await anonymous.dispose();
      }
      const current = await (await page.request.get(`${API_BASE}/v1/runtime/bindings/${agent.binding.id}/session`, { headers })).json();
      const stale = await page.request.post(`${API_BASE}/v1/runtime/bindings/${agent.binding.id}/session/commands`, { headers, data: { action: "send", instance: randomUUID(), text: "stale request", submission_id: randomUUID() } });
      expect(stale.status()).toBe(409);
      const closed = await page.request.post(`${API_BASE}/v1/runtime/bindings/${agent.binding.id}/session/commands`, { headers, data: { action: "close", instance: current.instance } });
      expect(closed.ok(), await closed.text()).toBe(true);
    });
  }
}

runtimeTest("remote terminal activity records outcomes and byte counts without content", async ({ page, device }) => {
  const { home, company, host, headers, session } = device;
  await writeFile(path.join(home, "grok-target"), '#!/bin/sh\nexec /bin/cat\n', { mode: 0o700 });
  const response = await page.request.post(`${WEB_BASE}/api/agents/provision`, { data: {
    name: uniqueName("terminal-activity"), instructions: "Terminal transport fixture.",
    driver_type: "grok_terminal", workspace_id: company.id, runtime_host_id: host.id,
  } });
  expect(response.status(), await response.text()).toBe(201);
  const bindingId = (await response.json()).binding.id;
  const privateInput = uniqueName("not-for-activity-storage");
  const traceId = randomUUID();
  const inputHeaders = { ...headers, "x-trace-id": traceId };
  const inputUrl = `${API_BASE}/v1/terminals/${bindingId}/input`;
  // No PTY exists yet: this must be recorded as a failed attempt, not success.
  const failed = await page.request.post(inputUrl, { headers: inputHeaders, data: { data: privateInput } });
  expect(failed.ok()).toBe(false);
  expect((await page.request.post(`${API_BASE}/v1/terminals/${bindingId}/ensure`, { headers })).ok()).toBe(true);
  expect((await page.request.post(inputUrl, { headers: inputHeaders, data: { data: privateInput } })).ok()).toBe(true);
  const marker = uniqueName("terminal-wire");
  const url = `${API_BASE.replace(/^http/, "ws")}/v1/ws/terminals/${bindingId}?token=${encodeURIComponent(session.token)}`;
  await page.evaluate(({ url, marker }) => new Promise<void>((resolve, reject) => {
    const socket = new WebSocket(url);
    socket.binaryType = "arraybuffer";
    let received = "";
    const timer = setTimeout(() => { socket.close(); reject(new Error("terminal echo not received")); }, 10000);
    socket.onopen = () => {
      socket.send(JSON.stringify({ type: "resize", cols: 90, rows: 30 }));
      socket.send(marker + "\r");
    };
    socket.onmessage = event => {
      received += new TextDecoder().decode(event.data);
      if (received.includes(marker)) socket.close();
    };
    socket.onerror = () => { clearTimeout(timer); reject(new Error("terminal socket failed")); };
    socket.onclose = () => { clearTimeout(timer); received.includes(marker) ? resolve() : reject(new Error("terminal closed before echo")); };
  }), { url, marker });
  const db = await postgresQueryClient();
  const records = () => db.query("SELECT action,metadata FROM audit_log WHERE actor_id=$1 AND target_id=$2 AND action LIKE 'terminal.%' ORDER BY created_at,id", [session.principal.id, bindingId]);
  await expect.poll(async () => (await records()).rows.filter(row => row.action === "terminal.detached").length).toBe(1);
  const rows = (await records()).rows;
  const finished = rows.filter(row => row.action === "terminal.submit_finished");
  expect(finished.map(row => row.metadata.outcome)).toEqual(["failed", "succeeded"]);
  for (const result of finished) {
    expect(result.metadata.trace_id).toBe(traceId);
    expect(rows.filter(row => row.action === "terminal.submit_started" && row.metadata.submission_id === result.metadata.submission_id)).toHaveLength(1);
  }
  const attached = rows.find(row => row.action === "terminal.attached")!;
  const detached = rows.find(row => row.action === "terminal.detached")!;
  expect(detached.metadata.attachment_id).toBe(attached.metadata.attachment_id);
  expect(detached.metadata.runtime_host_id).toBe(host.id);
  expect(detached.metadata.company_id).toBe(company.id);
  expect(detached.metadata.resize_count).toBe(1);
  expect(detached.metadata.input_bytes).toBe(Buffer.byteLength(marker + "\r"));
  expect(detached.metadata.output_bytes).toBeGreaterThan(0);
  expect(JSON.stringify(rows)).not.toContain(privateInput);
  expect(JSON.stringify(rows)).not.toContain(marker);
});

runtimeTest("remote terminals resolve the target executable and preserve explicit target paths", async ({ page, device }) => {
  const { home, company, host, headers } = device;
  const script = (marker: string) => `#!/bin/sh\nprintf '%s' '${marker}' > "$PWD/observed-binary"\nexec /bin/cat\n`;
  await writeFile(path.join(home, "bin", "grok"), script("wrong-path-command"), { mode: 0o700 });
  await writeFile(path.join(home, "grok-target"), script("target-environment"), { mode: 0o700 });
  const explicit = path.join(home, "grok-explicit");
  await writeFile(explicit, script("explicit-target"), { mode: 0o700 });
  const db = await postgresQueryClient();
  for (const mode of ["automatic", "explicit", "missing"]) {
    const response = await page.request.post(`${WEB_BASE}/api/agents/provision`, { data: { name: uniqueName(`binary-${mode}`), instructions: "Verify the target executable.", driver_type: "grok_terminal", workspace_id: company.id, runtime_host_id: host.id } });
    expect(response.status(), await response.text()).toBe(201);
    const created = await response.json();
    const stored = await db.query<{ config_json: Record<string, unknown> }>("SELECT config_json FROM agent_runtime_bindings WHERE id = $1", [created.binding.id]);
    expect(stored.rows[0].config_json.binary_path).toBeUndefined();
    if (mode !== "automatic") {
      await db.query("UPDATE agent_runtime_bindings SET config_json = config_json || $1::jsonb WHERE id = $2", [JSON.stringify({ binary_path: mode === "explicit" ? explicit : path.join(home, "not-installed") }), created.binding.id]);
    }
    const ensured = await page.request.post(`${API_BASE}/v1/terminals/${created.binding.id}/ensure`, { headers });
    if (mode === "missing") {
      expect(ensured.status()).toBe(500);
      await expect(readFile(path.join(created.workspace_path, "observed-binary"), "utf8")).rejects.toThrow();
    } else {
      expect(ensured.ok(), await ensured.text()).toBeTruthy();
      await expect.poll(() => readFile(path.join(created.workspace_path, "observed-binary"), "utf8").catch(() => "")).toBe(mode === "explicit" ? "explicit-target" : "target-environment");
    }
  }

  const workspace = path.join(home, "imported");
  const catalog = path.join(home, ".grok", "sessions", "project", "session");
  await mkdir(workspace);
  await mkdir(catalog, { recursive: true });
  const sessionId = randomUUID();
  await writeFile(path.join(catalog, "summary.json"), JSON.stringify({ info: { id: sessionId, cwd: workspace, title: "Imported target" } }));
  const imported = await page.request.post(`${API_BASE}/v1/workspace-sessions/import`, { headers, data: { company_id: company.id, runtime_host_id: host.id, workspace_path: workspace, sessions: [{ harness: "grok", native_session_id: sessionId, workspace_path: workspace }] } });
  expect(imported.ok(), await imported.text()).toBeTruthy();
  const bindingId = (await imported.json()).imported[0].binding_id;
  const binding = await db.query<{ config_json: Record<string, unknown> }>("SELECT config_json FROM agent_runtime_bindings WHERE id = $1", [bindingId]);
  expect(binding.rows[0].config_json.binary_path).toBeUndefined();
  const ensured = await page.request.post(`${API_BASE}/v1/terminals/${bindingId}/ensure`, { headers });
  expect(ensured.ok(), await ensured.text()).toBeTruthy();
  await expect.poll(() => readFile(path.join(workspace, "observed-binary"), "utf8").catch(() => "")).toBe("target-environment");
});

runtimeTest("remote Codex terminal uses the selected account on its own device", async ({ page, device }) => {
  test.setTimeout(90_000);
  const { session, headers, company, home, host } = device;
    const binary = path.join(home, "codex");
    await writeFile(binary, '#!/bin/sh\ncat "$CODEX_HOME/auth.json" > "$PWD/observed-auth"\nprintf "Account loaded\\n"\nexec cat\n', { mode: 0o700 });

    for (const profile of ["default", "isolated"]) {
      const accountId = randomUUID();
      const source = profile === "default" ? path.join(home, ".codex") : path.join(home, "accounts", accountId, "codex");
      const marker = `device-b-${profile}`;
      await mkdir(source, { recursive: true });
      await writeFile(path.join(source, "auth.json"), marker);
      const agent = await provisionAgent(page, session.token, uniqueName(`codex-${profile}`), { workspaceId: company.id });
      const workspace = path.join(home, profile);
      await mkdir(workspace);
      // Seed a selected binding; the contract starts at terminal ensure, not login
      // or provisioning. Only the external CLI is substituted below.
      const db = await postgresQueryClient();
      await db.query("INSERT INTO harness_account (id, company_id, runtime_host_id, driver_type, name, profile_kind, status) VALUES ($1, $2, $3, 'codex_terminal', $4, $4, 'active')", [accountId, company.id, host.id, profile]);
      const rows = await db.query("UPDATE agent_runtime_bindings SET driver_type = 'codex_terminal', workspace_path = $1, external_session_id = NULL, config_json = (config_json - 'model') || $2::jsonb WHERE agent_principal_id = $3 RETURNING id", [workspace, JSON.stringify({ runtime_host_id: host.id, binary_path: binary, harness_account_id: accountId, harness_account_profile_kind: profile }), agent.agentId]);
      expect(rows.rows).toHaveLength(1);
      const bindingId = rows.rows[0].id;
      const ensured = await page.request.post(`${API_BASE}/v1/terminals/${bindingId}/ensure`, { headers });
      expect(ensured.ok(), await ensured.text()).toBeTruthy();
      await expect.poll(() => readFile(path.join(workspace, "observed-auth"), "utf8").catch(() => "")).toBe(marker);
      expect(await realpath(path.join(home, "runtime", "codex-homes", String(bindingId), "auth.json"))).toBe(path.join(source, "auth.json"));
    }
});

test.describe("Terminal view (PTY)", () => {
  let terminalAgentName: string;
  let terminalAgentId: string;
  let terminalBindingId: string;

  test.beforeEach(async ({ page }) => {
    await login(page);
    terminalAgentName = uniqueName("terminal-agent");
    const agent = await provisionAgent(page, "", terminalAgentName);
    terminalAgentId = agent.agentId;
    const binary = path.join(agent.workspacePath, "fixture-cli");
    await writeFile(binary, await readFile(path.resolve("tests/fixtures/structured-cli.py")), {mode:0o700});
    const db = await postgresQueryClient();
    const bindings = await db.query("UPDATE agent_runtime_bindings SET config_json=(config_json - 'model') || $1::jsonb WHERE agent_principal_id=$2 RETURNING id", [JSON.stringify({binary_path:binary}), agent.agentId]);
    expect(bindings.rows).toHaveLength(1);
    terminalBindingId = bindings.rows[0].id;
    await gotoDashboard(page);
  });

  /* ---------------------------------------------------------------------- */
  /*  Helpers                                                                */
  /* ---------------------------------------------------------------------- */

  async function selectDirectAgentConv(
    page: import("@playwright/test").Page,
  ): Promise<void> {
    const item = page.locator(".conv-item").filter({ hasText: terminalAgentName }).first();
    await expect(item).toBeVisible({ timeout: 15_000 });
    const ready = page.waitForResponse(async response =>
      new URL(response.url()).pathname.endsWith(`/runtime/bindings/${terminalBindingId}/session`)
      && response.ok() && (await response.json()).status === "ready");
    await item.click();
    await ready;
    await expect(page.getByRole("region", {name:"Agent session"}).getByRole("status")).toHaveText("ready");
    await page.getByRole("button", {name:"Terminal", exact:true}).click();
    const term = page.locator(".terminal-container, .xterm, .xterm-screen").first();
    await expect(term).toBeVisible();
  }

  /* ---------------------------------------------------------------------- */
  /*  Terminal rendering                                                     */
  /* ---------------------------------------------------------------------- */

  test("should render terminal for direct agent conversations", async ({
    page,
  }) => {
    await selectDirectAgentConv(page);
    const term = page.locator(".terminal-container, .xterm, .xterm-screen");
    await expect(term.first()).toBeVisible();
  });

  test("fits the terminal screen to the conversation pane", async ({ page }) => {
    await selectDirectAgentConv(page);
    await expect.poll(async () => {
      const screen = await page.locator(".xterm-screen").boundingBox();
      const pane = await page.locator(".terminal-container").boundingBox();
      return screen && pane ? screen.width / pane.width : 0;
    }).toBeGreaterThan(0.9);
  });

  test("uses the light terminal palette on the light page", async ({ page }) => {
    await selectDirectAgentConv(page);
    const screenshot = await page.locator(".xterm-screen").screenshot();
    const rgb = await page.evaluate(async (bytes) => {
      const image = await createImageBitmap(new Blob([new Uint8Array(bytes)], {type:"image/png"}));
      const canvas = new OffscreenCanvas(image.width, image.height);
      const context = canvas.getContext("2d")!;
      context.drawImage(image, 0, 0);
      return Array.from(context.getImageData(Math.floor(image.width / 2), Math.floor(image.height / 2), 1, 1).data).slice(0, 3);
    }, [...screenshot]);
    expect(rgb).toEqual([250, 249, 245]);
  });

  /* ---------------------------------------------------------------------- */
  /*  WebSocket connection                                                   */
  /* ---------------------------------------------------------------------- */

  test("should attempt WebSocket connection for terminal", async ({
    page,
  }) => {
    const frames: string[] = [];
    page.on("websocket", (socket) => {
      if (socket.url().includes("/ws/terminals")) socket.on("framereceived", ({payload}) => frames.push(payload.toString()));
    });

    await selectDirectAgentConv(page);
    await expect.poll(() => frames.some((frame) => frame.includes("Raw terminal connected"))).toBe(true);
  });

  /* ---------------------------------------------------------------------- */
  /*  Reconnection                                                           */
  /* ---------------------------------------------------------------------- */

  test("reconnects the terminal after a dropped socket", async ({
    page,
  }) => {
    const sockets: string[] = [];
    page.on("websocket", (socket) => {
      if (socket.url().includes("/ws/terminals/")) sockets.push(socket.url());
    });
    await page.evaluate(() => {
      const NativeSocket = window.WebSocket;
      window.WebSocket = class extends NativeSocket {
        constructor(url: string | URL, protocols?: string | string[]) {
          super(url, protocols);
          if (String(url).includes("/ws/terminals/")) {
            (window as unknown as { closeTestTerminal: () => void }).closeTestTerminal = () => this.close(1000, "test disconnect");
          }
        }
      };
    });
    await selectDirectAgentConv(page);
    await expect.poll(() => sockets.length).toBe(1);
    await page.evaluate(() => (window as unknown as { closeTestTerminal: () => void }).closeTestTerminal());
    await expect.poll(() => sockets.length, {timeout:10000}).toBeGreaterThan(1);
  });

  /* ---------------------------------------------------------------------- */
  /*  Terminal focus                                                          */
  /* ---------------------------------------------------------------------- */

  test("should focus terminal on click", async ({ page }) => {
    await selectDirectAgentConv(page);
    const terminal = page.locator(".xterm, .terminal-container").first();
    await terminal.click();
    // Terminal should receive focus
    const hasFocus = await page.evaluate(() => {
      const active = document.activeElement;
      return active?.closest(".xterm") !== null ||
        active?.closest(".terminal-container") !== null;
    });
    expect(hasFocus).toBe(true);
  });

  /* ---------------------------------------------------------------------- */
  /*  No console errors                                                      */
  /* ---------------------------------------------------------------------- */

  test("should not produce terminal-related console errors", async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));

    await selectDirectAgentConv(page);
    await page.waitForTimeout(3000);
    const termErrors = errors.filter(
      (e) =>
        e.includes("xterm") ||
        e.includes("Terminal") ||
        e.includes("WebSocket"),
    );
    // Some WebSocket errors are expected if the backend terminal is not running
    // But there should be no xterm initialization errors
    const initErrors = termErrors.filter(
      (e) =>
        e.includes("Cannot read") ||
        e.includes("undefined") ||
        e.includes("is not a function"),
    );
    expect(initErrors).toHaveLength(0);
  });

  /* ---------------------------------------------------------------------- */
  /*  Terminal not shown for group chats                                     */
  /* ---------------------------------------------------------------------- */

  test("shows PTY for direct terminal agent chats and composer for groups with that agent", async ({
    page,
  }) => {
    const { token, principal } = await login(page);
    const groupName = uniqueName("terminal-group");
    const group = await createGroup(page, token, principal.id, groupName, [terminalAgentId]);
    const groupMessage = `normal transcript marker ${Date.now()}`;
    await sendMessage(page, token, principal.id, group.id, groupMessage);
    await gotoDashboard(page);

    const directItem = page.locator(".conv-item").filter({ hasText: terminalAgentName }).first();
    await expect(directItem).toBeVisible({ timeout: 15_000 });
    await selectDirectAgentConv(page);
    await expect(page.locator(".terminal-container:visible").first()).toBeVisible({
      timeout: 10_000,
    });
    await expect(page.locator(".xterm:visible, .xterm-screen:visible").first()).toBeVisible({
      timeout: 10_000,
    });
    await expect(page.locator(".terminal-container:visible > .xterm")).toHaveCount(1);
    await expect(page.locator(".chat-input-row textarea").first()).toBeHidden();

    const groupItem = page.locator(".conv-item").filter({ hasText: group.name }).first();
    await expect(groupItem).toBeVisible({ timeout: 15_000 });
    await groupItem.click();
    const messagesArea = page.locator(".messages-area").first();
    await expect(messagesArea).toBeVisible({ timeout: 10_000 });
    await expect(messagesArea.getByText(groupMessage)).toBeVisible({ timeout: 10_000 });
    await expect(page.locator(".chat-input-row textarea").first()).toBeVisible({
      timeout: 10_000,
    });
    await expect(page.locator(".terminal-container:visible")).toHaveCount(0);
  });
});
