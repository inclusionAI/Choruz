import { expect, test } from "@playwright/test";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, mkdir, writeFile, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { API_BASE, login } from "./auth";
import { createCompany, deleteCompany, uniqueName } from "./api";

type RuntimeDevice = {
  home: string;
  company: { id: string; name: string };
  host: { id: string; name: string };
  headers: { Authorization: string };
  session: Awaited<ReturnType<typeof login>>;
};

// A real connector process with device-local roots distinct from the API's.
export const runtimeTest = test.extend<{ device: RuntimeDevice }>({
  device: async ({ page }, use) => {
    const session = await login(page);
    const headers = { Authorization: `Bearer ${session.token}` };
    const company = await createCompany(page, session.token, session.principal.id, uniqueName("device-owner"));
    const home = await realpath(await mkdtemp(path.join(tmpdir(), "choruz-device-b-")));
    let child: ReturnType<typeof spawn> | undefined;
    try {
      await mkdir(path.join(home, "bin"));
      const paired = await page.request.post(`${API_BASE}/v1/companies/${company.id}/runtime-host-pairings`, { headers });
      expect(paired.ok()).toBeTruthy();
      const { code } = await paired.json();
      const redeemed = await page.request.post(`${API_BASE}/v1/runtime-host-pairings/redeem`, { data: { code, name: "Device B" } });
      expect(redeemed.ok()).toBeTruthy();
      const { host, host_token } = await redeemed.json();
      const config = path.join(home, "connector.json");
      await writeFile(config, JSON.stringify({ api_url: API_BASE, host_id: host.id, host_token, host_name: host.name, max_concurrency: 1 }), { mode: 0o600 });
      child = spawn(path.resolve("../../target/debug/choruz-connector"), ["run", "--config", config], {
        env: { ...process.env, HOME: home, PATH: `${path.join(home, "bin")}:${process.env.PATH}`, CHORUZ_CLAUDE_BINARY: path.join(home, "bin", "claude"), CHORUZ_CODEX_BINARY: path.join(home, "bin", "codex"), CHORUZ_GROK_BINARY: path.join(home, "grok-target"), CLAUDE_CONFIG_DIR: path.join(home, ".claude"), CODEX_HOME: path.join(home, ".codex"), CHORUZ_HARNESS_ACCOUNT_ROOT: path.join(home, "accounts"), CHORUZ_RUNTIME_DIR: path.join(home, "runtime"), CHORUZ_FS_BROWSE_ROOTS: home }, stdio: "ignore",
      });
      await expect.poll(async () => {
        const response = await page.request.post(`${API_BASE}/v1/runtime-hosts/${host.id}/operations`, { headers, data: { kind: "filesystem.home" } });
        return response.ok() ? (await response.json()).home : null;
      }).toBe(home);
      await use({ home, company, host, headers, session });
    } finally {
      if (child && child.exitCode === null) {
        const exited = once(child, "exit");
        child.kill("SIGTERM");
        await exited;
      }
      try { await deleteCompany(page, session.token, company.id); }
      finally { await rm(home, { recursive: true, force: true }); }
    }
  },
});
