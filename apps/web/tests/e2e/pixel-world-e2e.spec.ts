import { test, expect } from "@playwright/test";
import { API_BASE, CREDENTIALS, WEB_BASE } from "../fixtures/auth";
import { createGroup, provisionAgent, uniqueName } from "../fixtures/api";

// Login helper
async function loginAndOpenPixelWorld(
  page: import("@playwright/test").Page,
  { seedAgent = false }: { seedAgent?: boolean } = {},
) {
  const r = await page.request.post(`${API_BASE}/v1/auth/local/login`, {
    data: { username: CREDENTIALS.username, password: CREDENTIALS.password },
  });
  const d = await r.json();
  await page.context().addCookies([{
    name: "choruz_session", value: d.session_token,
    url: WEB_BASE, httpOnly: true, sameSite: "Lax",
    expires: Math.floor(Date.now()/1000)+3600,
  }]);
  if (seedAgent) {
    const agent = await provisionAgent(page, d.session_token, uniqueName("pixel-agent"));
    await createGroup(
      page,
      d.session_token,
      d.principal.id,
      uniqueName("pixel-room"),
      [agent.agentId],
    );
  }
  await page.goto(`${WEB_BASE}/dashboard`);
  await page.waitForLoadState("networkidle");
  await page.waitForTimeout(2000);

  // Sidebar refactor moved "Pixel World" behind the "+" Actions menu.
  // Open the menu first, then click the menu item.
  const actionsBtn = page.getByRole('button', { name: 'Actions menu' });
  if (await actionsBtn.isVisible()) {
    await actionsBtn.click();
    const pixelBtn = page.getByRole('button', { name: 'Pixel World' });
    if (await pixelBtn.isVisible()) {
      await pixelBtn.click();
      await page.waitForTimeout(5000);
    }
  }
}

