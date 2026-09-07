import { expect, test } from "@playwright/test";
import { randomUUID } from "node:crypto";
import { mkdtemp, mkdir, writeFile, readFile, rm } from "node:fs/promises";
import path, { join } from "node:path";
import { homedir } from "node:os";
import { createCompany, createGroup, deleteCompany } from "../fixtures/api";
import { DatabaseSync } from "node:sqlite";
import { postgresQueryClient } from "../../lib/groups/group-provisioning-db";
import { runtimeTest } from "../fixtures/runtime-device";

import { API_BASE, gotoDashboard, login } from "../fixtures/auth";

runtimeTest("imports and resumes default and isolated account sessions on the selected device", async ({ page, device }) => {
  test.setTimeout(90_000);
  const { home, company, host, headers } = device;
  const workspace = path.join(home, "project");
  await mkdir(workspace);
  const db = await postgresQueryClient();
  const fixture = path.join(home, "structured-cli");
  await writeFile(fixture, await readFile(path.resolve("tests/fixtures/structured-cli.py")), { mode: 0o700 });
  const selected = [] as Array<{ harness: string; native_session_id: string; workspace_path: string; harness_account_id: string | null; marker: string }>;
  for (const harness of ["claude", "codex"]) {
    const sessionId = randomUUID();
    const binary = path.join(home, "bin", harness);
    const profileEnv = harness === "claude" ? '${CLAUDE_CONFIG_DIR}' : '${CODEX_HOME}';
    const markerFile = harness === "claude" ? "auth-marker" : "auth.json";
    await writeFile(binary, `#!/bin/sh\nmarker=$(cat "${profileEnv}/${markerFile}")\nprintf '%s\\n' "$@" > "$PWD/observed-$marker"\nexec "${fixture}" "$@"\n`, { mode: 0o700 });
    for (const profile of ["default", "isolated"]) {
      const accountId = profile === "isolated" ? randomUUID() : null;
      const root = accountId ? path.join(home, "accounts", accountId, harness) : path.join(home, `.${harness}`);
      const marker = `device-b-${harness}-${profile}`;
      await mkdir(root, { recursive: true });
      await writeFile(path.join(root, markerFile), marker);
      if (accountId) {
        await db.query("INSERT INTO harness_account (id, company_id, runtime_host_id, driver_type, name, profile_kind, status) VALUES ($1, $2, $3, $4, $5, 'isolated', 'active')", [accountId, company.id, host.id, `${harness}_terminal`, marker]);
      }
      if (harness === "claude") {
        const projects = path.join(root, "projects", "project");
        await mkdir(projects, { recursive: true });
        await writeFile(path.join(projects, `${sessionId}.jsonl`), JSON.stringify({ cwd: workspace, customTitle: marker }) + "\n");
      } else {
        const sqlite = new DatabaseSync(path.join(root, "state_5.sqlite"));
        try {
          sqlite.exec("CREATE TABLE threads (id TEXT, title TEXT, cwd TEXT, updated_at INTEGER, model_provider TEXT, git_branch TEXT, archived INTEGER, model TEXT)");
          sqlite.prepare("INSERT INTO threads VALUES (?, ?, ?, ?, 'openai', NULL, 0, NULL)").run(sessionId, marker, workspace, 1700000000);
        } finally { sqlite.close(); }
        await mkdir(path.join(root, "sessions"));
        await writeFile(path.join(root, "sessions", `${sessionId}.jsonl`), JSON.stringify({ type: "session_meta", payload: { id: sessionId, cwd: workspace } }) + "\n" + JSON.stringify({ marker }) + "\n");
      }
      selected.push({ harness, native_session_id: sessionId, workspace_path: workspace, harness_account_id: accountId, marker });
    }
  }

  const inactiveId = randomUUID();
  const inactiveProjects = path.join(home, "accounts", inactiveId, "claude", "projects", "project");
  await mkdir(inactiveProjects, { recursive: true });
  await writeFile(path.join(inactiveProjects, `${randomUUID()}.jsonl`), JSON.stringify({ cwd: workspace, customTitle: "Needs sign-in" }) + "\n");
  await db.query("INSERT INTO harness_account (id, company_id, runtime_host_id, driver_type, name, profile_kind, status) VALUES ($1, $2, $3, 'claude_terminal', 'Needs sign-in', 'isolated', 'reauth_required')", [inactiveId, company.id, host.id]);

  await gotoDashboard(page);
  await page.getByRole("button", { name: "Select company" }).click();
  await page.locator(".company-dropdown-item").filter({ hasText: company.name }).locator(".company-dropdown-item-name").click();
  await page.getByRole("button", { name: "Actions menu" }).click();
  await page.getByRole("button", { name: "Import Sessions", exact: true }).click();
  const modal = page.getByRole("dialog", { name: "Import Sessions" });
  await modal.getByRole("combobox").selectOption(host.id);
  await modal.getByPlaceholder("/path/to/project").fill(workspace);
  await modal.getByRole("button", { name: "Scan", exact: true }).click();
  await expect(modal.getByText("4 found · 0 selected · newest first")).toBeVisible();
  await expect(modal.getByText("Needs sign-in", { exact: true })).toHaveCount(0);
  for (const session of selected.filter((session) => session.harness_account_id)) {
    await expect(modal.locator(".workspace-session-harness-badge").filter({ hasText: session.marker })).toBeVisible();
  }
  await modal.getByRole("button", { name: "Select all", exact: true }).click();
  const importedResponse = page.waitForResponse((response) => response.url().endsWith("/workspace-sessions/import") && response.request().method() === "POST");
  const openedSession = page.waitForResponse((response) => /\/runtime\/bindings\/[^/]+\/session$/.test(response.url()) && response.request().method() === "POST");
  await modal.getByRole("button", { name: "Import 4 sessions", exact: true }).click();
  const imported = await importedResponse;
  expect(imported.ok(), await imported.text()).toBeTruthy();
  expect((await imported.json()).imported).toHaveLength(4);
  // Import opens the first DM. Release its structured session before testing native PTY resume.
  const opened = await openedSession;
  expect(opened.ok(), await opened.text()).toBe(true);
  const snapshot = await opened.json();
  await page.goto("about:blank");
  await expect.poll(async () => {
    const response = await page.request.get(opened.url(), { headers });
    expect(response.ok(), await response.text()).toBe(true);
    return (await response.json()).status;
  }).toBe("ready");
  const closed = await page.request.post(`${opened.url()}/commands`, { headers, data: { action: "close", instance: snapshot.instance } });
  expect(closed.ok(), await closed.text()).toBe(true);

  const retry = await page.request.post(`${API_BASE}/v1/workspace-sessions/import`, { headers, data: { company_id: company.id, runtime_host_id: host.id, workspace_path: workspace, sessions: selected } });
  expect(retry.ok(), await retry.text()).toBeTruthy();
  expect((await retry.json()).imported.every((item: { already_imported: boolean }) => item.already_imported)).toBe(true);
  const unowned = await page.request.post(`${API_BASE}/v1/workspace-sessions/import`, { headers, data: { company_id: company.id, runtime_host_id: host.id, workspace_path: workspace, sessions: [{ ...selected[0], harness_account_id: randomUUID() }] } });
  expect(unowned.status()).toBe(404);
  for (const session of selected) {
    const rows = await db.query<{ binding_id: string; config_json: Record<string, unknown> }>("SELECT n.binding_id, b.config_json FROM native_session_import n JOIN agent_runtime_bindings b ON b.id = n.binding_id WHERE n.company_id = $1 AND n.driver_type = $2 AND n.harness_account_id IS NOT DISTINCT FROM $3", [company.id, `${session.harness}_terminal`, session.harness_account_id]);
    expect(rows.rows).toHaveLength(1);
    const { binding_id, config_json } = rows.rows[0];
    expect(config_json.harness_account_id ?? null).toBe(session.harness_account_id);
    expect(config_json.model).toBeUndefined();
    if (session.harness === "codex") {
      const anchor = config_json.terminal_session as { native_session_path: string };
      expect(await readFile(anchor.native_session_path, "utf8")).toContain(session.marker);
    }
    const ensured = await page.request.post(`${API_BASE}/v1/terminals/${binding_id}/ensure`, { headers });
    expect(ensured.ok(), await ensured.text()).toBeTruthy();
    await expect.poll(() => readFile(path.join(workspace, `observed-${session.marker}`), "utf8").catch(() => "")).toContain(session.native_session_id);
    expect(await readFile(path.join(workspace, `observed-${session.marker}`), "utf8")).not.toContain("--model");
  }
});

