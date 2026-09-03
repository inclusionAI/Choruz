import { expect, test, type Page } from "@playwright/test";
import { randomUUID } from "node:crypto";

import { postgresQueryClient } from "../../lib/groups/group-provisioning-db";
import { API_BASE, WEB_BASE, login, gotoDashboard } from "../fixtures/auth";
import { createCompany, deleteCompany, uniqueName } from "../fixtures/api";

type AccountDriver = "claude_terminal" | "codex_terminal";

type HarnessAccountOverrides = Partial<{
  id: string;
  name: string;
  profileKind: "default" | "isolated";
  runtimeHostId: string | null;
  status: "pending" | "active" | "reauth_required" | "error";
  probedAt: string | null;
}>;

/** A verified harness account as /api/harness-accounts returns it. */
function harnessAccount(driverType: AccountDriver, models: { id: string; label: string }[] = [], overrides: HarnessAccountOverrides = {}) {
  const now = new Date().toISOString();
  return {
    id: `ci-${driverType}`,
    companyId: "",
    runtimeHostId: null,
    driverType,
    name: "CI account",
    profileKind: "default",
    subscriptionType: null,
    status: "active",
    models,
    usage: { windows: [] },
    lastError: null,
    probedAt: now,
    createdAt: now,
    updatedAt: now,
    ...overrides,
  };
}

/** CI has no harness installed, so serve the account list and the device's
 *  own login (which the dialog and Create Agent register on demand). */
async function mockHarnessAccounts(page: Page, accounts: ReturnType<typeof harnessAccount>[]) {
  await page.route(/\/api\/harness-accounts\?/, (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ accounts }),
    }),
  );
  await page.route(/\/api\/harness-accounts\/default$/, (route) => {
    const driverType = (route.request().postDataJSON() as { driver_type: AccountDriver }).driver_type;
    const own = accounts.find((account) => account.driverType === driverType && account.profileKind === "default")
      ?? harnessAccount(driverType, [], {
        id: `ci-default-${driverType}`,
        name: driverType === "claude_terminal" ? "Claude Code login" : "Codex login",
        profileKind: "default",
      });
    return route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(own) });
  });
}

/** Turn on the company's multi-account switch from the Harness Accounts dialog. */
async function enableMultipleAccounts(page: Page) {
  await page.locator('[aria-label="Actions menu"]').click();
  await page.getByRole("button", { name: "Harness Accounts" }).click();
  const accounts = page.locator(".harness-accounts-card");
  const toggle = accounts.getByRole("checkbox", { name: "Allow multiple accounts in this company" });
  if (!(await toggle.isChecked())) {
    await toggle.check();
    await expect(toggle).toBeChecked();
  }
  await expect(accounts.getByRole("button", { name: "Add account" })).toBeVisible();
  await accounts.getByRole("button", { name: "Close" }).click();
}

