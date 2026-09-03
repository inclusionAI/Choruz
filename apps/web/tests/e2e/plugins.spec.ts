import { expect, test } from "@playwright/test";

import { createGroup, provisionAgent } from "../fixtures/api";
import { API_BASE, gotoDashboard, login, WEB_BASE } from "../fixtures/auth";

const configuredPluginIds = process.env.CHORUZ_PLUGINS === undefined
  ? new Set(["kanban", "pixel-world", "workspace-git", "remote-ssh", "remote-control", "agent-skills", "mathcode"])
  : new Set(process.env.CHORUZ_PLUGINS.split(",").map((id) => id.trim()).filter(Boolean));

test("Host manifests and Client contributions agree", async ({ page }) => {
  const { token, principal } = await login(page);
  const response = await page.request.get(`${API_BASE}/v1/console`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  expect(response.ok()).toBeTruthy();
  const snapshot = await response.json() as { plugins: Array<{ id: string; version: string }> };
  expect(snapshot.plugins.map((plugin) => plugin.id)).toEqual(
    ["kanban", "pixel-world", "workspace-git", "remote-ssh", "remote-control", "agent-skills", "mathcode"].filter((id) => configuredPluginIds.has(id)),
  );
  expect(snapshot.plugins.every((plugin) => plugin.version === "1")).toBeTruthy();

  const group = await createGroup(
    page,
    token,
    principal.id,
    `plugin-contract-${Date.now()}`,
  );
  const taskRequests: string[] = [];
  page.on("request", (request) => {
    if (/\/v1\/conversations\/[^/]+\/tasks(?:\/|$|\?)/.test(request.url())) {
      taskRequests.push(request.url());
    }
  });
  await page.goto(`${WEB_BASE}/dashboard?conversationId=${group.id}`);
  await page.waitForSelector(".chat-sidebar, .chat-app", { timeout: 15_000 });

  const tasksTab = page.getByRole("tab", { name: "Tasks" });
  if (configuredPluginIds.has("kanban")) {
    await expect(tasksTab).toBeVisible();
  } else {
    await expect(tasksTab).toHaveCount(0);
    await page.waitForTimeout(500);
    expect(taskRequests).toEqual([]);
  }

  await page.locator('.chat-header button[title="Toggle details"]').click();
  const gitTab = page.locator(".detail-tab").filter({ hasText: "Git" });
  if (configuredPluginIds.has("workspace-git")) {
    await expect(gitTab).toBeVisible();
  } else {
    await expect(gitTab).toHaveCount(0);
  }

  await gotoDashboard(page, { expandSidebarSections: false });
  await page.getByRole("button", { name: "Actions menu" }).click();
  const pixelWorldAction = page.getByRole("button", { name: "Pixel World" });
  if (configuredPluginIds.has("pixel-world")) {
    await expect(pixelWorldAction).toBeVisible();
  } else {
    await expect(pixelWorldAction).toHaveCount(0);
  }

  const remoteSshAction = page.getByRole("button", { name: "Servers" });
  if (configuredPluginIds.has("remote-ssh")) {
    await expect(remoteSshAction).toBeVisible();
    await remoteSshAction.click();
    await expect(page.getByRole("heading", { name: "Remote Servers" })).toBeVisible();
    await page.getByRole("button", { name: "Close" }).click();
  } else {
    await expect(remoteSshAction).toHaveCount(0);
  }

  const createAgentAction = page.getByRole("button", { name: "Create Agent" });
  if (!(await createAgentAction.isVisible())) {
    await page.getByRole("button", { name: "Actions menu" }).click();
  }
  await createAgentAction.click();
  const provisioningSkills = page.locator(".create-agent-section-label").filter({ hasText: "Skills (optional)" });
  if (configuredPluginIds.has("agent-skills")) {
    await expect(provisioningSkills).toBeVisible();
  } else {
    await expect(provisioningSkills).toHaveCount(0);
  }

  const gitApi = await page.request.get(`${WEB_BASE}/api/git-graph`);
  expect(gitApi.status()).toBe(configuredPluginIds.has("workspace-git") ? 400 : 404);
  const skillsApi = await page.request.get(`${WEB_BASE}/api/agent-skills`);
  expect(skillsApi.status()).toBe(configuredPluginIds.has("agent-skills") ? 400 : 404);
  const sshApi = await page.request.get(`${API_BASE}/v1/ssh/hosts`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  expect(sshApi.status()).toBe(configuredPluginIds.has("remote-ssh") ? 200 : 404);

  const agent = await provisionAgent(page, token, `plugin-agent-${Date.now()}`);
  await page.goto(`${WEB_BASE}/dashboard?conversationId=${agent.conversationId}`);
  await page.locator('.chat-header button[title="Toggle details"]').click();
  const skillsTab = page.locator(".detail-tab").filter({ hasText: "Skills" });
  if (configuredPluginIds.has("agent-skills")) {
    await expect(skillsTab).toBeVisible();
  } else {
    await expect(skillsTab).toHaveCount(0);
  }
});