test("hiding an inactive imported tab closes it and importing restores the same session", async ({ page }) => {
  const { token, principal } = await login(page);
  const workspace = await mkdtemp(join(homedir(), "choruz-hide-test-"));
  const claudeProjects = join(homedir(), ".claude", "projects");
  await mkdir(claudeProjects, { recursive: true });
  const catalog = await mkdtemp(join(claudeProjects, "choruz-hide-test-"));
  const nativeId = crypto.randomUUID();
  const otherId = crypto.randomUUID();
  const otherTitle = `Unselected import ${otherId}`;
  const title = `Hidden import ${nativeId}`;
  let companyId: string | undefined;
  try {
    await writeFile(join(catalog, `${nativeId}.jsonl`), `${JSON.stringify({ type: "user", cwd: workspace, customTitle: title })}\n`);
    await writeFile(join(catalog, `${otherId}.jsonl`), `${JSON.stringify({ type: "user", cwd: workspace, customTitle: otherTitle })}\n`);
    const company = await createCompany(page, token, principal.id, title);
    companyId = company.id;
    const group = await createGroup(page, token, principal.id, "Other conversation", [], company.id);
    // This scenario owns scan/import/hide persistence, not provider execution.
    await page.routeWebSocket((url) => url.pathname.startsWith("/v1/ws/terminals/"), (socket) => socket.close());
    await gotoDashboard(page);
    await page.locator(".company-selector-btn").click();
    await page.locator(".company-dropdown-item-name").filter({ hasText: title }).click();
    const importSession = async () => {
      await page.getByRole("button", { name: "Actions menu" }).click();
      await page.getByRole("button", { name: "Import Sessions", exact: true }).click();
      const modal = page.getByRole("dialog", { name: "Import Sessions" });
      const folder = modal.getByPlaceholder("/path/to/project");
      // Own the hide/reimport flow after the path picker finishes initialization.
      await expect(folder).toHaveValue(homedir());
      await folder.fill(workspace);
      await expect(folder).toHaveValue(workspace);
      for (const harness of ["Codex", "Pi", "Grok", "OpenCode"]) await modal.getByLabel(harness, { exact: true }).uncheck();
      await modal.getByRole("button", { name: "Scan", exact: true }).click();
      await expect(modal.getByText(title, { exact: true })).toBeVisible();
      await expect(modal.getByText(otherTitle, { exact: true })).toBeVisible();
      await modal.getByRole("searchbox", { name: "Filter sessions" }).fill(nativeId);
      await expect(modal.locator(".workspace-session-row")).toHaveCount(1);
      await modal.getByRole("button", { name: "Select visible", exact: true }).click();
      await expect(modal.getByRole("button", { name: "Import 1 session", exact: true })).toBeEnabled();
      await modal.getByRole("searchbox", { name: "Filter sessions" }).fill(otherId);
      await modal.getByRole("button", { name: "Select visible", exact: true }).click();
      await expect(modal.getByRole("button", { name: "Import 2 sessions", exact: true })).toBeEnabled();
      await modal.getByRole("button", { name: "Clear visible", exact: true }).click();
      await expect(modal.getByRole("button", { name: "Import 1 session", exact: true })).toBeEnabled();
      await modal.getByRole("searchbox", { name: "Filter sessions" }).fill("no matching native session");
      await expect(modal.getByRole("button", { name: /^(Select|Clear) (visible|all)$/ })).toHaveCount(0);
      await modal.getByRole("searchbox", { name: "Filter sessions" }).fill(nativeId);
      await expect(modal.locator(".workspace-session-row input")).toBeChecked();
      const result = page.waitForResponse((response) => response.url().endsWith("/api/v1/workspace-sessions/import"));
      await modal.getByRole("button", { name: "Import 1 session", exact: true }).click();
      const response = await result;
      expect(response.ok(), await response.text()).toBeTruthy();
      expect(response.request().postDataJSON().sessions).toHaveLength(1);
      const importedSessions = (await response.json()).imported;
      expect(importedSessions).toHaveLength(1);
      await expect(modal).toHaveCount(0);
      return importedSessions[0] as { conversation_id: string; binding_id: string; agent_name: string; already_imported: boolean };
    };
    const imported = await importSession();
    const groups = page.getByRole("button", { name: /Group Conversations/ });
    if (await groups.getAttribute("aria-expanded") !== "true") await groups.click();
    await page.locator(`[data-conversation-id="${group.id}"]`).click();
    const closeTab = page.getByRole("button", { name: `Close ${imported.agent_name}`, exact: true });
    await expect(closeTab).toBeVisible();
    await page.getByRole("button", { name: `Hide session: ${imported.agent_name}`, exact: true }).click();
    await expect(closeTab).toHaveCount(0);
    await expect(page.locator(`[data-conversation-id="${imported.conversation_id}"]`)).toHaveCount(0);
    const restored = await importSession();
    expect(restored).toMatchObject({ conversation_id: imported.conversation_id, binding_id: imported.binding_id, already_imported: true });
    await expect(page.locator(`[data-conversation-id="${imported.conversation_id}"]`)).toBeVisible();
    await page.reload();
    await expect(page.locator(`[data-conversation-id="${imported.conversation_id}"]`)).toBeVisible();
    await expect(page.getByText(otherTitle, { exact: true })).toHaveCount(0);
  } finally {
    try {
      if (companyId) await deleteCompany(page, token, companyId);
    } finally {
      await rm(workspace, { recursive: true, force: true });
      await rm(catalog, { recursive: true, force: true });
    }
  }
});