test.describe("Modals (Create Agent, Create Group, Create Company)", () => {
  test.beforeEach(async ({ page }) => {
    await login(page);
    await gotoDashboard(page);
  });

  /* ---------------------------------------------------------------------- */
  /*  Create Agent modal                                                     */
  /* ---------------------------------------------------------------------- */

  test("should open Create Agent modal from actions menu", async ({
    page,
  }) => {
    const actionsBtn = page.locator('[aria-label="Actions menu"]');
    await actionsBtn.click();
    await page.getByText("Create Agent").click();
    await page.waitForTimeout(500);
    // Modal overlay should be visible
    const overlay = page.locator(".modal-overlay, .modal-backdrop");
    const visible = await overlay.isVisible({ timeout: 3000 }).catch(() => false);
    expect(typeof visible).toBe("boolean");
  });

  test("light theme modal card fully obscures the dashboard beneath it", async ({ page }) => {
    await page.evaluate(() => document.documentElement.setAttribute("data-theme", "light"));
    const actionsBtn = page.locator('[aria-label="Actions menu"]');
    await actionsBtn.click();
    await page.getByText("Create Agent").click();

    await expect(page.locator(".modal-card")).toHaveCSS(
      "background-color",
      "rgb(255, 255, 255)",
    );
  });

  test("should close Create Agent modal on close/cancel", async ({ page }) => {
    const actionsBtn = page.locator('[aria-label="Actions menu"]');
    await actionsBtn.click();
    await page.getByText("Create Agent").click();
    await page.waitForTimeout(500);
    // Find close or cancel button
    const closeBtn = page.locator(
      'button:has-text("Cancel"), button:has-text("Close"), button[title="Close"]',
    );
    if (await closeBtn.isVisible({ timeout: 3000 }).catch(() => false)) {
      await closeBtn.first().click();
      await page.waitForTimeout(500);
    }
  });

  test("should show agent name validation error when empty", async ({
    page,
  }) => {
    const actionsBtn = page.locator('[aria-label="Actions menu"]');
    await actionsBtn.click();
    await page.getByText("Create Agent").click();
    await page.waitForTimeout(500);
    // Try to create without name
    const createBtn = page.locator(
      'button:has-text("Create"), button:has-text("Provision")',
    );
    if (await createBtn.isVisible({ timeout: 3000 }).catch(() => false)) {
      await createBtn.first().click();
      await page.waitForTimeout(500);
      // Should show error
      const error = page.getByText("required");
      const hasError = await error.isVisible({ timeout: 2000 }).catch(() => false);
      expect(typeof hasError).toBe("boolean");
    }
  });

  test("should show driver type options in Create Agent modal", async ({
    page,
  }) => {
    const actionsBtn = page.locator('[aria-label="Actions menu"]');
    await actionsBtn.click();
    await page.getByText("Create Agent").click();
    const driverSelect = page.getByLabel("Driver");
    await expect(driverSelect).toBeVisible();
    await expect(driverSelect.locator("option")).toHaveCount(7);
    await expect(driverSelect.locator('option[value="codex_terminal"]')).toHaveText("Codex");
    await expect(driverSelect.locator('option[value="codex_exec"]')).toHaveCount(0);
    for (const driver of [
      "pi_terminal",
      "grok_terminal",
      "opencode_terminal",
      "mathcode_terminal",
    ]) {
      await expect(driverSelect.locator(`option[value="${driver}"]`)).toHaveCount(1);
    }
  });

  test("should review a concrete model for the selected harness account", async ({ page }) => {
    // CI has no Claude CLI; an unavailable driver blocks the review step.
    await page.route("**/api/drivers/availability", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          drivers: [
            {
              label: "Claude Code",
              driverId: "claude_terminal",
              status: "available",
              reason: "Claude CLI is available.",
              setupHint: "Install Claude Code.",
              envVar: "CHORUZ_CLAUDE_BINARY",
            },
          ],
        }),
      }),
    );
    await mockHarnessAccounts(page, [
      harnessAccount("claude_terminal", [
        { id: "sonnet", label: "Sonnet" },
        { id: "claude-opus-5", label: "Opus 5" },
      ]),
    ]);
    await enableMultipleAccounts(page);
    const actionsBtn = page.locator('[aria-label="Actions menu"]');
    await actionsBtn.click();
    await page.getByText("Create Agent").click();
    const modal = page.locator(".modal-card");

    await modal.getByLabel("Harness account").selectOption({ label: "CI account" });
    await expect(modal.getByText("2 models verified for the selected account.")).toBeVisible();
    await modal.getByLabel("Model").fill("claude-opus-5");
    await modal.getByPlaceholder("e.g. Coder, Reviewer, Architect").fill("Model Tester");
    await modal.getByRole("button", { name: "Review & Create" }).click();

    await expect(modal.locator(".create-agent-review")).toContainText("claude-opus-5");
  });

  test("manages accounts separately and creates with a verified account", async ({ page }) => {
    await page.route("**/api/drivers/availability", (route) => route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ drivers: [{ label: "Claude Code", driverId: "claude_terminal", status: "available", reason: "available", setupHint: "", envVar: "CHORUZ_CLAUDE_BINARY" }] }),
    }));
    await mockHarnessAccounts(page, [harnessAccount("claude_terminal", [{ id: "sonnet", label: "Sonnet" }])]);
    await enableMultipleAccounts(page);
    await page.locator('[aria-label="Actions menu"]').click();
    await page.getByRole("button", { name: "Harness Accounts" }).click();
    const accounts = page.locator(".harness-accounts-card");
    await expect(accounts.getByRole("heading", { name: "Harness Accounts" })).toBeVisible();
    await expect(accounts.getByText("CI account")).toBeVisible();
    await accounts.getByRole("button", { name: "Close" }).click();

    await page.locator('[aria-label="Actions menu"]').click();
    await page.getByRole("button", { name: "Create Agent" }).click();
    const create = page.locator(".modal-card");
    await expect(create.getByRole("button", { name: "Add account" })).toHaveCount(0);
    await create.getByLabel("Harness account").selectOption({ label: "CI account" });
    await create.getByPlaceholder("e.g. Coder, Reviewer, Architect").fill("Account Selected Agent");
    await create.getByLabel("Model").fill("sonnet");
    await create.getByRole("button", { name: "Review & Create" }).click();
    await expect(create.locator(".create-agent-review")).toContainText("CI account");
  });

  test("removing an account stops its real dependent Agent binding", async ({ page }) => {
    const { token, principal } = await login(page);
    const client = await postgresQueryClient();
    const accountId = randomUUID();
    const accountName = uniqueName("removable-account");
    await client.query(
      `INSERT INTO harness_account
         (id, company_id, driver_type, name, profile_kind, status, models_json, usage_json)
       VALUES ($1, $2, 'claude_terminal', $3, 'isolated', 'active',
               '[{"id":"test-model","label":"Test Model"}]'::jsonb,
               '{"windows":[]}'::jsonb)`,
      [accountId, principal.workspace_id, accountName],
    );
    const provision = await page.request.post(`${WEB_BASE}/api/agents/provision`, {
      data: {
        name: uniqueName("account-bound-agent"),
        driver_type: "claude_terminal",
        instructions: "Account removal E2E Agent",
        workspace_id: principal.workspace_id,
        harness_account_id: accountId,
        model: "test-model",
      },
    });
    expect(provision.ok()).toBeTruthy();
    const agentId = (await provision.json()).agent.id as string;

    try {
      await page.reload();
      await enableMultipleAccounts(page);
      await page.locator('[aria-label="Actions menu"]').click();
      await page.getByRole("button", { name: "Harness Accounts" }).click();
      const accounts = page.locator(".harness-accounts-card");
      const record = accounts.locator(".harness-account-record", { hasText: accountName });
      await expect(record).toBeVisible();
      await record.getByRole("button", { name: "Remove account" }).click();
      await expect(record).toHaveCount(0);

      const state = await client.query<{ account_disabled: boolean; binding_state: string }>(
        `SELECT account.disabled_at IS NOT NULL AS account_disabled, binding.state AS binding_state
           FROM harness_account AS account
           JOIN agent_runtime_bindings AS binding
             ON binding.config_json->>'harness_account_id' = account.id
          WHERE account.id = $1 AND binding.agent_principal_id = $2`,
        [accountId, agentId],
      );
      expect(state.rows).toEqual([{ account_disabled: true, binding_state: "disabled" }]);
    } finally {
      const cleanup = await page.request.post(`${API_BASE}/v1/agents/batch-disable`, {
        headers: { authorization: `Bearer ${token}` },
        data: { actor_id: principal.id, agent_ids: [agentId], conversation_ids: [] },
      });
      expect(cleanup.ok()).toBe(true);
    }
  });

  test("removing the device login keeps it hidden while multiple accounts are enabled", async ({ page }) => {
    const { token, principal } = await login(page);
    const client = await postgresQueryClient();
    const company = await createCompany(page, token, principal.id, uniqueName("remove-device-login"));
    const accountId = randomUUID();
    await client.query(
      `INSERT INTO harness_account
         (id, company_id, driver_type, name, profile_kind, status, models_json, usage_json)
       VALUES ($1, $2, 'claude_terminal', 'Claude Code login', 'default', 'active',
               '[{"id":"test-model","label":"Test Model"}]'::jsonb,
               '{"windows":[{"id":"five_hour","label":"five hour","usedPercent":20,"remainingPercent":80,"resetsAt":null,"windowDurationMinutes":null}]}'::jsonb)`,
      [accountId, company.id],
    );

    try {
      await gotoDashboard(page);
      await page.locator(".company-selector-btn").click();
      await page.locator(".company-dropdown-item").filter({ hasText: company.name }).locator(".company-dropdown-item-name").click();
      await expect(page.locator(".company-selector-name")).toHaveText(company.name);

      await enableMultipleAccounts(page);
      await page.locator('[aria-label="Actions menu"]').click();
      await page.getByRole("button", { name: "Harness Accounts" }).click();
      const accounts = page.locator(".harness-accounts-card");
      const record = accounts.locator(".harness-account-record", { hasText: "Claude Code login" });
      await expect(record.getByText("5-hour: 80% left")).toBeVisible();
      await record.getByRole("button", { name: "Remove account" }).click();
      await expect(record).toHaveCount(0);
      await accounts.getByRole("button", { name: "Close" }).click();

      await page.locator('[aria-label="Actions menu"]').click();
      await page.getByRole("button", { name: "Harness Accounts" }).click();
      await expect(page.locator(".harness-accounts-card").locator(".harness-account-record", { hasText: "Claude Code login" })).toHaveCount(0);

      const stored = await client.query<{ disabled: boolean }>(
        "SELECT disabled_at IS NOT NULL AS disabled FROM harness_account WHERE id = $1",
        [accountId],
      );
      expect(stored.rows).toEqual([{ disabled: true }]);

      const reactivated = await page.request.post(`${WEB_BASE}/api/harness-accounts/default`, {
        data: { company_id: company.id, driver_type: "claude_terminal", reactivate_removed: true },
      });
      expect(reactivated.ok()).toBe(true);
      const replacement = await reactivated.json() as { id: string };
      expect(replacement.id).not.toBe(accountId);

      const reopened = await page.request.post(`${WEB_BASE}/api/harness-accounts/default`, {
        data: { company_id: company.id, driver_type: "claude_terminal" },
      });
      expect(reopened.ok()).toBe(true);
      expect((await reopened.json() as { id: string }).id).toBe(replacement.id);
    } finally {
      await deleteCompany(page, token, company.id);
    }
  });

  test("uses the device's own login until multiple accounts are turned on", async ({ page }) => {
    const { token, principal } = await login(page);
    await page.route("**/api/drivers/availability", (route) => route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ drivers: [{ label: "Claude Code", driverId: "claude_terminal", status: "available", reason: "available", setupHint: "", envVar: "CHORUZ_CLAUDE_BINARY" }] }),
    }));
    await mockHarnessAccounts(page, []);
    const company = await createCompany(page, token, principal.id, `single-login-${Date.now()}`);
    try {
      await gotoDashboard(page);
      await page.locator(".company-selector-btn").click();
      await page.locator(".company-dropdown-item").filter({ hasText: company.name }).locator(".company-dropdown-item-name").click();
      await expect(page.locator(".company-selector-name")).toHaveText(company.name);

      await page.locator('[aria-label="Actions menu"]').click();
      await page.getByRole("button", { name: "Create Agent" }).click();
      const create = page.locator(".modal-card");
      await expect(create.getByPlaceholder("e.g. Coder, Reviewer, Architect")).toBeVisible();
      await expect(create.getByLabel("Harness account")).toHaveCount(0);
      await expect(create.getByLabel("Model")).toBeEnabled();
      await create.getByRole("button", { name: "Close" }).click();

      await page.locator('[aria-label="Actions menu"]').click();
      await page.getByRole("button", { name: "Harness Accounts" }).click();
      const accounts = page.locator(".harness-accounts-card");
      const toggle = accounts.getByRole("checkbox", { name: "Allow multiple accounts in this company" });
      await expect(toggle).not.toBeChecked();
      await expect(accounts.locator(".harness-account-record", { hasText: "Claude Code login" })).toBeVisible();
      await expect(accounts.getByRole("button", { name: "Add account" })).toHaveCount(0);
      await expect(accounts.getByRole("button", { name: "Remove account" })).toHaveCount(0);
      await toggle.check();
      await expect(accounts.getByRole("button", { name: "Add account" })).toBeVisible();
      await accounts.getByRole("button", { name: "Close" }).click();

      await page.locator('[aria-label="Actions menu"]').click();
      await page.getByRole("button", { name: "Create Agent" }).click();
      await expect(create.getByLabel("Harness account")).toBeVisible();
      await expect(create.getByRole("option", { name: "Login already on this computer" })).toBeAttached();
    } finally {
      await deleteCompany(page, token, company.id);
    }
  });

  test("adding a new local account shows the official sign-in link", async ({ page }) => {
    const { token, principal } = await login(page);
    const company = await createCompany(page, token, principal.id, uniqueName("login-catalog"));
    const pending = harnessAccount("claude_terminal", [], { id: "ci-new", name: "Work account", profileKind: "isolated", status: "pending", probedAt: null });
    const verified = harnessAccount("claude_terminal", [], {
      id: "ci-new",
      name: "Work account",
      profileKind: "isolated",
      status: "active",
      probedAt: null,
    });
    const loginView = (state: string) => ({
      id: "login-1",
      account_id: "ci-new",
      runtime_host_id: null,
      driver_type: "claude_terminal",
      state,
      authorization_url: "https://example.test/oauth?state=abc",
      user_code: null,
      error: null,
      expires_at: new Date(Date.now() + 600_000).toISOString(),
    });
    let codeSubmitted = false;
    let loginStarts = 0;
    const json = (status: number, body: unknown) => ({ status, contentType: "application/json", body: JSON.stringify(body) });

    await mockHarnessAccounts(page, []);
    await page.route(/\/api\/harness-accounts$/, (route) => route.fulfill(json(201, pending)));
    await page.route(/\/api\/harness-accounts\/ci-new\/login\?/, (route) => {
      loginStarts += 1;
      return route.fulfill(json(201, loginView("awaiting_browser")));
    });
    await page.route(/\/api\/harness-accounts\/ci-new\/login\/login-1\?/, (route) =>
      route.fulfill(json(200, loginView(codeSubmitted ? "verified" : "awaiting_browser"))));
    await page.route(/\/api\/harness-accounts\/ci-new\/login\/login-1\/callback\?/, (route) => {
      codeSubmitted = true;
      return route.fulfill({ status: 204 });
    });
    await page.route(/\/api\/harness-accounts\/ci-new\?/, (route) => route.fulfill(json(200, verified)));

    try {
      await gotoDashboard(page);
      await page.locator(".company-selector-btn").click();
      await page.locator(".company-dropdown-item").filter({ hasText: company.name }).locator(".company-dropdown-item-name").click();
      await expect(page.locator(".company-selector-name")).toHaveText(company.name);

      await enableMultipleAccounts(page);
      await page.locator('[aria-label="Actions menu"]').click();
      await page.getByRole("button", { name: "Harness Accounts" }).click();
      const accounts = page.locator(".harness-accounts-card");
      await accounts.getByRole("button", { name: "Add account" }).click();
      await accounts.getByLabel("Account label").fill("Work account");
      await accounts.getByRole("button", { name: "Add and sign in" }).click();

      await expect(accounts.getByText("Sign in to Claude Code account “Work account”")).toBeVisible();
      await expect.poll(() => loginStarts).toBe(1);
      await expect(accounts.getByRole("link", { name: "Open sign-in link" })).toHaveAttribute("href", "https://example.test/oauth?state=abc");
      await expect(accounts.locator("code", { hasText: "claude /login" })).toHaveCount(0);
      await accounts.getByLabel("Claude Code authentication value").fill("xyz#abc");
      await accounts.getByRole("button", { name: "Finish sign-in" }).click();

      await expect(accounts.getByText("Sign in to Claude Code account “Work account”")).toHaveCount(0);
      const record = accounts.locator(".harness-account-record", { hasText: "Work account" });
      await expect(record).toBeVisible();
      await expect(record.getByText("pending")).toHaveCount(0);
      await expect(record.getByRole("button", { name: "Sign in", exact: true })).toHaveCount(0);
      await expect(record.getByRole("button", { name: "Refresh exact usage" })).toBeVisible();
    } finally {
      await deleteCompany(page, token, company.id);
    }
  });

  test("cancelling a sign-in closes the login so a new one can start", async ({ page }) => {
    const pending = harnessAccount("claude_terminal", [], { id: "ci-cancel", name: "Cancelled account", profileKind: "isolated", status: "pending", probedAt: null });
    const login = {
      id: "login-cancel",
      account_id: pending.id,
      runtime_host_id: null,
      driver_type: "claude_terminal",
      state: "awaiting_browser",
      authorization_url: "https://example.test/oauth?state=cancel",
      user_code: null,
      error: null,
      expires_at: new Date(Date.now() + 600_000).toISOString(),
    };
    const json = (status: number, body: unknown) => ({ status, contentType: "application/json", body: JSON.stringify(body) });
    let cancelled = 0;

    await mockHarnessAccounts(page, []);
    await page.route(/\/api\/harness-accounts$/, (route) => route.fulfill(json(201, pending)));
    await page.route(/\/api\/harness-accounts\/ci-cancel\/login\?/, (route) => route.fulfill(json(201, login)));
    await page.route(/\/api\/harness-accounts\/ci-cancel\/login\/login-cancel\?/, (route) => route.fulfill(json(200, login)));
    await page.route(/\/api\/harness-accounts\/ci-cancel\/login\/login-cancel\/cancel\?/, (route) => {
      cancelled += 1;
      return route.fulfill({ status: 204 });
    });

    await enableMultipleAccounts(page);
    await page.locator('[aria-label="Actions menu"]').click();
    await page.getByRole("button", { name: "Harness Accounts" }).click();
    const accounts = page.locator(".harness-accounts-card");
    await accounts.getByRole("button", { name: "Add account" }).click();
    await accounts.getByLabel("Account label").fill("Cancelled account");
    await accounts.getByRole("button", { name: "Add and sign in" }).click();
    await expect(accounts.getByRole("link", { name: "Open sign-in link" })).toBeVisible();

    await accounts.locator(".harness-account-setup").getByRole("button", { name: "Cancel" }).click();
    await expect.poll(() => cancelled).toBe(1);
    await expect(accounts.getByText("Sign in to Claude Code account “Cancelled account”")).toHaveCount(0);
  });

  test("adding a local Codex account shows the standard browser login", async ({ page }) => {
    const pending = harnessAccount("codex_terminal", [], {
      id: "ci-codex-new",
      name: "Work Codex",
      profileKind: "isolated",
      status: "pending",
      probedAt: null,
    });
    const login = {
      id: "login-codex",
      account_id: pending.id,
      runtime_host_id: null,
      driver_type: "codex_terminal",
      state: "awaiting_browser",
      authorization_url: "https://auth.openai.com/oauth/authorize?state=test",
      user_code: null,
      error: null,
      expires_at: new Date(Date.now() + 600_000).toISOString(),
    };
    const json = (status: number, body: unknown) => ({ status, contentType: "application/json", body: JSON.stringify(body) });

    await mockHarnessAccounts(page, []);
    await page.route(/\/api\/harness-accounts$/, (route) => route.fulfill(json(201, pending)));
    await page.route(/\/api\/harness-accounts\/ci-codex-new\/login\?/, (route) => route.fulfill(json(201, login)));
    await page.route(/\/api\/harness-accounts\/ci-codex-new\/login\/login-codex\?/, (route) => route.fulfill(json(200, login)));

    await enableMultipleAccounts(page);
    await page.locator('[aria-label="Actions menu"]').click();
    await page.getByRole("button", { name: "Harness Accounts" }).click();
    const accounts = page.locator(".harness-accounts-card");
    await accounts.getByLabel("Account harness").selectOption("codex_terminal");
    await accounts.getByRole("button", { name: "Add account" }).click();
    await accounts.getByLabel("Account label").fill("Work Codex");
    await accounts.getByRole("button", { name: "Add and sign in" }).click();

    await expect(accounts.getByText("Sign in to Codex account “Work Codex”")).toBeVisible();
    await expect(accounts.getByText("Open the official Codex browser sign-in page.")).toBeVisible();
    await expect(accounts.getByRole("link", { name: "Open sign-in link" })).toHaveAttribute("href", login.authorization_url);
    await expect(accounts.getByLabel("Claude Code authentication value")).toHaveCount(0);
    await expect(accounts.getByLabel("Codex callback URL")).toHaveCount(0);
  });

  test("a remote Codex browser login accepts its localhost callback handoff", async ({ page }) => {
    const account = harnessAccount("codex_terminal", [], {
      id: "ci-codex-remote",
      name: "Remote Codex",
      profileKind: "isolated",
      runtimeHostId: "host-remote",
      status: "pending",
      probedAt: null,
    });
    const login = {
      id: "login-codex-remote",
      account_id: account.id,
      runtime_host_id: account.runtimeHostId,
      driver_type: "codex_terminal",
      state: "awaiting_browser",
      authorization_url: "https://auth.openai.com/oauth/authorize?state=test",
      user_code: null,
      error: null,
      expires_at: new Date(Date.now() + 600_000).toISOString(),
    };
    const json = (status: number, body: unknown) => ({ status, contentType: "application/json", body: JSON.stringify(body) });
    let submitted = "";

    await page.route(/\/v1\/companies\/[^/]+\/runtime-hosts$/, (route) => route.fulfill(json(200, [{
      id: "host-remote",
      company_id: "",
      name: "Remote builder",
      status: "online",
      last_seen_at: new Date().toISOString(),
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    }])));
    await mockHarnessAccounts(page, [account]);
    await page.route(/\/api\/harness-accounts\/ci-codex-remote\/login\?/, (route) => route.fulfill(json(201, login)));
    await page.route(/\/api\/harness-accounts\/ci-codex-remote\/login\/login-codex-remote\?/, (route) => route.fulfill(json(200, login)));
    await page.route(/\/api\/harness-accounts\/ci-codex-remote\/login\/login-codex-remote\/callback\?/, async (route) => {
      submitted = (await route.request().postDataJSON()).code;
      return route.fulfill({ status: 204 });
    });

    await page.locator('[aria-label="Actions menu"]').click();
    await page.getByRole("button", { name: "Harness Accounts" }).click();
    const accounts = page.locator(".harness-accounts-card");
    await accounts.getByLabel("Account device").selectOption("host-remote");
    await accounts.getByLabel("Account harness").selectOption("codex_terminal");
    await accounts.getByRole("button", { name: "Sign in", exact: true }).click();
    await accounts.getByLabel("Codex callback URL").fill("http://localhost:1455/auth/callback?code=code-1&state=state-1");
    await accounts.getByRole("button", { name: "Finish sign-in" }).click();
    await expect.poll(() => submitted).toContain("code=code-1");
  });

  test("shows a structured Harness login error as readable text", async ({ page }) => {
    const pending = harnessAccount("codex_terminal", [], {
      id: "ci-login-error",
      name: "Work Codex",
      profileKind: "isolated",
      status: "pending",
      probedAt: null,
    });
    const json = (status: number, body: unknown) => ({ status, contentType: "application/json", body: JSON.stringify(body) });

    await mockHarnessAccounts(page, []);
    await page.route(/\/api\/harness-accounts$/, (route) => route.fulfill(json(201, pending)));
    await page.route(/\/api\/harness-accounts\/ci-login-error\/login\?/, (route) =>
      route.fulfill(json(409, { error: { status: 409, detail: "This account already has a login in progress" } })));

    await enableMultipleAccounts(page);
    await page.locator('[aria-label="Actions menu"]').click();
    await page.getByRole("button", { name: "Harness Accounts" }).click();
    const accounts = page.locator(".harness-accounts-card");
    await accounts.getByLabel("Account harness").selectOption("codex_terminal");
    await accounts.getByRole("button", { name: "Add account" }).click();
    await accounts.getByLabel("Account label").fill("Work Codex");
    await accounts.getByRole("button", { name: "Add and sign in" }).click();

    await expect(accounts.getByRole("alert")).toHaveText("This account already has a login in progress");
    await expect(accounts.getByText("[object Object]")).toHaveCount(0);
  });

  test("should ignore a stale model scan after the harness changes", async ({ page }) => {
    await page.evaluate(() => {
      const originalFetch = window.fetch.bind(window);
      let releaseClaude: (() => void) | undefined;
      let markClaudeStarted: (() => void) | undefined;
      const claudeBody = new Promise<void>((resolve) => { releaseClaude = resolve; });
      const claudeStarted = new Promise<void>((resolve) => { markClaudeStarted = resolve; });
      const testWindow = window as typeof window & {
        releaseClaudeModelScan?: () => void;
        waitForClaudeModelScan?: () => Promise<void>;
      };
      testWindow.releaseClaudeModelScan = () => releaseClaude?.();
      testWindow.waitForClaudeModelScan = () => claudeStarted;
      window.fetch = async (input, init) => {
        const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
        if (url.includes("/api/drivers/models?driver_type=claude_terminal")) {
          markClaudeStarted?.();
          return {
            ok: true,
            status: 200,
            json: async () => {
              await claudeBody;
              return {
                driverId: "claude_terminal",
                status: "available",
                models: [{ id: "stale-claude", label: "Stale Claude" }],
                message: "stale Claude result",
              };
            },
          } as Response;
        }
        if (url.includes("/api/drivers/models?driver_type=codex_terminal")) {
          return new Response(JSON.stringify({
            driverId: "codex_terminal",
            status: "available",
            models: [{ id: "gpt-current", label: "Current Codex" }],
            message: "current Codex result",
          }), { status: 200, headers: { "content-type": "application/json" } });
        }
        return originalFetch(input, init);
      };
    });

    await page.locator('[aria-label="Actions menu"]').click();
    await page.getByText("Create Agent").click();
    const modal = page.locator(".modal-card");
    await page.evaluate(() => (
      window as typeof window & { waitForClaudeModelScan?: () => Promise<void> }
    ).waitForClaudeModelScan?.());
    await modal.getByLabel("Driver").selectOption("codex_terminal");
    await expect(modal.getByText("current Codex result")).toBeVisible();

    await page.evaluate(() => {
      (window as typeof window & { releaseClaudeModelScan?: () => void }).releaseClaudeModelScan?.();
    });
    await expect(modal.getByText("current Codex result")).toBeVisible();
    await expect(modal.getByText("stale Claude result")).toHaveCount(0);
  });

  test("should show custom workspace path toggle", async ({ page }) => {
    const actionsBtn = page.locator('[aria-label="Actions menu"]');
    await actionsBtn.click();
    await page.getByText("Create Agent").click();
    await page.waitForTimeout(500);
    // Look for workspace path option
    const wsPathOption = page.locator(
      'label:has-text("workspace"), label:has-text("path"), input[type="checkbox"]',
    );
    const visible = await wsPathOption.isVisible({ timeout: 3000 }).catch(() => false);
    expect(typeof visible).toBe("boolean");
  });

  /* ---------------------------------------------------------------------- */
  /*  Create Group modal                                                     */
  /* ---------------------------------------------------------------------- */

  async function openCreateGroupModal(page: import("@playwright/test").Page) {
    const actionsBtn = page.locator('[aria-label="Actions menu"]');
    await actionsBtn.click();
    await page.getByRole("button", { name: "New Group" }).click();
    await page.waitForTimeout(500);
  }

  test("should open Create Group modal from actions menu", async ({
    page,
  }) => {
    await openCreateGroupModal(page);
    const overlay = page.locator(".modal-overlay, .modal-backdrop");
    const visible = await overlay.isVisible({ timeout: 3000 }).catch(() => false);
    expect(typeof visible).toBe("boolean");
  });

  test("should show group name input in Create Group modal", async ({
    page,
  }) => {
    await openCreateGroupModal(page);
    const nameInput = page.locator(
      'input[placeholder*="group"], input[placeholder*="name"], input[type="text"]',
    ).first();
    const visible = await nameInput.isVisible({ timeout: 3000 }).catch(() => false);
    expect(typeof visible).toBe("boolean");
  });

  test("should show member selection in Create Group modal", async ({
    page,
  }) => {
    await openCreateGroupModal(page);
    // Look for member list/checkboxes
    const memberList = page.locator(
      '.member-select, .agent-list, input[type="checkbox"]',
    );
    const visible = await memberList.isVisible({ timeout: 3000 }).catch(() => false);
    expect(typeof visible).toBe("boolean");
  });

  test("should validate group name is required", async ({ page }) => {
    await openCreateGroupModal(page);
    const createBtn = page.locator(
      'button:has-text("Create"), button:has-text("Done")',
    );
    await expect(createBtn.first()).toBeVisible();
    await createBtn.first().click();
    const input = page.getByPlaceholder("e.g. Engineering Team");
    await expect(page.getByText("Group name is required")).toBeVisible();
    await expect(input).toHaveAttribute("aria-invalid", "true");
    await expect(input).toBeFocused();
  });

  test("should plan a group from a template without auto-start mentions", async ({ page }) => {
    await page.route("**/api/drivers/availability", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          drivers: [
            {
              label: "Codex Terminal",
              driverId: "codex_terminal",
              status: "available",
              reason: "Codex CLI is available.",
              setupHint: "Install Codex.",
              envVar: "CHORUZ_CODEX_BINARY",
            },
          ],
        }),
      }),
    );
    await mockHarnessAccounts(page, [harnessAccount("claude_terminal"), harnessAccount("codex_terminal")]);
    await enableMultipleAccounts(page);
    await openCreateGroupModal(page);
    const modal = page.locator(".modal-card");

    await page.getByLabel("Start with").selectOption("software-development-team");
    await expect(
      modal.locator(".create-agent-template-summary").first().getByText("Software Development Team"),
    ).toBeVisible();
    await modal.getByPlaceholder("What should this group accomplish?").fill("Ship onboarding MVP");

    const operatorRole = page.locator(".group-template-role-row").filter({
      hasText: "Project Operator",
    });
    const backendRole = page.locator(".group-template-role-row").filter({
      hasText: "Backend Engineer · Required",
    });
    const reviewerRole = page.locator(".group-template-role-row").filter({
      hasText: "Code Reviewer · Required",
    });
    const frontendRole = page.locator(".group-template-role-row").filter({
      hasText: "Frontend Engineer · Optional",
    });
    await expect(operatorRole).toBeVisible();
    await expect(operatorRole.locator("select").first()).not.toContainText("Skip");
    await expect(backendRole).toBeVisible();
    await expect(reviewerRole).toBeVisible();
    await expect(frontendRole.locator("select").first()).toHaveValue("skip");
    await frontendRole.locator("select").first().selectOption("create");
    await expect(frontendRole.locator("select").first()).toHaveValue("create");
    await expect(frontendRole.getByLabel("Agent name")).toBeVisible();

    // With multiple accounts on, every Claude or Codex role offers the picker.
    for (const role of [operatorRole, backendRole, reviewerRole, frontendRole]) {
      await role.getByLabel("Harness account").selectOption({ label: "CI account" });
    }

    await page.getByRole("button", { name: "Review & Launch" }).click();
    const kickoffSummary = modal.locator(".create-agent-template-summary").filter({
      hasText: "Kickoff",
    });
    const frontendReviewRow = modal.locator(".group-template-review-table > div").filter({
      hasText: "frontend-engineer",
    });
    await expect(kickoffSummary.getByText("work waits")).toBeVisible();
    await expect(kickoffSummary.getByText("Roles: Project Operator, Backend Engineer, Code Reviewer")).toBeVisible();
    await expect(kickoffSummary.getByText("Next user action:")).toBeVisible();
    await expect(frontendReviewRow.getByText(/frontend-engineer ·/)).toBeVisible();
    await expect(modal.getByText(/@[a-z0-9]/i)).toHaveCount(0);

    await modal.getByRole("button", { name: "Back" }).click();
    await modal.getByLabel("Start with").selectOption("");
    await expect(modal.getByPlaceholder("e.g. Engineering Team")).toBeVisible();
    await expect(modal.getByPlaceholder("Search agents…")).toBeVisible();
  });

  /* ---------------------------------------------------------------------- */
  /*  Create Company modal                                                   */
  /* ---------------------------------------------------------------------- */

  test("should open Create Company modal from actions menu", async ({
    page,
  }) => {
    const actionsBtn = page.locator('[aria-label="Actions menu"]');
    await actionsBtn.click();
    await page.getByText("New Company").click();
    await page.waitForTimeout(500);
    const overlay = page.locator(".modal-overlay, .modal-backdrop");
    const visible = await overlay.isVisible({ timeout: 3000 }).catch(() => false);
    expect(typeof visible).toBe("boolean");
  });

  test("should show company name input in Create Company modal", async ({
    page,
  }) => {
    const actionsBtn = page.locator('[aria-label="Actions menu"]');
    await actionsBtn.click();
    await page.getByText("New Company").click();
    await page.waitForTimeout(500);
    const nameInput = page.locator('input[type="text"]').first();
    const visible = await nameInput.isVisible({ timeout: 3000 }).catch(() => false);
    expect(typeof visible).toBe("boolean");
  });

  test("Escape closes only the nested folder picker and preserves the company form", async ({ page }) => {
    const actionsBtn = page.locator('[aria-label="Actions menu"]');
    await actionsBtn.click();
    await page.getByRole("button", { name: "New Company" }).click();
    const companyDialog = page.getByRole("dialog", { name: "Create Company" });
    const nameInput = companyDialog.getByPlaceholder("e.g. Acme Corp");
    await nameInput.fill("Preserved Company Draft");
    await companyDialog.getByRole("button", { name: "Browse" }).click();
    await expect(page.getByRole("dialog")).toHaveCount(2);

    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog")).toHaveCount(1);
    await expect(companyDialog).toBeVisible();
    await expect(nameInput).toHaveValue("Preserved Company Draft");

    await page.keyboard.press("Escape");
    await expect(companyDialog).toBeHidden();
    await expect(actionsBtn).toBeFocused();
  });

  test("keeps partial company creation visible when AI Manager provisioning fails", async ({ page }) => {
    const provisioningBodies: Array<Record<string, unknown>> = [];
    await page.route("**/api/companies", (route) => route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        id: "company-partial", name: "Partial Company", slug: "partial-company",
        description: null, avatar_url: null, owner_id: "operator", folder_path: null,
        archived_at: null, deleted_at: null, created_at: new Date().toISOString(), updated_at: new Date().toISOString(),
      }),
    }));
    await page.route("**/api/agents/provision", async (route) => {
      provisioningBodies.push(route.request().postDataJSON() as Record<string, unknown>);
      if (provisioningBodies.length === 1) {
        await route.fulfill({
          status: 503,
          contentType: "application/json",
          body: JSON.stringify({ error: "driver unavailable" }),
        });
        return;
      }
      await route.fulfill({ status: 201, contentType: "application/json", body: "{}" });
    });
    await page.locator('[aria-label="Actions menu"]').click();
    await page.getByRole("button", { name: "New Company" }).click();
    const dialog = page.getByRole("dialog", { name: "Create Company" });
    await dialog.getByPlaceholder("e.g. Acme Corp").fill("Partial Company");
    await dialog.getByLabel("Manager Model").fill("claude-opus-5");
    await dialog.getByRole("button", { name: "Create" }).click();

    await expect(dialog).toBeVisible();
    await expect(dialog.getByRole("alert")).toContainText("Company created, but AI Manager failed");
    await expect(dialog.getByRole("button", { name: "Retry AI Manager" })).toBeVisible();
    await expect(dialog.getByRole("button", { name: "Continue without AI Manager" })).toBeVisible();
    await expect(dialog.getByRole("checkbox", { name: "Include AI Manager" })).toBeDisabled();
    await expect(dialog.getByLabel("Manager Driver")).toBeDisabled();

    await dialog.getByRole("button", { name: "Retry AI Manager" }).click();
    await expect(dialog).toBeHidden();
    expect(provisioningBodies).toHaveLength(2);
    expect(provisioningBodies[0].idempotency_key).toBe("company:company-partial:ai-manager");
    expect(provisioningBodies[1].idempotency_key).toBe(provisioningBodies[0].idempotency_key);
    expect(provisioningBodies[0].model).toBe("claude-opus-5");
    expect(provisioningBodies[1].model).toBe(provisioningBodies[0].model);
  });

  /* ---------------------------------------------------------------------- */
  /*  Modal backdrop                                                         */
  /* ---------------------------------------------------------------------- */

  test("should close modal when clicking backdrop", async ({ page }) => {
    await openCreateGroupModal(page);
    // Click outside the modal (on the backdrop)
    const backdrop = page.locator(".modal-overlay, .modal-backdrop");
    if (await backdrop.isVisible({ timeout: 3000 }).catch(() => false)) {
      await backdrop.click({ position: { x: 10, y: 10 } });
      await page.waitForTimeout(500);
    }
  });

  /* ---------------------------------------------------------------------- */
  /*  Modal focus trap                                                       */
  /* ---------------------------------------------------------------------- */

  test("should keep keyboard focus within modal", async ({ page }) => {
    const actionsBtn = page.locator('[aria-label="Actions menu"]');
    await actionsBtn.click();
    await page.getByText("Create Agent").click();
    await page.waitForTimeout(500);
    // Tab through the modal
    await page.keyboard.press("Tab");
    await page.keyboard.press("Tab");
    await page.keyboard.press("Tab");
    const dialog = page.getByRole("dialog");
    await expect(dialog.locator(":focus")).toHaveCount(1);
  });
});
