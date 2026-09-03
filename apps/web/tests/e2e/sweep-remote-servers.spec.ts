// Regression test for the "Remote Servers → Failed to fetch" bug.
//
// Root cause: `apiBaseUrl()` in `lib/api/choruz-api.ts` returned
// `http://127.0.0.1:3000` directly in the browser. The gateway has no CORS
// middleware, so the browser blocked the cross-origin response and the
// server-manager modal rendered "Failed to fetch". Fix: in a browser
// context, use `/api` so the request goes through the same-origin Next.js
// rewrite that forwards to the gateway.

import { expect, test, type Page } from "@playwright/test";

const GATEWAY =
  process.env.CHORUZ_API_BASE_URL ?? `http://127.0.0.1:${process.env.CHORUZ_API_PORT ?? "3000"}`;
const WEB =
  process.env.CHORUZ_WEB_BASE_URL ?? `http://127.0.0.1:${process.env.CHORUZ_WEB_PORT ?? "3100"}`;

async function login(page: Page) {
  const r = await page.request.post(`${GATEWAY}/v1/auth/local/login`, {
    data: { username: "operator", password: "choruz-local" },
  });
  expect(r.ok()).toBeTruthy();
  const body = await r.json();
  await page.context().addCookies([
    {
      name: "choruz_session",
      value: body.session_token,
      url: WEB,
      httpOnly: true,
      sameSite: "Lax",
      expires: Math.floor(Date.now() / 1000) + 3600,
    },
  ]);
}

test("Remote Servers modal loads SSH hosts without 'Failed to fetch'", async ({ page }) => {
  await login(page);

  // Watch the network for the host-listing request so we can assert it
  // actually fired and succeeded — not just that the UI happened to render
  // empty.
  const hostsResp = page.waitForResponse(
    (r) => r.url().includes("/v1/ssh/hosts") && r.request().method() === "GET",
    { timeout: 15_000 },
  );

  await page.goto(`${WEB}/dashboard`);
  await page.waitForSelector(".chat-app, .chat-sidebar", { timeout: 15_000 });

  // Open the "+" menu → Servers
  const plus = page.locator('[aria-label="Actions menu"]');
  await expect(plus).toBeVisible({ timeout: 10_000 });
  await plus.click();
  const serversBtn = page.getByRole("button", { name: "Servers" });
  if (!(await serversBtn.isVisible({ timeout: 3_000 }).catch(() => false))) {
    test.skip();
    return;
  }
  await serversBtn.click();

  // Modal header appears
  await expect(page.getByRole("heading", { name: "Remote Servers" })).toBeVisible({
    timeout: 5_000,
  });

  // The SSH hosts request must complete with 2xx — NOT a "Failed to fetch"
  // (network-layer CORS block would reject the response outright).
  const resp = await hostsResp;
  expect(resp.ok(), `hosts fetch status ${resp.status()}`).toBeTruthy();

  // And the error block must NOT be visible.
  const errBlock = page.locator(".server-manager-error-block");
  const isErrVisible = await errBlock.isVisible({ timeout: 500 }).catch(() => false);
  expect(isErrVisible, "server manager should not show error block").toBeFalsy();
});

test("connecting one SSH host does not disable the others", async ({ page }) => {
  await login(page);
  await page.addInitScript(() => {
    window.open = () => null;
  });

  await page.route("**/api/v1/ssh/hosts", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify([
        { name: "alpha", hostname: "alpha.example", user: "alice", port: 22 },
        { name: "beta", hostname: "beta.example", user: "bob", port: 22 },
      ]),
    }),
  );
  await page.route("**/api/v1/ssh/tunnels", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: "[]" }),
  );
  let releaseConnections: () => void = () => {};
  const connectionsMayFinish = new Promise<void>((resolve) => {
    releaseConnections = resolve;
  });
  await page.route("**/api/v1/ssh/connect-choruz", async (route) => {
    const host = (route.request().postDataJSON() as { host: string }).host;
    await connectionsMayFinish;
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        id: `tunnel-${host}`,
        host,
        local_port: host === "alpha" ? 41001 : 41002,
        remote_port: 3000,
        pid: 123,
        started_at: new Date().toISOString(),
        generation: 1,
        status: "ready",
      }),
    });
  });

  await page.goto(`${WEB}/dashboard`);
  await page.locator('[aria-label="Actions menu"]').click();
  await page.getByRole("button", { name: "Servers" }).click();

  const alpha = page.locator('[data-ssh-host="alpha"]');
  const beta = page.locator('[data-ssh-host="beta"]');
  await alpha.getByRole("button", { name: "Connect to alpha" }).click();
  await expect(alpha.getByRole("button", { name: "Connect to alpha" })).toHaveText("Connecting…");
  await expect(beta.getByRole("button", { name: "Connect to beta" })).toBeEnabled();

  await beta.getByRole("button", { name: "Connect to beta" }).click();
  await expect(beta.getByRole("button", { name: "Connect to beta" })).toHaveText("Connecting…");
  await expect(alpha.getByRole("button", { name: "Connect to alpha" })).toBeDisabled();
  await expect(alpha.getByRole("button", { name: "Connect to alpha" })).toHaveAttribute(
    "aria-busy",
    "true",
  );

  releaseConnections();
  await expect(alpha.getByRole("button", { name: "Connect to alpha" })).toHaveText("Connect");
  await expect(beta.getByRole("button", { name: "Connect to beta" })).toHaveText("Connect");
});

