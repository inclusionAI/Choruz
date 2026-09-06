import { expect, test } from "@playwright/test";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, mkdir, writeFile, readFile, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { API_BASE, login, gotoDashboard } from "../fixtures/auth";
import { createCompany, deleteCompany, uniqueName } from "../fixtures/api";

test("remote creation reads B's harness and writes B's custom workspace through the real host link", async ({ page }) => {
  test.setTimeout(90_000);
  const session = await login(page);
  const company = await createCompany(page, session.token, session.principal.id, uniqueName("device-owner"));
  const home = await realpath(await mkdtemp(path.join(tmpdir(), "choruz-device-b-")));
  const workspace = path.join(home, "project");
  const binary = path.join(home, "opencode");
  const config = path.join(home, "connector.json");
  const connector = path.resolve("../../target/debug/choruz-connector");
  const headers = { Authorization: `Bearer ${session.token}` };
  let processB: ReturnType<typeof spawn> | undefined;
  try {
    await mkdir(workspace);
    // Only the external CLI is a deterministic substitute. Pairing, Web/API
    // validation, host dispatch, workspace writes and binding storage are real.
    await writeFile(binary, '#!/bin/sh\ncase "$1" in\n--version) echo device-b-cli;;\nmodels) echo fixture/device-b-model;;\n*) exit 0;;\nesac\n', { mode: 0o700 });
    const pairing = await page.request.post(`${API_BASE}/v1/companies/${company.id}/runtime-host-pairings`, { headers });
    expect(pairing.ok()).toBeTruthy();
    const { code } = await pairing.json();
    const redeemed = await page.request.post(`${API_BASE}/v1/runtime-host-pairings/redeem`, { data: { code, name: "Device B" } });
    expect(redeemed.ok()).toBeTruthy();
    const { host, host_token } = await redeemed.json();
    await writeFile(config, JSON.stringify({ api_url: API_BASE, host_id: host.id, host_name: host.name, host_token, max_concurrency: 1 }), { mode: 0o600 });
    processB = spawn(connector, ["run", "--config", config], { env: { ...process.env, HOME: home, CHORUZ_FS_BROWSE_ROOTS: home, CHORUZ_OPENCODE_BINARY: binary }, stdio: "ignore" });
    await expect.poll(async () => {
      const response = await page.request.post(`${API_BASE}/v1/runtime-hosts/${host.id}/operations`, { headers, data: { kind: "filesystem.home" } });
      return response.ok() ? (await response.json()).home : null;
    }).toBe(home);

    await gotoDashboard(page);
    await page.getByRole("button", { name: "Select company" }).click();
    await page.locator(".company-dropdown-item").filter({ hasText: company.name }).locator(".company-dropdown-item-name").click();
    await page.getByRole("button", { name: "Actions menu" }).click();
    await page.getByRole("button", { name: "Create Agent", exact: true }).click();
    const modal = page.getByRole("dialog", { name: "Create Agent" });
    await modal.getByLabel("Agent name", { exact: true }).fill(uniqueName("device-owned-agent"));
    await modal.getByLabel("Runtime server").selectOption(host.id);
    await modal.getByLabel("Driver", { exact: true }).selectOption("opencode_terminal");
    await expect(modal.locator('datalist option[value="fixture/device-b-model"]')).toHaveCount(1);
    await modal.getByLabel("Model", { exact: true }).fill("fixture/device-b-model");
    await modal.getByLabel("Custom workspace path").check();
    await modal.getByPlaceholder("/path/to/workspace").fill(workspace);
    await modal.getByRole("button", { name: "Review & Create", exact: true }).click();
    const resultPromise = page.waitForResponse((response) => response.url().endsWith("/api/agents/provision") && response.request().method() === "POST", { timeout: 20_000 });
    await modal.getByRole("button", { name: "Create Agent", exact: true }).click();
    const result = await resultPromise;
    expect(result.status(), await result.text()).toBe(201);
    const created = await result.json();
    expect(created.workspace_path).toBe(workspace);
    expect(created.binding.runtime_host_id).toBe(host.id);
    expect(await readFile(path.join(workspace, "AGENTS.md"), "utf8")).toContain("choruz");
    const binding = await page.request.get(`${API_BASE}/v1/runtime/bindings/${created.binding.id}`, { headers });
    expect(binding.ok()).toBeTruthy();
    expect((await binding.json()).runtime_host_id).toBe(host.id);
  } finally {
    if (processB && processB.exitCode === null) {
      const exited = once(processB, "exit");
      processB.kill("SIGTERM");
      await exited;
    }
    try {
      await deleteCompany(page, session.token, company.id);
    } finally {
      await rm(home, { recursive: true, force: true });
    }
  }
});
