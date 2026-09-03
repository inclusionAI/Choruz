import { expect, test } from "@playwright/test";
import { realpath, rm } from "node:fs/promises";
import path from "node:path";
import { gotoDashboard, login } from "../fixtures/auth";
import {
  createCompany,
  createGroup,
  deleteCompany,
  getMessages,
  provisionAgent,
  sendMessage,
  uniqueName,
} from "../fixtures/api";

const LIVE_PROBE_ENABLED = process.env.CHORUZ_REAL_DRIVER_SMOKE === "1";
const HEADLESS_CODEX_DRIVERS = [
  "codex_terminal",
  "codex_exec",
  "codex_app_server",
] as const;
const GENERATED_WORKSPACES_ROOT = path.resolve(process.cwd(), "..", "..", ".choruz-runtime", "workspaces");

async function removeGeneratedWorkspace(workspacePath: string): Promise<void> {
  const [workspacesRoot, resolvedWorkspace] = await Promise.all([
    realpath(GENERATED_WORKSPACES_ROOT),
    realpath(workspacePath),
  ]);
  const workspaceParent = path.dirname(resolvedWorkspace);

  if (
    path.dirname(workspaceParent) !== workspacesRoot ||
    path.basename(resolvedWorkspace) !== "workspace"
  ) {
    throw new Error("live Codex probe received a workspace outside the generated runtime root");
  }

  await rm(workspaceParent, { recursive: true, force: true });
}

async function waitForPersistedAgentReply(
  page: Parameters<typeof getMessages>[0],
  token: string,
  principalId: string,
  conversationId: string,
  agentId: string,
): Promise<void> {
  const deadline = Date.now() + 180_000;
  while (Date.now() < deadline) {
    const messages = await getMessages(page, token, principalId, conversationId);
    if (messages.some((message) => message.sender_id === agentId && message.content.trim())) {
      return;
    }
    await page.waitForTimeout(1_000);
  }
  throw new Error("Codex host turn did not produce a persisted agent reply");
}

test.describe("real Codex host drivers", () => {
  test.skip(!LIVE_PROBE_ENABLED, "requires explicitly authorized authenticated Codex probes");

  for (const driver of HEADLESS_CODEX_DRIVERS) {
    test(`routes, persists, and fans out a ${driver} headless turn`, async ({ page }) => {
      test.setTimeout(240_000);

      const { token, principal } = await login(page);
      const company = await createCompany(page, token, principal.id, uniqueName("codex-live-company"));
      let generatedWorkspace: string | undefined;
      let probeError: unknown;
      const cleanupErrors: unknown[] = [];
      try {
        const agent = await provisionAgent(page, token, uniqueName(`codex-live-${driver}`), {
          driver,
          workspaceId: company.id,
        });
        generatedWorkspace = agent.workspacePath;
        const group = await createGroup(
          page,
          token,
          principal.id,
          uniqueName("codex-live-group"),
          [agent.agentId],
          company.id,
        );

        const syncSocket = page.waitForEvent("websocket", {
          predicate: (socket) => socket.url().includes("/v1/ws/sync"),
          timeout: 15_000,
        });
        await gotoDashboard(page);
        await page.locator(".company-selector-btn").click();
        await page
          .locator(".company-dropdown-item")
          .filter({ hasText: company.name })
          .click();
        await page.locator(".conv-item").filter({ hasText: group.name }).first().click();
        await syncSocket;

        const command = `@${agent.agentName} Reply with exactly: CHORUZ_HOST_PROBE_OK`;
        const sent = await sendMessage(page, token, principal.id, group.id, command);
        expect(sent.content).toBe(command);

        await waitForPersistedAgentReply(page, token, principal.id, group.id, agent.agentId);
        await expect(page.locator(".messages-area").getByText("CHORUZ_HOST_PROBE_OK")).toBeVisible({
          timeout: 30_000,
        });
      } catch (error) {
        probeError = error;
      } finally {
        try {
          await deleteCompany(page, token, company.id);
        } catch (error) {
          cleanupErrors.push(error);
        }
        if (generatedWorkspace) {
          try {
            await removeGeneratedWorkspace(generatedWorkspace);
          } catch (error) {
            cleanupErrors.push(error);
          }
        }
      }
      if (probeError) {
        if (cleanupErrors.length > 0) console.warn("live Codex probe cleanup failed after a probe failure");
        throw probeError;
      }
      if (cleanupErrors.length > 0) throw new AggregateError(cleanupErrors, "live Codex probe cleanup failed");
    });
  }
});
