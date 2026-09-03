import { expect, test } from "@playwright/test";

import { createCompany, deleteCompany, uniqueName } from "../fixtures/api";
import { gotoDashboard, login } from "../fixtures/auth";

test("Company Machines is separate from Remote Access and supports inline rename", async ({ page }) => {
  const session = await login(page);
  const company = await createCompany(
    page,
    session.token,
    session.principal.id,
    uniqueName("machines-company"),
  );
  const west = {
    id: "host-west",
    company_id: company.id,
    name: "Build Server West",
    status: "online",
    last_seen_at: new Date().toISOString(),
    created_at: new Date().toISOString(),
  };
  const gpu = {
    id: "host-gpu",
    company_id: company.id,
    name: "GPU Server",
    status: "offline",
    last_seen_at: null,
    created_at: new Date().toISOString(),
  };
  let hosts = [west, gpu];

  await page.route(`**/api/v1/companies/${company.id}/runtime-hosts`, (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(hosts) }),
  );
  await page.route("**/api/v1/runtime-hosts/host-west", async (route) => {
    if (route.request().method() !== "PUT") return route.continue();
    const body = route.request().postDataJSON() as { name: string };
    hosts = [{ ...west, name: body.name }, gpu];
    return route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(hosts[0]) });
  });

  try {
    await gotoDashboard(page);
    await page.getByRole("button", { name: "Select company" }).click();
    const companyRow = page.locator(".company-dropdown-item").filter({ hasText: company.name });
    await companyRow.locator(".company-dropdown-item-name").click();

    await page.getByRole("button", { name: "Select company" }).click();
    await companyRow.getByRole("button", { name: `Actions for ${company.name}` }).click();
    await page.getByRole("button", { name: "Machines", exact: true }).click();

    await expect(page.getByRole("heading", { name: "Machines", exact: true })).toBeVisible();
    await expect(page.getByText("This computer", { exact: true })).toBeVisible();
    await expect(page.getByText("Build Server West", { exact: true })).toBeVisible();
    await expect(page.getByText("GPU Server", { exact: true })).toBeVisible();
    await expect(page.getByText("3", { exact: true }).first()).toBeVisible();

    await page.getByRole("button", { name: "Rename Build Server West" }).click();
    const rename = page.getByRole("textbox", { name: "Rename Build Server West" });
    await rename.fill("West Builder");
    await rename.press("Enter");
    await expect(page.getByText("West Builder", { exact: true })).toBeVisible();

    await page.getByRole("button", { name: "Close Machines" }).click();
    await page.getByRole("button", { name: "Actions menu" }).click();
    await page.getByRole("button", { name: "Remote Control" }).click();
    await expect(page.getByRole("heading", { name: "Remote Control" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Machines", exact: true })).toHaveCount(0);
    await expect(page.getByRole("heading", { name: "Runtime servers" })).toHaveCount(0);
  } finally {
    await deleteCompany(page, session.token, company.id);
  }
});

test("Add machine reveals an expiring connector code", async ({ page }) => {
  const session = await login(page);
  const company = await createCompany(
    page,
    session.token,
    session.principal.id,
    uniqueName("machine-pairing"),
  );

  await page.route(`**/api/v1/companies/${company.id}/runtime-hosts`, (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: "[]" }),
  );
  await page.route(`**/api/v1/companies/${company.id}/runtime-host-pairings`, (route) =>
    route.fulfill({
      status: 201,
      contentType: "application/json",
      body: JSON.stringify({ code: "12345678", expires_at: new Date(Date.now() + 600_000).toISOString() }),
    }),
  );

  try {
    await gotoDashboard(page);
    await page.getByRole("button", { name: "Select company" }).click();
    const companyRow = page.locator(".company-dropdown-item").filter({ hasText: company.name });
    await companyRow.locator(".company-dropdown-item-name").click();
    await page.getByRole("button", { name: "Select company" }).click();
    await companyRow.getByRole("button", { name: `Actions for ${company.name}` }).click();
    await page.getByRole("button", { name: "Machines", exact: true }).click();
    await page.getByRole("button", { name: "Add machine" }).click();

    await expect(page.getByRole("heading", { name: "Connect another computer" })).toBeVisible();
    await expect(page.getByText("1234 5678", { exact: true })).toBeVisible();
    await expect(page.getByText(/Single use · expires/)).toBeVisible();
  } finally {
    await deleteCompany(page, session.token, company.id);
  }
});

test("Remote Control accepts another computer's pairing credential in Choruz", async ({ page }) => {
  await login(page);
  await page.route("**/api/v1/remote-control/settings", (route) => route.fulfill({
    status: 200,
    contentType: "application/json",
    body: JSON.stringify({ gateway_url: "https://gateway.example", gateway_ticket: null }),
  }));
  await page.route("**/api/v1/remote-control/devices", (route) => route.fulfill({
    status: 200,
    contentType: "application/json",
    body: "[]",
  }));
  await page.route("https://gateway.example/remote**", (route) => route.fulfill({
    status: 200,
    contentType: "text/html",
    body: "<title>Remote Choruz</title>",
  }));

  await gotoDashboard(page);
  await page.getByRole("button", { name: "Actions menu" }).click();
  await page.getByRole("button", { name: "Remote Control" }).click();
  const credential = "v1.AAAAAAAAAAAAAAAAAAAAAA.BBBBBBBBBBBBBBBBBBBBBB";
  await page.getByRole("textbox", { name: "Other computer pairing credential" }).fill(credential);
  await page.getByRole("button", { name: "Connect", exact: true }).click();

  await expect(page).toHaveURL(
    new RegExp(`/remote\\?gateway=https%3A%2F%2Fgateway\\.example&device_name=.+#credential=${credential}$`),
  );
});
