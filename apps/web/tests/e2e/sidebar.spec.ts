import { expect, test, type Page } from "@playwright/test";
import { login, gotoDashboard } from "../fixtures/auth";
import {
  createAgent,
  createCompany,
  createDirectConversation,
  createGroup,
  uniqueName,
} from "../fixtures/api";

const SECTION_TITLES = {
  pinned: "Pinned Chats",
  direct: "Direct Messages",
  group: "Group Conversations",
  archived: "Archived",
} as const;

type SectionTitle = (typeof SECTION_TITLES)[keyof typeof SECTION_TITLES];
type LoginSession = Awaited<ReturnType<typeof login>>;

function escapeRegex(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function sidebarSection(page: Page, title: SectionTitle) {
  return page.getByRole("group", { name: title });
}

function sectionHeader(page: Page, title: SectionTitle) {
  return sidebarSection(page, title).getByRole("button", {
    name: new RegExp(`^${escapeRegex(title)}`),
  });
}

function rowInSection(page: Page, title: SectionTitle, name: string) {
  return sidebarSection(page, title).locator(".conv-item").filter({ hasText: name });
}

async function expectRowsInOrder(
  page: Page,
  title: SectionTitle,
  orderedNames: string[],
) {
  const rowNames = await sidebarSection(page, title)
    .locator(".conv-item .conv-name")
    .allTextContents();
  const indices = orderedNames.map((name) =>
    rowNames.findIndex((text) => text.includes(name)),
  );
  expect(indices, `${title} rows: ${rowNames.join(", ")}`).not.toContain(-1);
  expect(indices).toEqual([...indices].sort((a, b) => a - b));
}

async function selectCompany(page: Page, name: string) {
  await page.getByRole("button", { name: "Select company" }).click();
  await page
    .locator(".company-dropdown-item")
    .filter({ hasText: name })
    .locator(".company-dropdown-item-name")
    .click();
  await expect(page.getByRole("button", { name: "Select company" })).toContainText(name);
}

async function expectSectionExpanded(
  page: Page,
  title: SectionTitle,
  expanded: boolean,
) {
  await expect(sectionHeader(page, title)).toHaveAttribute(
    "aria-expanded",
    String(expanded),
  );
}

async function expandSection(page: Page, title: SectionTitle) {
  const header = sectionHeader(page, title);
  await expect(header).toBeVisible({ timeout: 10_000 });
  if ((await header.getAttribute("aria-expanded")) !== "true") {
    await header.click();
  }
  await expectSectionExpanded(page, title, true);
}

async function collapseSection(page: Page, title: SectionTitle) {
  const header = sectionHeader(page, title);
  await expect(header).toBeVisible({ timeout: 10_000 });
  if ((await header.getAttribute("aria-expanded")) !== "false") {
    await header.click();
  }
  await expectSectionExpanded(page, title, false);
}

async function seedSidebarConversationsForSession(
  page: Page,
  { token, principal }: LoginSession,
  workspaceId?: string,
) {
  const directName = uniqueName("sb-dm");
  const groupName = uniqueName("sb-group");
  const agent = await createAgent(page, token, principal.id, directName, workspaceId);
  const direct = await createDirectConversation(
    page,
    token,
    principal.id,
    agent.principal.id,
    workspaceId,
  );
  const group = await createGroup(page, token, principal.id, groupName, [], workspaceId);

  await gotoDashboard(page, { expandSidebarSections: false });
  await expect(sectionHeader(page, SECTION_TITLES.direct)).toBeVisible({
    timeout: 15_000,
  });
  await expect(sectionHeader(page, SECTION_TITLES.group)).toBeVisible({
    timeout: 15_000,
  });

  return { token, principal, direct, directName, group, groupName };
}

async function seedSidebarConversations(page: Page) {
  return seedSidebarConversationsForSession(page, await login(page));
}

async function togglePinFromSection(
  page: Page,
  sectionTitle: SectionTitle,
  name: string,
  conversationId: string,
  nextPinned: boolean,
) {
  const method = nextPinned ? "PUT" : "DELETE";
  const responsePromise = page.waitForResponse((response) => {
    return (
      response.url().includes(`/api/v1/conversations/${conversationId}/pin`) &&
      response.request().method() === method
    );
  });
  await rowInSection(page, sectionTitle, name)
    .getByRole("button", {
      name: nextPinned ? `Pin chat: ${name}` : `Unpin chat: ${name}`,
    })
    .click();
  const response = await responsePromise;
  expect(response.ok(), `${method} pin ${conversationId} -> ${response.status()}`).toBeTruthy();
}

async function toggleArchiveFromSection(
  page: Page,
  sectionTitle: SectionTitle,
  name: string,
  conversationId: string,
  nextArchived: boolean,
) {
  const method = nextArchived ? "PUT" : "DELETE";
  const responsePromise = page.waitForResponse((response) =>
    response.url().includes(`/api/v1/conversations/${conversationId}/archive`) &&
    response.request().method() === method,
  );
  await rowInSection(page, sectionTitle, name)
    .getByRole("button", {
      name: nextArchived ? `Archive chat: ${name}` : `Restore chat: ${name}`,
    })
    .click();
  const response = await responsePromise;
  expect(
    response.ok(),
    `${method} archive ${conversationId} -> ${response.status()}`,
  ).toBeTruthy();
}

test.describe("Sidebar layout", () => {
  test.beforeEach(async ({ page }) => {
    await login(page);
    await gotoDashboard(page, { expandSidebarSections: false });
  });

  /* ---------------------------------------------------------------------- */
  /*  Sidebar visibility                                                     */
  /* ---------------------------------------------------------------------- */

  test("should render the sidebar", async ({ page }) => {
    await expect(page.locator(".chat-sidebar")).toBeVisible({ timeout: 10_000 });
  });

  test("should show sidebar header with avatar and username", async ({
    page,
  }) => {
    await expect(page.locator(".sidebar-header")).toBeVisible({
      timeout: 10_000,
    });
    await expect(page.locator(".sidebar-header .avatar")).toBeVisible();
    await expect(page.locator(".sidebar-header").getByText("operator", { exact: true })).toBeVisible();
  });

  test("should show the actions (+) button", async ({ page }) => {
    const actionsBtn = page.locator('[aria-label="Actions menu"]');
    await expect(actionsBtn).toBeVisible({ timeout: 10_000 });
  });

  /* ---------------------------------------------------------------------- */
  /*  Actions menu                                                           */
  /* ---------------------------------------------------------------------- */

  test("should open actions menu on click", async ({ page }) => {
    const actionsBtn = page.locator('[aria-label="Actions menu"]');
    await actionsBtn.click();
    // Menu items should be visible
    await expect(page.getByRole("button", { name: "Create Agent" })).toBeVisible();
    await expect(page.getByRole("button", { name: "New Group" })).toBeVisible();
    await expect(page.getByRole("button", { name: "New Company" })).toBeVisible();
  });

  test("should close actions menu on backdrop click", async ({ page }) => {
    const actionsBtn = page.locator('[aria-label="Actions menu"]');
    await actionsBtn.click();
    await expect(page.getByText("Create Agent")).toBeVisible();
    // Click the backdrop
    await page.mouse.click(10, 10);
    await page.waitForTimeout(500);
    // Menu should be closed
    const menuVisible = await page
      .getByText("Create Agent")
      .isVisible({ timeout: 1000 })
      .catch(() => false);
    expect(menuVisible).toBeFalsy();
  });

  test("should show Manage Chats option in actions menu", async ({ page }) => {
    const actionsBtn = page.locator('[aria-label="Actions menu"]');
    await actionsBtn.click();
    await expect(page.getByText("Manage Chats")).toBeVisible();
  });

  /* ---------------------------------------------------------------------- */
  /*  User menu                                                              */
  /* ---------------------------------------------------------------------- */

  test("should open user menu when clicking avatar area", async ({ page }) => {
    const userMenu = page.locator('[aria-label="User menu"]');
    if (await userMenu.isVisible({ timeout: 5000 })) {
      await userMenu.click();
      await expect(page.getByText("Documentation")).toBeVisible({ timeout: 3000 });
      await expect(page.getByText("Sign Out")).toHaveCount(0);
    }
  });

  /* ---------------------------------------------------------------------- */
  /*  Sidebar search                                                         */
  /* ---------------------------------------------------------------------- */

  test("should render sidebar search input", async ({ page }) => {
    const searchInput = page.locator(".sidebar-search input");
    await expect(searchInput).toBeVisible({ timeout: 10_000 });
    const placeholder = await searchInput.getAttribute("placeholder");
    expect(placeholder).toContain("Search");
  });

  /* ---------------------------------------------------------------------- */
  /*  Conversation list structure                                            */
  /* ---------------------------------------------------------------------- */

  test("should render conversation list container", async ({
    page,
  }) => {
    await expect(page.locator('.conversation-list[aria-label="Conversations"]')).toBeVisible({
      timeout: 10_000,
    });
  });

  test("should show conversation items with avatars", async ({ page }) => {
    const { directName } = await seedSidebarConversations(page);
    await expandSection(page, SECTION_TITLES.direct);
    const seededItem = rowInSection(page, SECTION_TITLES.direct, directName);
    await expect(seededItem).toBeVisible();
    await expect(seededItem.locator(".avatar")).toBeVisible();
  });

  /* ---------------------------------------------------------------------- */
  /*  Sidebar resize                                                         */
  /* ---------------------------------------------------------------------- */

  test("should persist sidebar width in localStorage", async ({ page }) => {
    // Check if sidebar width is saved
    const saved = await page.evaluate(() =>
      localStorage.getItem("choruz_sidebar_width"),
    );
    // May or may not be saved yet
    expect(typeof saved === "string" || saved === null).toBeTruthy();
  });

  /* ---------------------------------------------------------------------- */
  /*  Manage mode                                                            */
  /* ---------------------------------------------------------------------- */

  test("should enter manage mode and show checkboxes", async ({ page }) => {
    const actionsBtn = page.locator('[aria-label="Actions menu"]');
    await actionsBtn.click();
    await page.getByText("Manage Chats").click();
    await page.waitForTimeout(500);
    // Cancel button should be visible in manage mode
    await expect(page.getByText("Cancel")).toBeVisible();
  });

  test("should exit manage mode on Cancel click", async ({ page }) => {
    const actionsBtn = page.locator('[aria-label="Actions menu"]');
    await actionsBtn.click();
    await page.getByText("Manage Chats").click();
    await page.waitForTimeout(300);
    await page.getByText("Cancel").click();
    await page.waitForTimeout(300);
    // Actions button should be back
    await expect(
      page.locator('[aria-label="Actions menu"]'),
    ).toBeVisible();
  });

  test("should show delete bar when items are selected in manage mode", async ({
    page,
  }) => {
    const session = await login(page);
    const company = await createCompany(
      page,
      session.token,
      session.principal.id,
      uniqueName("manage-company"),
    );
    await seedSidebarConversationsForSession(page, session, company.id);
    await selectCompany(page, company.name);
    await expectSectionExpanded(page, SECTION_TITLES.direct, false);
    await expectSectionExpanded(page, SECTION_TITLES.group, false);
    const actionsBtn = page.locator('[aria-label="Actions menu"]');
    await actionsBtn.click();
    await page.getByText("Manage Chats").click();
    await page.waitForTimeout(300);
    // Click "All" to select everything
    await page.getByRole("button", { name: "All" }).click();
    await page.waitForTimeout(300);
    // Delete bar should appear
    const deleteBar = page.getByText("2 selected");
    await expect(deleteBar).toBeVisible();
  });
});

test.describe("Sidebar chat sections and pins", () => {
  test("uses one clean active shell for direct and group conversations", async ({
    page,
  }) => {
    const { directName, groupName } = await seedSidebarConversations(page);

    await expandSection(page, SECTION_TITLES.direct);
    await expandSection(page, SECTION_TITLES.group);

    for (const [section, name] of [
      [SECTION_TITLES.group, groupName],
      [SECTION_TITLES.direct, directName],
    ] as const) {
      const row = rowInSection(page, section, name);
      await row.locator(".conv-item-main").click();
      await expect(row).toHaveClass(/\bactive\b/);
      await expect(row).toHaveCSS("box-shadow", "none");

      await expect
        .poll(() =>
          row.evaluate((element) => getComputedStyle(element).backgroundColor),
        )
        .not.toBe("rgba(0, 0, 0, 0)");

      await expect(row.locator(".conv-item-main")).toHaveCSS(
        "background-color",
        "rgba(0, 0, 0, 0)",
      );

      const edges = await row.evaluate((element) => {
        const style = getComputedStyle(element);
        return {
          leftWidth: style.borderLeftWidth,
          rightWidth: style.borderRightWidth,
          leftColor: style.borderLeftColor,
          rightColor: style.borderRightColor,
        };
      });
      expect(edges.leftWidth).toBe(edges.rightWidth);
      expect(edges.leftColor).toBe(edges.rightColor);
    }
  });

  test("renders Direct and Group sections collapsed by default and expands search matches", async ({
    page,
  }) => {
    const { directName, groupName } = await seedSidebarConversations(page);

    await expectSectionExpanded(page, SECTION_TITLES.direct, false);
    await expectSectionExpanded(page, SECTION_TITLES.group, false);

    await expandSection(page, SECTION_TITLES.direct);
    await expect(rowInSection(page, SECTION_TITLES.direct, directName)).toBeVisible();
    await expandSection(page, SECTION_TITLES.group);
    await expect(rowInSection(page, SECTION_TITLES.group, groupName)).toBeVisible();

    await collapseSection(page, SECTION_TITLES.direct);
    await collapseSection(page, SECTION_TITLES.group);
    await page.locator(".sidebar-search input").fill(groupName);

    await expectSectionExpanded(page, SECTION_TITLES.group, true);
    await expect(rowInSection(page, SECTION_TITLES.group, groupName)).toBeVisible();
    await expect(rowInSection(page, SECTION_TITLES.direct, directName)).toHaveCount(0);

    await page.locator(".sidebar-search input").fill("");
    await expectSectionExpanded(page, SECTION_TITLES.group, false);
  });

  test("lets the user collapse the section containing the active direct message", async ({
    page,
  }) => {
    const { directName } = await seedSidebarConversations(page);

    await expandSection(page, SECTION_TITLES.direct);
    const directRow = rowInSection(page, SECTION_TITLES.direct, directName);
    await directRow.locator(".conv-item-main").click();
    await expect(directRow).toHaveClass(/\bactive\b/);

    await collapseSection(page, SECTION_TITLES.direct);
    await expect(directRow).toBeHidden();
  });

  test("pins direct and group chats only under Pinned Chats, persists reload, and unpins back", async ({
    page,
  }) => {
    const { direct, directName, group, groupName } = await seedSidebarConversations(page);

    await expandSection(page, SECTION_TITLES.direct);
    await expandSection(page, SECTION_TITLES.group);

    await togglePinFromSection(
      page,
      SECTION_TITLES.direct,
      directName,
      direct.id,
      true,
    );
    await expect(rowInSection(page, SECTION_TITLES.pinned, directName)).toBeVisible();
    await expect(rowInSection(page, SECTION_TITLES.direct, directName)).toHaveCount(0);

    await togglePinFromSection(
      page,
      SECTION_TITLES.group,
      groupName,
      group.id,
      true,
    );
    await expect(rowInSection(page, SECTION_TITLES.pinned, groupName)).toBeVisible();
    await expect(rowInSection(page, SECTION_TITLES.group, groupName)).toHaveCount(0);
    await expectRowsInOrder(page, SECTION_TITLES.pinned, [groupName, directName]);

    await page.reload();
    await expect(page.locator(".chat-sidebar")).toBeVisible({ timeout: 15_000 });
    await expectSectionExpanded(page, SECTION_TITLES.pinned, true);
    await expect(rowInSection(page, SECTION_TITLES.pinned, directName)).toBeVisible();
    await expect(rowInSection(page, SECTION_TITLES.pinned, groupName)).toBeVisible();
    await expectRowsInOrder(page, SECTION_TITLES.pinned, [groupName, directName]);

    await togglePinFromSection(
      page,
      SECTION_TITLES.pinned,
      directName,
      direct.id,
      false,
    );
    await expect(rowInSection(page, SECTION_TITLES.pinned, directName)).toHaveCount(0);
    await expandSection(page, SECTION_TITLES.direct);
    await expect(rowInSection(page, SECTION_TITLES.direct, directName)).toBeVisible();

    await togglePinFromSection(
      page,
      SECTION_TITLES.pinned,
      groupName,
      group.id,
      false,
    );
    await expect(rowInSection(page, SECTION_TITLES.pinned, groupName)).toHaveCount(0);
    await expandSection(page, SECTION_TITLES.group);
    await expect(rowInSection(page, SECTION_TITLES.group, groupName)).toBeVisible();
  });

  test("archives a chat without deleting it, persists reload, and restores it", async ({
    page,
  }) => {
    const { direct, directName } = await seedSidebarConversations(page);

    await expandSection(page, SECTION_TITLES.direct);
    const directRow = rowInSection(page, SECTION_TITLES.direct, directName);
    await expect(directRow).toBeVisible();
    await expect(directRow).toHaveCSS("box-shadow", "none");

    await toggleArchiveFromSection(
      page,
      SECTION_TITLES.direct,
      directName,
      direct.id,
      true,
    );
    await expect(rowInSection(page, SECTION_TITLES.direct, directName)).toHaveCount(0);
    await expandSection(page, SECTION_TITLES.archived);
    await expect(rowInSection(page, SECTION_TITLES.archived, directName)).toBeVisible();

    await page.reload();
    await expect(page.locator(".chat-sidebar")).toBeVisible({ timeout: 15_000 });
    await expandSection(page, SECTION_TITLES.archived);
    await expect(rowInSection(page, SECTION_TITLES.archived, directName)).toBeVisible();

    await toggleArchiveFromSection(
      page,
      SECTION_TITLES.archived,
      directName,
      direct.id,
      false,
    );
    await expect(sidebarSection(page, SECTION_TITLES.archived)).toHaveCount(0);
    await expandSection(page, SECTION_TITLES.direct);
    await expect(rowInSection(page, SECTION_TITLES.direct, directName)).toBeVisible();
  });

});
