import { expect, test } from "@playwright/test";
import { API_BASE, WEB_BASE, CREDENTIALS, login, gotoDashboard } from "../fixtures/auth";

test.describe("Authentication", () => {
  /* ---------------------------------------------------------------------- */
  /*  Local entry                                                            */
  /* ---------------------------------------------------------------------- */

  test("should enter the local dashboard without a login screen", async ({ page }) => {
    await page.goto(WEB_BASE);
    await expect(page.locator(".chat-sidebar, .chat-app")).toBeVisible({ timeout: 15_000 });
    await expect(page.getByRole("img", { name: "Choruz" })).toBeVisible();
    await expect(page.locator("#signin-panel, #signup-panel")).toHaveCount(0);
    expect(new URL(page.url()).pathname).toBe("/dashboard");
  });

  test("should reject wrong password via API", async ({ page }) => {
    const res = await page.request.post(`${API_BASE}/v1/auth/local/login`, {
      data: { username: "operator", password: "wrong-password" },
    });
    expect(res.ok()).toBeFalsy();
    expect(res.status()).toBeGreaterThanOrEqual(400);
  });

  /* ---------------------------------------------------------------------- */
  /*  Successful login                                                       */
  /* ---------------------------------------------------------------------- */

  test("should login via API and receive a session token", async ({ page }) => {
    const res = await page.request.post(`${API_BASE}/v1/auth/local/login`, {
      data: CREDENTIALS,
    });
    expect(res.ok()).toBeTruthy();
    const payload = await res.json();
    expect(payload.session_token).toBeTruthy();
    expect(payload.principal).toBeTruthy();
    expect(payload.principal.name).toBe("operator");
  });

  test("should access dashboard after cookie injection", async ({ page }) => {
    await login(page);
    await gotoDashboard(page);
    await expect(page.locator(".chat-sidebar, .chat-app")).toBeVisible();
  });

  test("should not expose the retired onboarding route", async ({ page }) => {
    await login(page);
    const response = await page.goto(`${WEB_BASE}/onboarding`);
    expect(response?.status()).toBe(404);
    await expect(page.getByText("This page could not be found.")).toBeVisible();
  });

  test("should replace the retired cookie with a local session", async ({ page }) => {
    const { token } = await login(page);
    await page.context().clearCookies();
    await page.context().addCookies([
      {
        name: "echat_session",
        value: token,
        url: WEB_BASE,
        httpOnly: true,
        sameSite: "Lax",
        expires: Math.floor(Date.now() / 1000) + 60 * 60,
      },
    ]);

    await page.goto(`${WEB_BASE}/dashboard`);
    await expect(page.locator(".chat-sidebar, .chat-app")).toBeVisible({ timeout: 15_000 });
    const cookies = await page.context().cookies(WEB_BASE);
    expect(cookies.some((cookie) => cookie.name === "choruz_session")).toBe(true);
  });

  test("should replace a malformed session and recover to dashboard", async ({ page }) => {
    await page.context().addCookies([
      {
        name: "choruz_session",
        value: "malformed-session-token",
        url: WEB_BASE,
        httpOnly: true,
        sameSite: "Lax",
        expires: Math.floor(Date.now() / 1000) + 60 * 60,
      },
    ]);

    await page.goto(`${WEB_BASE}/dashboard`);

    await expect(page.locator(".chat-sidebar, .chat-app")).toBeVisible({ timeout: 15_000 });
    expect(new URL(page.url()).pathname).toBe("/dashboard");
    const cookies = await page.context().cookies(WEB_BASE);
    const session = cookies.find((cookie) => cookie.name === "choruz_session");
    expect(session?.value).toBeTruthy();
    expect(session?.value).not.toBe("malformed-session-token");
  });

  test("should create and authenticate a second human account", async ({ page }) => {
    const operator = await login(page);
    const username = `h-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
    const password = "choruz-human-pass";
    const response = await page.request.post(`${API_BASE}/v1/auth/local/signup`, {
      data: { username, password },
    });
    expect(response.status()).toBe(201);
    const signup = await response.json();
    expect(signup.principal.name).toBe(username);
    expect(signup.principal.workspace_id).not.toBe(operator.principal.workspace_id);

    const loginResponse = await page.request.post(`${API_BASE}/v1/auth/local/login`, {
      data: { username, password },
    });
    expect(loginResponse.ok()).toBeTruthy();
    const authenticated = await loginResponse.json();
    expect(authenticated.principal.id).toBe(signup.principal.id);
    expect(authenticated.principal.workspace_id).toBe(signup.principal.workspace_id);
  });

  test("should display the logged-in username in sidebar", async ({ page }) => {
    await login(page);
    await gotoDashboard(page);
    await expect(page.locator(".sidebar-header").getByText("operator", { exact: true })).toBeVisible({
      timeout: 10_000,
    });
  });

  /* ---------------------------------------------------------------------- */
  /*  Session persistence                                                    */
  /* ---------------------------------------------------------------------- */

  test("should persist session across page reload", async ({ page }) => {
    await login(page);
    await gotoDashboard(page);
    await expect(page.locator(".chat-sidebar, .chat-app")).toBeVisible();
    // Reload and verify still logged in
    await page.reload();
    await expect(page.locator(".chat-sidebar, .chat-app")).toBeVisible({
      timeout: 15_000,
    });
  });

  test("should bootstrap an unauthenticated local browser", async ({ page }) => {
    await page.goto(`${WEB_BASE}/dashboard`);
    await expect(page.locator(".chat-sidebar, .chat-app")).toBeVisible({ timeout: 15_000 });
    expect(new URL(page.url()).pathname).toBe("/dashboard");
  });

  test("should return valid principal type from login", async ({ page }) => {
    const res = await page.request.post(`${API_BASE}/v1/auth/local/login`, {
      data: CREDENTIALS,
    });
    const payload = await res.json();
    expect(["human"]).toContain(payload.principal.principal_type);
  });
});
