import { expect, test } from "@playwright/test";

// The Remote page needs no session: it pairs with a host through the Cloud
// Gateway named in the URL. These checks run without a gateway, so they cover
// the entry form and the failure it reports when the gateway is unreachable.

const unreachableGateway = "Pairing credential is invalid, expired, or not ready.";
const credential = "v1.AAAAAAAAAAAAAAAAAAAAAA.BBBBBBBBBBBBBBBBBBBBBB";

test("the Remote page consumes the fragment credential and reports an unreachable gateway", async ({ page }) => {
  await page.goto(`/remote?gateway=http://127.0.0.1:9&device_name=Playwright#credential=${credential}`);

  await expect(page.getByLabel("Cloud Gateway URL")).toHaveValue("http://127.0.0.1:9");
  await expect(page.getByLabel("Pairing credential")).toHaveValue(credential);
  await expect(page.getByLabel("Device name")).toHaveValue("Playwright");
  await expect.poll(() => new URL(page.url()).hash).toBe("");
  expect(new URL(page.url()).searchParams.get("gateway")).toBe("http://127.0.0.1:9");
  expect(new URL(page.url()).searchParams.get("device_name")).toBe("Playwright");
  // A complete credential starts pairing at once; the closed port ends it.
  await expect(page.getByText(unreachableGateway)).toBeVisible();
  await expect(page.getByRole("button", { name: "Connect" })).toBeEnabled();
});

test("Connect waits for a complete credential and a gateway", async ({ page }) => {
  await page.goto("/remote");

  const connect = page.getByRole("button", { name: "Connect" });
  const credentialInput = page.getByLabel("Pairing credential");
  await expect(connect).toBeDisabled();
  // The formatted value proves the page is hydrated before the rest is typed.
  await credentialInput.fill(credential);
  await expect(connect).toBeDisabled();
  await page.getByLabel("Cloud Gateway URL").fill("http://127.0.0.1:9");
  await expect(connect).toBeEnabled();
  await credentialInput.fill("v1.short");
  await expect(connect).toBeDisabled();
  await credentialInput.fill(credential);
  await expect(connect).toBeEnabled();

  await connect.click();
  await expect(page.getByText(unreachableGateway)).toBeVisible();
});
