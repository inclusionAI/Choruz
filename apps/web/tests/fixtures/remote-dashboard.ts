import { createServer, request } from "node:http";
import { once } from "node:events";
import { connect, type Socket } from "node:net";
import { expect, test as base, type APIRequestContext, type Page } from "@playwright/test";
import { API_BASE, WEB_BASE } from "./auth";

export const test = base.extend<{
  hostedDashboard: { origin: string; apiRequests: string[] };
  ownedResources: Array<(request: APIRequestContext) => Promise<void>>;
}>({
  ownedResources: async ({ page, request }, use) => {
    const cleanup: Array<(request: APIRequestContext) => Promise<void>> = [];
    try { await use(cleanup); }
    finally {
      if (!page.isClosed()) await page.close();
      const results = await Promise.allSettled(cleanup.reverse().map((dispose) => dispose(request)));
      const failures = results.filter((result) => result.status === "rejected");
      if (failures.length) throw new AggregateError(failures.map((result) => result.reason), "Remote fixture cleanup failed");
    }
  },
  hostedDashboard: async ({}, use) => {
    const apiRequests: string[] = [];
    const upgrades = new Set<Socket>();
    // A hosts the product page/assets but has no Choruz API or B's data.
    const server = createServer((incoming, outgoing) => {
      if (incoming.url?.startsWith("/api/")) {
        apiRequests.push(`${incoming.method} ${incoming.url}`);
        outgoing.writeHead(404, { "Content-Type": "application/json" });
        outgoing.end(JSON.stringify({ error: "Hosted dashboard has no local API" }));
        return;
      }
      const target = new URL(incoming.url ?? "/", WEB_BASE);
      const proxy = request(target, {
        method: incoming.method, agent: false,
        headers: { ...incoming.headers, host: target.host },
      }, (response) => {
        outgoing.writeHead(response.statusCode ?? 502, response.headers);
        response.pipe(outgoing);
      });
      proxy.on("error", () => { outgoing.writeHead(502); outgoing.end(); });
      incoming.pipe(proxy);
    });
    server.on("upgrade", (incoming, socket, head) => {
      if (!incoming.url?.startsWith("/_next/")) { socket.destroy(); return; }
      const target = new URL(WEB_BASE);
      const upstream = connect(Number(target.port), target.hostname, () => {
        upstream.write(`${incoming.method} ${incoming.url} HTTP/1.1\r\n${Object.entries({ ...incoming.headers, host: target.host }).map(([key, value]) => `${key}: ${value}`).join("\r\n")}\r\n\r\n`);
        upstream.write(head);
        socket.pipe(upstream).pipe(socket);
      });
      upgrades.add(upstream);
      upstream.on("error", () => socket.destroy());
      socket.on("error", () => upstream.destroy());
      socket.on("close", () => upstream.destroy());
      upstream.on("close", () => { upgrades.delete(upstream); socket.destroy(); });
    });
    server.listen(0, "127.0.0.1");
    await once(server, "listening");
    const address = server.address();
    if (!address || typeof address === "string") throw new Error("Hosted dashboard did not bind TCP");
    try {
      await use({ origin: `http://127.0.0.1:${address.port}`, apiRequests });
    } finally {
      const closed = new Promise<void>((resolve, reject) => server.close((error) => error ? reject(error) : resolve()));
      server.closeAllConnections();
      const upgradedClosed = [...upgrades].map((socket) => once(socket, "close"));
      for (const socket of upgrades) socket.destroy();
      await Promise.all([closed, ...upgradedClosed]);
    }
  },
});

export async function openRemoteDashboard(page: Page, token: string, origin: string) {
  const gateway = process.env.CHORUZ_REMOTE_CONTROL_GATEWAY_URL;
  expect(gateway, "remote spec must use the Worker-owning host runner").toMatch(/^http:\/\/127\.0\.0\.1:/);
  const paired = await page.request.post(`${API_BASE}/v1/remote-control/pairings`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  expect(paired.ok(), await paired.text()).toBeTruthy();
  const { credential } = await paired.json();
  await page.goto(`${origin}/remote?gateway=${encodeURIComponent(gateway!)}&device_name=OwnedBrowser#credential=${credential}`);
  await expect(page.locator(".chat-shell")).toBeVisible({ timeout: 30_000 });
}