test.describe("Pixel World", () => {

  test("loads only retained runtime assets", async ({ page }) => {
    const assetResponses: Array<{ path: string; status: number }> = [];
    page.on("response", (response) => {
      const url = new URL(response.url());
      if (url.pathname.startsWith("/sprites/") || url.pathname.startsWith("/game/floors/")) {
        assetResponses.push({ path: url.pathname, status: response.status() });
      }
    });

    await loginAndOpenPixelWorld(page);

    expect(assetResponses.length).toBeGreaterThan(0);
    expect(assetResponses.filter(({ status }) => status >= 400)).toEqual([]);
    expect(assetResponses.some(({ path }) => path.endsWith("/floor1_bg.png"))).toBe(true);
    expect(assetResponses.some(({ path }) => path.endsWith("/agent_atlas.png"))).toBe(true);

    const removedCollections = ["/characters/", "/creatures/", "/furniture/", "/ui/"];
    expect(
      assetResponses.filter(({ path }) => removedCollections.some((segment) => path.includes(segment))),
    ).toEqual([]);
  });

  test("renders non-black background", async ({ page }) => {
    await loginAndOpenPixelWorld(page);

    // Get canvas pixel data to verify it's not all black
    const hasColor = await page.evaluate(() => {
      const canvas = document.querySelector('.pixel-world-panel canvas') as HTMLCanvasElement;
      if (!canvas) return false;
      const ctx = canvas.getContext('2d');
      if (!ctx) return false;
      const data = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
      let nonBlack = 0;
      for (let i = 0; i < data.length; i += 4) {
        if (data[i] > 30 || data[i+1] > 30 || data[i+2] > 30) nonBlack++;
      }
      return nonBlack > (data.length / 4) * 0.1; // At least 10% non-black
    });
    expect(hasColor).toBe(true);
  });

  test("player sprite exists and is visible", async ({ page }) => {
    await loginAndOpenPixelWorld(page);

    const playerInfo = await page.evaluate(() => {
      const player = (window as any).__PHASER_PLAYER;
      if (!player) return null;
      return {
        x: player.x,
        y: player.y,
        visible: player.visible,
        alpha: player.alpha,
        width: player.width,
        height: player.height,
      };
    });
    expect(playerInfo).not.toBeNull();
    expect(playerInfo!.visible).toBe(true);
    expect(playerInfo!.alpha).toBeGreaterThan(0);
    expect(playerInfo!.width).toBeGreaterThan(0);
  });

  test("WASD movement works", async ({ page }) => {
    await loginAndOpenPixelWorld(page);

    // Click canvas to focus it
    const canvas = page.locator('.pixel-world-panel canvas').first();
    await canvas.click();
    await page.waitForTimeout(500);

    const before = await page.evaluate(() => {
      const p = (window as any).__PHASER_PLAYER;
      return p ? { x: p.x, y: p.y } : null;
    });

    expect(before).not.toBeNull();

    // Hold W until the game loop has moved the player up (y decreases). A
    // fixed wait is not enough on a loaded CI runner, where Phaser may not
    // get a frame for a while.
    await page.keyboard.down('w');
    try {
      await expect
        .poll(
          () =>
            page.evaluate(() => {
              const p = (window as any).__PHASER_PLAYER;
              return p ? p.y : null;
            }),
          { timeout: 10_000 },
        )
        .toBeLessThan(before!.y);
    } finally {
      await page.keyboard.up('w');
    }
  });

  test("NPCs are spawned", async ({ page }) => {
    await loginAndOpenPixelWorld(page, { seedAgent: true });

    const npcCount = await page.evaluate(() => {
      const scene = (window as any).__PHASER_SCENE;
      return scene?.agentSprites?.size ?? 0;
    });
    expect(npcCount).toBeGreaterThan(0);
  });

  test("camera follows player after drag", async ({ page }) => {
    await loginAndOpenPixelWorld(page);

    const canvas = page.locator('.pixel-world-panel canvas').first();
    await canvas.click();
    await page.waitForTimeout(500);

    // Drag the canvas
    const box = await canvas.boundingBox();
    if (box) {
      await page.mouse.move(box.x + box.width/2, box.y + box.height/2);
      await page.mouse.down();
      await page.mouse.move(box.x + box.width/2 + 100, box.y + box.height/2 + 100, { steps: 10 });
      await page.mouse.up();
      await page.waitForTimeout(500);
    }

    // Move player
    for (let i = 0; i < 20; i++) {
      await page.keyboard.down('d');
      await page.waitForTimeout(50);
    }
    await page.keyboard.up('d');
    await page.waitForTimeout(1000);

    // Camera should be near player
    const result = await page.evaluate(() => {
      const scene = (window as any).__PHASER_SCENE;
      const player = (window as any).__PHASER_PLAYER;
      if (!scene || !player) return null;
      const cam = scene.cameras.main;
      return {
        playerX: player.x,
        cameraScrollX: cam.scrollX,
        cameraWidth: cam.width,
      };
    });

    expect(result).not.toBeNull();
    // Player should be within camera viewport
    const camLeft = result!.cameraScrollX;
    const camRight = result!.cameraScrollX + result!.cameraWidth;
    expect(result!.playerX).toBeGreaterThanOrEqual(camLeft);
    expect(result!.playerX).toBeLessThanOrEqual(camRight);
  });

  test("no console errors during gameplay", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));

    await loginAndOpenPixelWorld(page);

    const canvas = page.locator('.pixel-world-panel canvas').first();
    await canvas.click();

    // Walk around
    for (const key of ['w', 'a', 's', 'd']) {
      for (let i = 0; i < 5; i++) {
        await page.keyboard.down(key);
        await page.waitForTimeout(50);
      }
      await page.keyboard.up(key);
      await page.waitForTimeout(200);
    }

    // Filter out non-game errors
    const gameErrors = errors.filter(e =>
      e.includes('Phaser') || e.includes('MainScene') || e.includes('pixel') ||
      e.includes('Cannot read') || e.includes('undefined') || e.includes('null')
    );
    expect(gameErrors).toHaveLength(0);
  });
});
