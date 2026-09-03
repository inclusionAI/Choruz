import { expect, test } from "@playwright/test";

import { gotoDashboard, login } from "../fixtures/auth";

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
