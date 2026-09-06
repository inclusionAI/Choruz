import { expect, test } from "@playwright/test";
import { API_BASE, WEB_BASE, signup } from "../fixtures/auth";
import { createCompany, createGroup, deleteCompany, uniqueName } from "../fixtures/api";

test("sending a message should show exactly one bubble, not duplicated", async ({ page }, testInfo) => {
  const { token, principal } = await signup(page, uniqueName("dedup-user"), "dedup-test-password");
  const company = await createCompany(page, token, principal.id, uniqueName("dedup-company"));
  try {
    const group = await createGroup(page, token, principal.id, uniqueName("dedup-group"), [], company.id);
    await page.goto(`${WEB_BASE}/dashboard?conversationId=${group.id}`);
    await expect(page.getByRole("heading", { name: group.name, level: 1 })).toBeVisible();
    const content = uniqueName("dedup-test");
    await page.locator(".chat-input-bar textarea").fill(content);
    await page.locator(".send-btn").click();

    let messageId = "";
    await expect.poll(async () => {
      const response = await page.request.get(`${API_BASE}/v1/conversations/${group.id}/messages?principal_id=${principal.id}`, {
        headers: { Authorization: `Bearer ${token}` },
      });
      expect(response.ok()).toBeTruthy();
      const messages = (await response.json() as Array<{ id: string; content: string }>)
        .filter(message => message.content === content);
      messageId = messages[0]?.id ?? "";
      return messages.length;
    }).toBe(1);
    // Wait for canonical identity, not just the optimistic bubble.
    await expect(page.locator(`[data-msg-id="${messageId}"]`)).toBeVisible();
    await expect(page.locator(".msg-group").filter({ hasText: content })).toHaveCount(1);
    await page.screenshot({ path: testInfo.outputPath("message-dedup.png") });
  } finally {
    await deleteCompany(page, token, company.id);
  }
});