const ROOT = "/projects";
const SESSIONS = [
  ["claude", "claude-1", "/projects/api", "Claude API"],
  ["codex", "codex-1", "/projects/web", "Codex Web"],
  ["pi", "pi-1", "/projects/research", "Pi Research"],
  ["grok", "grok-1", "/projects/infra", "Grok Infra"],
  ["open_code", "opencode-1", "/projects/tools", "OpenCode Tools"],
] as const;

test("imports nested sessions from every supported harness with their real workspaces", async ({
  page,
}) => {
  await login(page);
  const scanBodies: Array<{ workspace_path: string; harnesses: string[] }> = [];

  let importBody: {
    workspace_path: string;
    sessions: Array<{
      harness: string;
      native_session_id: string;
      workspace_path: string;
    }>;
  } | null = null;

  await page.route("**/api/v1/workspace-sessions/scan", async (route) => {
    const request = route.request().postDataJSON() as {
      workspace_path: string;
      harnesses: string[];
    };
    scanBodies.push(request);
    const matchingSessions = request.workspace_path === ROOT
      ? SESSIONS.filter(([harness]) => request.harnesses.includes(harness))
      : [];
    if (request.workspace_path === ROOT && request.harnesses.length === 4) {
      await new Promise((resolve) => setTimeout(resolve, 450));
    }
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        workspace_path: request.workspace_path,
        sessions: matchingSessions.map(([harness, native_session_id, workspace_path, title], index) => ({
          harness,
          native_session_id,
          workspace_path,
          title,
          updated_at: `2026-08-31T12:0${index}:00Z`,
          model: null,
          branch: null,
          archived: false,
        })),
        warnings: [],
      }),
    });
  });

  await page.route("**/api/v1/workspace-sessions/import", async (route) => {
    importBody = route.request().postDataJSON() as typeof importBody;
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ imported: [] }),
    });
  });

  await gotoDashboard(page);
  let companyCreateRequests = 0;
  page.on("request", (request) => {
    if (request.method() === "POST" && request.url().includes("/v1/companies")) {
      companyCreateRequests += 1;
    }
  });
  await page.getByRole("button", { name: "Actions menu" }).click();
  await page.getByRole("button", { name: "Import Sessions" }).click();

  const modal = page.getByRole("dialog", { name: "Import Sessions" });
  const pathInput = modal.getByPlaceholder("/path/to/project");
  await expect(pathInput).not.toHaveValue("");
  await pathInput.fill(ROOT);
  await expect(modal.getByText("Ready to scan", { exact: true })).toBeVisible();
  expect(scanBodies).toHaveLength(0);

  await modal.getByRole("button", { name: "Scan", exact: true }).click();
  await expect(modal.getByText("5 found · 0 selected · newest first")).toBeVisible();
  await expect.poll(() => scanBodies.some((body) => (
    body.workspace_path === ROOT
      && body.harnesses.join(",") === "claude,codex,pi,grok,open_code"
  ))).toBe(true);
  for (const [, , workspace, title] of SESSIONS) {
    await expect(modal.getByText(workspace.slice(ROOT.length + 1), { exact: true })).toBeVisible();
    await expect(modal.getByText(title, { exact: true })).toBeVisible();
  }
  await expect(modal.locator(".workspace-session-row-title strong")).toHaveText([
    "OpenCode Tools",
    "Grok Infra",
    "Pi Research",
    "Codex Web",
    "Claude API",
  ]);

  await modal.getByRole("button", { name: "Select all" }).click();
  await expect(modal.getByText("5 found · 5 selected · newest first")).toBeVisible();

  await modal.getByLabel("OpenCode", { exact: true }).uncheck();
  await expect(modal.getByText("Ready to scan", { exact: true })).toBeVisible();
  await expect(modal.getByRole("button", { name: "Import 5 sessions" })).toHaveCount(0);
  await modal.getByRole("button", { name: "Scan", exact: true }).click();
  await expect(modal.getByText("4 found · 0 selected · newest first")).toBeVisible();
  await expect.poll(() => scanBodies.at(-1)?.harnesses).toEqual([
    "claude",
    "codex",
    "pi",
    "grok",
  ]);
  await modal.getByLabel("OpenCode", { exact: true }).check();
  await modal.getByRole("button", { name: "Scan", exact: true }).click();
  await expect(modal.getByText("5 found · 0 selected · newest first")).toBeVisible();
  await expect(modal.getByRole("button", { name: "Import 0 sessions" })).toBeDisabled();
  await modal.getByRole("button", { name: "Select all" }).click();
  await expect(modal.getByText("5 found · 5 selected · newest first")).toBeVisible();

  await modal.getByRole("button", { name: "Import 5 sessions" }).click();
  await expect.poll(() => importBody).not.toBeNull();
  expect(importBody).toMatchObject({
    company_id: expect.any(String),
    workspace_path: ROOT,
    sessions: SESSIONS.map(([harness, native_session_id, workspace_path]) => ({
      harness,
      native_session_id,
      workspace_path,
    })),
  });
  expect(companyCreateRequests).toBe(0);
});

