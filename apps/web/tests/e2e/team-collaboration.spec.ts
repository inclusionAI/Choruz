import { expect, test, type Page } from "@playwright/test";
import { createServer, type Server } from "node:http";
import { API_BASE, WEB_BASE, login } from "../fixtures/auth";
import {
  createDirectConversation,
  createGroup,
  disablePrincipal,
  getMessages,
  provisionAgent,
  sendMessage,
  uniqueName,
} from "../fixtures/api";

type WebhookEvent = {
  event_type?: string;
  payload?: {
    conversation_id?: string;
    content?: string;
    sender?: { id?: string; name?: string; type?: string };
  };
};

type CapturingWebhook = {
  url: string;
  server: Server;
  events: WebhookEvent[];
  setStatus: (status: number) => void;
};

async function startCapturingWebhook(status = 200): Promise<CapturingWebhook> {
  const received: WebhookEvent[] = [];
  let responseStatus = status;
  const server = createServer((req, res) => {
    if (req.method !== "POST") {
      res.writeHead(405).end();
      return;
    }

    let body = "";
    req.setEncoding("utf8");
    req.on("data", (chunk) => {
      body += chunk;
    });
    req.on("end", () => {
      received.push(JSON.parse(body) as WebhookEvent);
      res.writeHead(responseStatus, { "content-type": "application/json" });
      res.end(JSON.stringify({ ok: responseStatus >= 200 && responseStatus < 300 }));
    });
  });

  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      server.off("error", reject);
      resolve();
    });
  });
  const address = server.address();
  if (!address || typeof address === "string") {
    throw new Error("team collaboration webhook did not bind to a TCP port");
  }

  return {
    url: `http://127.0.0.1:${address.port}/hook`,
    server,
    events: received,
    setStatus: (nextStatus) => {
      responseStatus = nextStatus;
    },
  };
}

async function closeServer(server: Server) {
  await new Promise<void>((resolve, reject) => {
    server.close((error) => (error ? reject(error) : resolve()));
  });
}

async function waitForEventCount(
  webhook: CapturingWebhook,
  count: number,
  timeoutMs = 15_000,
) {
  await expect
    .poll(() => webhook.events.length, { timeout: timeoutMs })
    .toBeGreaterThanOrEqual(count);
}

async function sendAgentMessage(
  page: Page,
  secret: string,
  agentId: string,
  conversationId: string,
  content: string,
) {
  const response = await page.request.post(`${API_BASE}/v1/messages`, {
    headers: { Authorization: `Bearer ${secret}` },
    data: {
      actor_id: agentId,
      conversation_id: conversationId,
      content,
      content_type: "text/plain",
      idempotency_key: `team-e2e-${Date.now()}-${Math.random().toString(16).slice(2)}`,
      metadata: {},
    },
  });
  expect(
    response.ok(),
    `agent message -> ${response.status()}: ${await response.text().catch(() => "")}`,
  ).toBeTruthy();
}

async function waitForAgentMessages(
  page: Page,
  token: string,
  principalId: string,
  conversationId: string,
  agentIds: string[],
) {
  let messages: Awaited<ReturnType<typeof getMessages>> = [];
  await expect
    .poll(async () => {
      messages = await getMessages(page, token, principalId, conversationId, 100);
      const respondingAgentIds = messages
        .filter((message) => agentIds.includes(message.sender_id))
        .map((message) => message.sender_id);
      return new Set(respondingAgentIds).size;
    }, { timeout: 15_000 })
    .toBe(agentIds.length);
  return messages;
}

async function openGroup(page: Page, conversationId: string) {
  await page.goto(`${WEB_BASE}/dashboard?conversationId=${conversationId}`);
  await expect(page.locator(".messages-area")).toBeVisible({ timeout: 15_000 });
}

