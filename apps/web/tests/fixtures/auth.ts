import { expect, type Page } from "@playwright/test";

/* -------------------------------------------------------------------------- */
/*  Constants                                                                  */
/* -------------------------------------------------------------------------- */

export const API_BASE =
  process.env.CHORUZ_API_BASE_URL ?? `http://127.0.0.1:${process.env.CHORUZ_API_PORT ?? "3000"}`;
export const WEB_BASE =
  process.env.CHORUZ_WEB_BASE_URL ?? `http://127.0.0.1:${process.env.CHORUZ_WEB_PORT ?? "3100"}`;
export const CREDENTIALS = {
  username: process.env.CHORUZ_OPERATOR_USER ?? "operator",
  password: process.env.CHORUZ_OPERATOR_PASSWORD ?? "choruz-local",
};

export type LoginPrincipal = {
  id: string;
  name: string;
  workspace_id: string;
  principal_type: string;
};

export type GotoDashboardOptions = {
  expandSidebarSections?: boolean;
};

const CONVERSATION_SECTION_TITLES = [
  "Direct Messages",
  "Group Conversations",
] as const;

function escapeRegex(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/* -------------------------------------------------------------------------- */
/*  Login helper                                                               */
/* -------------------------------------------------------------------------- */

/**
 * Login via the API gateway and inject the session cookie into the browser
 * context so all subsequent page.goto() calls hit the app as an authenticated
 * user.  Returns the session token + principal payload for API calls.
 */
export async function login(page: Page) {
  const res = await page.request.post(`${API_BASE}/v1/auth/local/login`, {
    data: { username: CREDENTIALS.username, password: CREDENTIALS.password },
  });
  expect(res.ok()).toBeTruthy();
  const payload = await res.json();
  const token: string = payload.session_token;
  const principal: LoginPrincipal = payload.principal;

  await page.context().addCookies([
    {
      name: "choruz_session",
      value: token,
      url: WEB_BASE,
      httpOnly: true,
      sameSite: "Lax",
      expires: Math.floor(Date.now() / 1000) + 60 * 60,
    },
  ]);

  return { token, principal };
}

export async function signup(page: Page, username: string, password: string) {
  const res = await page.request.post(`${API_BASE}/v1/auth/local/signup`, {
    data: { username, password },
  });
  expect(res.ok(), `signup ${username} -> ${res.status()}: ${await res.text().catch(() => "")}`).toBeTruthy();
  const payload = await res.json();
  const token: string = payload.session_token;
  const principal: LoginPrincipal = payload.principal;

  await page.context().addCookies([
    {
      name: "choruz_session",
      value: token,
      url: WEB_BASE,
      httpOnly: true,
      sameSite: "Lax",
      expires: Math.floor(Date.now() / 1000) + 60 * 60,
    },
  ]);

  return { token, principal };
}

/**
 * Navigate to the dashboard and wait for the main UI to appear.
 */
export async function gotoDashboard(
  page: Page,
  { expandSidebarSections = true }: GotoDashboardOptions = {},
) {
  await page.goto(`${WEB_BASE}/dashboard`);
  // Wait for sidebar or main layout to be rendered
  await page.waitForSelector(".chat-sidebar, .chat-app", { timeout: 15_000 });
  const viewport = page.viewportSize();
  if (expandSidebarSections && (!viewport || viewport.width >= 1024)) {
    await expandSidebarConversationSections(page);
  }
}

export async function expandSidebarConversationSections(page: Page) {
  for (const title of CONVERSATION_SECTION_TITLES) {
    const header = page
      .getByRole("group", { name: title })
      .getByRole("button", { name: new RegExp(`^${escapeRegex(title)}`) });
    await expect(header).toBeVisible({ timeout: 10_000 });
    if ((await header.getAttribute("aria-expanded")) !== "true") {
      await header.click();
    }
  }
}