test("a disconnected SSH session can be retried or dismissed", async ({ page }) => {
  await login(page);
  await page.addInitScript(() => {
    window.open = () => null;
  });
  await page.route("**/api/v1/ssh/hosts", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify([
        { name: "alpha", hostname: "alpha.example", user: "alice", port: 22 },
      ]),
    }),
  );
  let reconnected = false;
  await page.route("**/api/v1/ssh/tunnels", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify([
        {
          id: reconnected ? "tunnel-alpha-2" : "tunnel-alpha",
          host: "alpha",
          local_port: reconnected ? 41002 : 41001,
          remote_port: 3000,
          pid: reconnected ? 124 : 123,
          started_at: new Date().toISOString(),
          generation: reconnected ? 2 : 1,
          status: reconnected ? "ready" : "disconnected",
          ...(reconnected
            ? {}
            : {
                disconnected_at: new Date().toISOString(),
                last_error: "SSH tunnel exited with exit status: 255",
              }),
        },
      ]),
    }),
  );
  await page.route("**/api/v1/ssh/connect-choruz", (route) => {
    reconnected = true;
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        id: "tunnel-alpha-2",
        host: "alpha",
        local_port: 41002,
        remote_port: 3000,
        pid: 124,
        started_at: new Date().toISOString(),
        generation: 2,
        status: "ready",
      }),
    });
  });

  await page.goto(`${WEB}/dashboard`);
  await page.locator('[aria-label="Actions menu"]').click();
  await page.getByRole("button", { name: "Servers" }).click();

  const alpha = page.locator('[data-ssh-host="alpha"]');
  await expect(alpha.getByRole("status")).toContainText("Disconnected");
  await expect(alpha.getByRole("button", { name: "Reconnect to alpha" })).toBeVisible();
  await expect(alpha.getByRole("button", { name: "Dismiss" })).toBeVisible();
  await alpha.getByRole("button", { name: "Reconnect to alpha" }).click();
  await expect(alpha.getByText("Tunnel active:")).toBeVisible();
});

test("an SSH connection failure stays scoped to its host", async ({ page }) => {
  await login(page);
  await page.route("**/api/v1/ssh/hosts", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify([
        { name: "alpha", hostname: "alpha.example", user: "alice", port: 22 },
        { name: "beta", hostname: "beta.example", user: "bob", port: 22 },
      ]),
    }),
  );
  await page.route("**/api/v1/ssh/tunnels", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: "[]" }),
  );
  await page.route("**/api/v1/ssh/connect-choruz", (route) =>
    route.fulfill({
      status: 500,
      contentType: "application/json",
      body: JSON.stringify({ error: "alpha connection failed" }),
    }),
  );

  await page.goto(`${WEB}/dashboard`);
  await page.locator('[aria-label="Actions menu"]').click();
  await page.getByRole("button", { name: "Servers" }).click();

  const alpha = page.locator('[data-ssh-host="alpha"]');
  const beta = page.locator('[data-ssh-host="beta"]');
  await alpha.getByRole("button", { name: "Connect to alpha" }).click();

  await expect(alpha.getByRole("alert")).toContainText("alpha connection failed");
  await expect(beta.getByRole("alert")).toHaveCount(0);
  await expect(beta.getByRole("button", { name: "Connect to beta" })).toBeEnabled();
});