test("switching the scan device rejects a delayed controller home response", async ({ page }) => {
  await login(page);
  let release!: () => void;
  let requested!: () => void;
  const pending = new Promise<void>((resolve) => { release = resolve; });
  const started = new Promise<void>((resolve) => { requested = resolve; });
  await page.route(/\/api\/filesystem\?action=home$/, async (route) => {
    requested();
    await pending;
    await route.fulfill({ json: { home: "/controller-home" } });
  });
  await page.route("**/api/v1/companies/*/runtime-hosts", (route) => route.fulfill({ json: [{
    id: "owned-home-host", name: "Target B", status: "online", company_id: "company",
  }] }));
  await page.route("**/api/v1/runtime-hosts/owned-home-host/operations", (route) => route.fulfill({ json: { home: "/target-home", entries: [] } }));
  try {
    await gotoDashboard(page);
    await page.getByRole("button", { name: "Actions menu" }).click();
    await page.getByRole("button", { name: "Import Sessions" }).click();
    await started;
    const modal = page.getByRole("dialog", { name: "Import Sessions" });
    await modal.getByLabel("Device to scan").selectOption("owned-home-host");
    release();
    await expect(modal.getByPlaceholder("/path/to/project")).toHaveValue("/target-home");
  } finally {
    release();
  }
});

