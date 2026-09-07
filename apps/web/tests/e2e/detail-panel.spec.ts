import { expect, test, type Page } from "@playwright/test";
import { login, WEB_BASE } from "../fixtures/auth";
import { createGroup, provisionAgent, uniqueName } from "../fixtures/api";

async function openDetailPanel(page: Page) {
  await page.getByTitle("Toggle details").click();
  await expect(page.locator(".detail-panel")).toBeVisible();
}

test("group details show its identity, members and close explicitly", async ({ page }) => {
  const { token, principal } = await login(page);
  const group = await createGroup(page, token, principal.id, uniqueName("detail"));
  await page.goto(`${WEB_BASE}/dashboard?conversationId=${group.id}`);
  await expect(page.getByRole("heading", { name: group.name, level: 1 })).toBeVisible();
  await openDetailPanel(page);
  const panel = page.locator(".detail-panel");
  await expect(panel.locator(".detail-identity-title")).toHaveText(group.name);
  await expect(panel.locator(".detail-identity-kind")).toHaveText("Group conversation");
  await expect(panel.locator(".detail-tab").filter({ hasText: "Members" })).toBeVisible();
  await expect(panel.getByRole("button", { name: "Add member", exact: true })).toBeVisible();
  await expect(panel.getByText(`${principal.name} (you)`, { exact: true })).toBeVisible();
  const initialWidth = (await panel.boundingBox())!.width;
  await page.getByRole("separator", { name: "Resize detail panel" }).press("ArrowLeft");
  await expect.poll(async () => (await panel.boundingBox())!.width).toBeCloseTo(initialWidth + 20, 2);
  await page.reload();
  await page.getByTitle("Toggle details").click();
  await expect.poll(async () => (await panel.boundingBox())!.width).toBeCloseTo(initialWidth + 20, 2);
  await panel.getByRole("button", { name: "Close details", exact: true }).click();
  await expect(panel).toHaveCount(0);
});

test("agent details expose Overview, Config and Skills", async ({ page }) => {
  const { token } = await login(page);
  const agent = await provisionAgent(page, token, uniqueName("detail-agent"));
  expect(agent.conversationId).toBeTruthy();
  await page.goto(`${WEB_BASE}/dashboard?conversationId=${agent.conversationId}`);
  await expect(page.getByRole("heading", { name: agent.agentName, level: 1 })).toBeVisible();
  await openDetailPanel(page);
  for (const label of ["Overview", "Config", "Skills"]) {
    await expect(page.locator(".detail-tab").filter({ hasText: label })).toBeVisible();
  }
});

test("should hide legacy group workflow tasks and show queue state", async ({ page }) => {
    const { token, principal } = await login(page);
    const group = await createGroup(page, token, principal.id, uniqueName("detail-queue"));

    await page.route("**/api/v1/conversations/*/runtime-status", async (route) => {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify([
          {
            conversation_id: group.id,
            agent_principal_id: principal.id,
            agent_name: principal.name,
            status: "busy",
            queued_count: 2,
            active_command: {
              command_id: "cmd-1",
              message_id: "msg-1",
              turn_id: "turn-1",
              status: "started",
              created_at: "2026-05-25T00:00:00Z",
              updated_at: "2026-05-25T00:00:30Z",
              lease_age_seconds: 90,
              attempt_count: 1,
              last_error: null,
            },
            last_error: null,
          },
        ]),
      });
    });

    await page.goto(`${WEB_BASE}/dashboard?conversationId=${group.id}`);
    await expect(page.getByRole("heading", { name: group.name, level: 1 })).toBeVisible({ timeout: 15_000 });
    await openDetailPanel(page);

    const panel = page.locator(".detail-panel");
    await expect(panel).toBeVisible();
    await expect(panel.locator(".detail-tab").filter({ hasText: "Tasks" })).toHaveCount(0);

    await panel.locator(".detail-tab").filter({ hasText: "Queue" }).click();
    await expect(panel.getByText(/wait behind 2 earlier turns/i)).toBeVisible();
    await expect(panel.locator(".runtime-status-pill").filter({ hasText: "busy" })).toBeVisible();
    await expect(panel.getByText("2 queued turns")).toBeVisible();
  });