test.describe("Team collaboration", () => {
  test("fans out once to each eligible agent and restores distinct results after reload", async ({ page }) => {
    const { token, principal } = await login(page);
    const backendWebhook = await startCapturingWebhook();
    const reviewerWebhook = await startCapturingWebhook();
    const disabledWebhook = await startCapturingWebhook();

    try {
      const backend = await provisionAgent(page, token, uniqueName("team-backend"), {
        driver: "webhook_agent",
        webhookUrl: backendWebhook.url,
      });
      const reviewer = await provisionAgent(page, token, uniqueName("team-reviewer"), {
        driver: "webhook_agent",
        webhookUrl: reviewerWebhook.url,
      });
      const disabled = await provisionAgent(page, token, uniqueName("team-disabled"), {
        driver: "webhook_agent",
        webhookUrl: disabledWebhook.url,
      });
      const group = await createGroup(
        page,
        token,
        principal.id,
        uniqueName("team-fanout"),
        [backend.agentId, reviewer.agentId, disabled.agentId],
      );
      await disablePrincipal(page, token, principal.id, disabled.agentId);

      const request = `@${backend.agentName} @${reviewer.agentName} @${disabled.agentName} inspect separate parts of this release`;
      await sendMessage(page, token, principal.id, group.id, request);
      await waitForEventCount(backendWebhook, 1);
      await waitForEventCount(reviewerWebhook, 1);
      await page.waitForTimeout(500);

      expect(backendWebhook.events).toHaveLength(1);
      expect(reviewerWebhook.events).toHaveLength(1);
      expect(disabledWebhook.events).toHaveLength(0);
      for (const event of [backendWebhook.events[0], reviewerWebhook.events[0]]) {
        expect(event.event_type).toBe("app_mention");
        expect(event.payload?.conversation_id).toBe(group.id);
        expect(event.payload?.content).toBe(request);
        expect(event.payload?.sender?.id).toBe(principal.id);
      }

      const backendResult = `Backend result ${Date.now()}: API contract verified.`;
      const reviewerResult = `Review result ${Date.now()}: regression checks approved.`;
      await sendAgentMessage(page, backend.secret, backend.agentId, group.id, backendResult);
      await sendAgentMessage(page, reviewer.secret, reviewer.agentId, group.id, reviewerResult);

      const messages = await waitForAgentMessages(
        page,
        token,
        principal.id,
        group.id,
        [backend.agentId, reviewer.agentId],
      );
      expect(messages.filter((message) => message.sender_id === backend.agentId)).toHaveLength(1);
      expect(messages.filter((message) => message.sender_id === reviewer.agentId)).toHaveLength(1);

      await openGroup(page, group.id);
      await expect(page.locator(".msg-group", { hasText: backendResult })).toBeVisible();
      await expect(page.locator(".msg-group", { hasText: reviewerResult })).toBeVisible();
      await page.reload();
      await expect(page.locator(".msg-group", { hasText: backendResult })).toBeVisible({
        timeout: 15_000,
      });
      await expect(page.locator(".msg-group", { hasText: reviewerResult })).toBeVisible();
    } finally {
      await Promise.all([
        closeServer(backendWebhook.server),
        closeServer(reviewerWebhook.server),
        closeServer(disabledWebhook.server),
      ]);
    }
  });

  test("routes an agent handoff without leaking another conversation's context", async ({ page }) => {
    const { token, principal } = await login(page);
    const implementerWebhook = await startCapturingWebhook();
    const reviewerWebhook = await startCapturingWebhook();

    try {
      const implementer = await provisionAgent(page, token, uniqueName("handoff-implementer"), {
        driver: "webhook_agent",
        webhookUrl: implementerWebhook.url,
      });
      const reviewer = await provisionAgent(page, token, uniqueName("handoff-reviewer"), {
        driver: "webhook_agent",
        webhookUrl: reviewerWebhook.url,
      });
      const privateConversation = await createDirectConversation(
        page,
        token,
        principal.id,
        implementer.agentId,
      );
      const privateMarker = `PRIVATE-${Date.now()}-must-not-cross`;
      await sendMessage(page, token, principal.id, privateConversation.id, privateMarker);
      const privateMessages = await getMessages(
        page,
        token,
        principal.id,
        privateConversation.id,
        20,
      );
      expect(privateMessages).toEqual(
        expect.arrayContaining([
          expect.objectContaining({ sender_id: principal.id, content: privateMarker }),
        ]),
      );

      const group = await createGroup(
        page,
        token,
        principal.id,
        uniqueName("team-handoff"),
        [implementer.agentId, reviewer.agentId],
      );
      const initialRequest = `@${implementer.agentName} implement the public release check`;
      await sendMessage(page, token, principal.id, group.id, initialRequest);
      await waitForEventCount(implementerWebhook, 1);
      expect(reviewerWebhook.events).toHaveLength(0);

      const handoff = `@${reviewer.agentName} review the public release check from the implementer`;
      await sendAgentMessage(page, implementer.secret, implementer.agentId, group.id, handoff);
      await waitForEventCount(reviewerWebhook, 1);

      const reviewEvent = reviewerWebhook.events[0];
      expect(reviewEvent.event_type).toBe("app_mention");
      expect(reviewEvent.payload?.conversation_id).toBe(group.id);
      expect(reviewEvent.payload?.sender?.id).toBe(implementer.agentId);
      expect(reviewEvent.payload?.content).toBe(handoff);
      expect(JSON.stringify(reviewEvent)).not.toContain(privateMarker);

      const reviewResult = `Review complete ${Date.now()}: public handoff approved.`;
      await sendAgentMessage(page, reviewer.secret, reviewer.agentId, group.id, reviewResult);
      await waitForAgentMessages(
        page,
        token,
        principal.id,
        group.id,
        [implementer.agentId, reviewer.agentId],
      );

      await openGroup(page, group.id);
      await expect(page.locator(".msg-group", { hasText: handoff })).toBeVisible();
      await expect(page.locator(".msg-group", { hasText: reviewResult })).toBeVisible();
      await expect(page.locator(".messages-area")).not.toContainText(privateMarker);
    } finally {
      await Promise.all([
        closeServer(implementerWebhook.server),
        closeServer(reviewerWebhook.server),
      ]);
    }
  });

  test("keeps a successful agent leg usable while a failed webhook remains retriable", async ({ page }) => {
    const { token, principal } = await login(page);
    const healthyWebhook = await startCapturingWebhook();
    const failingWebhook = await startCapturingWebhook(503);

    try {
      const healthy = await provisionAgent(page, token, uniqueName("partial-healthy"), {
        driver: "webhook_agent",
        webhookUrl: healthyWebhook.url,
      });
      const failing = await provisionAgent(page, token, uniqueName("partial-failing"), {
        driver: "webhook_agent",
        webhookUrl: failingWebhook.url,
      });
      const group = await createGroup(
        page,
        token,
        principal.id,
        uniqueName("team-partial-failure"),
        [healthy.agentId, failing.agentId],
      );

      await sendMessage(
        page,
        token,
        principal.id,
        group.id,
        `@${healthy.agentName} @${failing.agentName} run independent checks`,
      );
      await waitForEventCount(healthyWebhook, 1);
      await waitForEventCount(failingWebhook, 1);

      const healthyResult = `Healthy leg ${Date.now()}: completed despite peer failure.`;
      await sendAgentMessage(page, healthy.secret, healthy.agentId, group.id, healthyResult);
      await waitForEventCount(failingWebhook, 2);
      expect(healthyWebhook.events).toHaveLength(1);

      const flush = await page.request.post(`${API_BASE}/v1/webhooks/flush`, {
        headers: { Authorization: `Bearer ${token}` },
      });
      expect(flush.ok()).toBeTruthy();
      expect(await flush.json()).toMatchObject({ attempted: 1, delivered: 0 });

      const messages = await getMessages(page, token, principal.id, group.id, 100);
      expect(messages.filter((message) => message.sender_id === healthy.agentId)).toHaveLength(1);
      expect(messages.filter((message) => message.sender_id === failing.agentId)).toHaveLength(0);

      await openGroup(page, group.id);
      await expect(page.locator(".msg-group", { hasText: healthyResult })).toBeVisible();
    } finally {
      failingWebhook.setStatus(200);
      try {
        const cleanupFlush = await page.request.post(`${API_BASE}/v1/webhooks/flush`, {
          headers: { Authorization: `Bearer ${token}` },
        });
        expect(cleanupFlush.ok()).toBeTruthy();
      } finally {
        await Promise.all([
          closeServer(healthyWebhook.server),
          closeServer(failingWebhook.server),
        ]);
      }
    }
  });
});