test("scans and imports sessions on the selected connected device", async ({ page }) => {
  await login(page);
  const host = {
    id: "host-west",
    company_id: "company",
    name: "Build Server West",
    status: "online",
    last_seen_at: new Date().toISOString(),
    created_at: new Date().toISOString(),
  };
  let scanBody: Record<string, unknown> | null = null;
  let importBody: Record<string, unknown> | null = null;
  const operationBodies: Array<Record<string, unknown>> = [];
  await page.route("**/api/v1/companies/*/runtime-hosts", (route) => route.fulfill({
    status: 200,
    contentType: "application/json",
    body: JSON.stringify([host]),
  }));
  await page.route("**/api/v1/workspace-sessions/scan", async (route) => {
    scanBody = route.request().postDataJSON() as Record<string, unknown>;
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        workspace_path: "/srv/projects",
        sessions: [{
          harness: "claude",
          native_session_id: "remote-session",
          workspace_path: "/srv/projects/app",
          title: "Remote work",
          updated_at: "2026-09-03T12:00:00Z",
          model: "claude-sonnet-4-5",
          branch: "main",
          archived: false,
        }],
        warnings: [],
      }),
    });
  });
  await page.route("**/api/v1/workspace-sessions/import", async (route) => {
    importBody = route.request().postDataJSON() as Record<string, unknown>;
    await route.fulfill({ status: 200, contentType: "application/json", body: "{\"imported\":[]}" });
  });
  await page.route("**/api/v1/runtime-hosts/host-west/operations", async (route) => {
    operationBodies.push(route.request().postDataJSON() as Record<string, unknown>);
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        path: "/srv/projects",
        parent: "/srv",
        entries: [{ name: "app", type: "directory", path: "/srv/projects/app" }],
      }),
    });
  });

  await gotoDashboard(page);
  await page.getByRole("button", { name: "Actions menu" }).click();
  await page.getByRole("button", { name: "Import Sessions" }).click();
  const modal = page.getByRole("dialog", { name: "Import Sessions" });
  await expect(modal.getByPlaceholder("/path/to/project")).toHaveValue(homedir());
  await modal.getByLabel("Device to scan").selectOption(host.id);
  await modal.getByPlaceholder("/path/to/project").fill("/srv/projects");
  await expect(modal.getByPlaceholder("/path/to/project")).toHaveValue("/srv/projects");
  await modal.getByRole("button", { name: "Browse" }).click();
  const picker = page.getByRole("dialog", { name: "Select Folder" });
  await expect(picker.getByText("app", { exact: true })).toBeVisible();
  expect(operationBodies).toContainEqual({
    kind: "filesystem.list",
    request: { path: "/srv/projects", include_files: false },
  });
  await picker.getByRole("button", { name: "Cancel" }).click();
  await modal.getByRole("button", { name: "Scan", exact: true }).click();
  await expect(modal.getByText("Remote work", { exact: true })).toBeVisible();
  expect(scanBody).toMatchObject({ runtime_host_id: host.id, workspace_path: "/srv/projects" });
  await modal.getByRole("button", { name: "Select all" }).click();
  await modal.getByRole("button", { name: "Import 1 session" }).click();
  await expect.poll(() => importBody).not.toBeNull();
  expect(importBody).toMatchObject({ runtime_host_id: host.id, workspace_path: "/srv/projects" });
});
