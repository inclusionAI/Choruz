import { expect, test, type Page } from "@playwright/test";
import { login, gotoDashboard } from "../fixtures/auth";
import { createGroup, provisionAgent, uniqueName } from "../fixtures/api";

/* ========================================================================== */
/*  Helpers                                                                    */
/* ========================================================================== */

/**
 * Login, navigate to dashboard, open Pixel World via the "+" menu, and wait
 * until the zustand store has agents + houses populated. Returns an agent id
 * + two house ids:
 *   - `otherHouseId`: any house the agent is not currently in (for walk tests).
 *   - `deskHouseId`: a house whose interactionPoints include a desk /
 *     reception_desk (for typing/talking/resumeState tests). `null` when the
 *     generated floor plan happens to have none.
 */
async function openPixelAndWaitForStore(page: Page): Promise<{
  agentId: string;
  otherHouseId: string;
  deskHouseId: string | null;
} | null> {
  const { token, principal } = await login(page);
  const agent = await provisionAgent(page, token, uniqueName("pixel-agent"));
  await createGroup(page, token, principal.id, uniqueName("pixel-room"), [agent.agentId]);
  await gotoDashboard(page);
  await page.waitForSelector(".chat-sidebar, .chat-app", { timeout: 15_000 });

  const plus = page.locator('[aria-label="Actions menu"]');
  if (!(await plus.isVisible({ timeout: 5_000 }).catch(() => false))) {
    return null;
  }
  await plus.click();
  await page.waitForTimeout(300);

  const pixelBtn = page.getByText("Pixel World");
  if (!(await pixelBtn.isVisible({ timeout: 3_000 }).catch(() => false))) {
    return null;
  }
  await pixelBtn.click();
  await page.waitForTimeout(800);

  // Wait for the store to be mounted AND initialize() to finish populating agents + houses.
  await page.waitForFunction(
    () => {
      const store = (window as any).__pixelWorldStore;
      if (!store) return false;
      const s = store.getState();
      return s.agents.size > 0 && s.houses.size > 0;
    },
    undefined,
    { timeout: 15_000 },
  );

  // Grab an agent id and two house ids: one the agent isn't currently in
  // (for walk tests), and preferably one that has a desk-type interaction
  // point (for typing/talking/resumeState tests).
  const ids = await page.evaluate(() => {
    const store = (window as any).__pixelWorldStore;
    const s = store.getState();
    const houseEntries = Array.from(s.houses.entries()) as Array<[string, any]>;
    const deskHouse = houseEntries.find(([, h]) => {
      const pts = h.room?.interactionPoints ?? [];
      return pts.some(
        (p: any) => p.type === "desk" || p.type === "reception_desk",
      );
    });
    for (const [agentId, a] of s.agents.entries() as Iterable<[string, any]>) {
      const other = houseEntries.find(([id]) => id !== a.currentHouseId);
      if (other) {
        return {
          agentId,
          otherHouseId: other[0] as string,
          deskHouseId: (deskHouse?.[0] ?? null) as string | null,
        };
      }
    }
    return null;
  });

  return ids;
}

/** Drive the store's `tick` exactly `n` times at 1:1 speed (no throttling). */
async function advanceTicks(page: Page, n: number): Promise<void> {
  await page.evaluate((count) => {
    const store = (window as any).__pixelWorldStore;
    const tick = store.getState().tick;
    for (let i = 0; i < count; i++) tick(800, 600);
  }, n);
}

/** Read a snapshot of the named agent. */
async function readAgent(page: Page, agentId: string) {
  return page.evaluate((id) => {
    const store = (window as any).__pixelWorldStore;
    const s = store.getState();
    const agent = s.agents.get(id);
    if (!agent) return null;
    return {
      state: agent.anim.state as string,
      arrivalState: agent.anim.arrivalState as string | undefined,
      resumeState: agent.anim.resumeState as string | undefined,
      bubble: agent.anim.bubble as string | null | undefined,
      bubbleTicks: (agent.anim.bubbleTicks ?? 0) as number,
      ticksInState: agent.anim.ticksInState as number,
      currentHouseId: agent.currentHouseId as string | null,
      x: agent.anim.x as number,
      y: agent.anim.y as number,
      targetX: agent.anim.targetX as number,
      targetY: agent.anim.targetY as number,
    };
  }, agentId);
}

/** Teleport the agent and player onto the same room interaction point, then
 * send the message before the game loop can move either actor. */
async function sendMessageAtRoomPoint(
  page: Page,
  agentId: string,
  houseId: string,
): Promise<string | null> {
  return page.evaluate(
    ({ agentId, houseId }) => {
      const store = (window as any).__pixelWorldStore;
      const s = store.getState();
      const agent = s.agents.get(agentId);
      const house = s.houses.get(houseId);
      if (!agent || !house || !s.player) return null;
      const pts = house.room.interactionPoints ?? [];
      if (pts.length === 0) return null;
      const preferred =
        pts.find((p: any) => p.type === "desk") ||
        pts.find((p: any) => p.type === "reception_desk") ||
        pts.find((p: any) => p.type === "sofa") ||
        pts[0];
      const TILE = 16;
      const next = new Map(s.agents);
      next.set(agentId, {
        ...agent,
        anim: {
          ...agent.anim,
          state: "idle",
          x: preferred.tileX * TILE,
          y: preferred.tileY * TILE,
          targetX: preferred.tileX * TILE,
          targetY: preferred.tileY * TILE,
          arrivalState: undefined,
          resumeState: undefined,
          bubble: null,
          bubbleTicks: 0,
          ticksInState: 0,
          frame: 0,
        },
        currentHouseId: houseId,
      });
      store.setState({
        agents: next,
        player: {
          ...s.player,
          x: preferred.tileX * TILE,
          y: preferred.tileY * TILE,
          targetX: preferred.tileX * TILE,
          targetY: preferred.tileY * TILE,
        },
      });
      store.getState().handleMessage(agentId, houseId);
      return preferred.type as string;
    },
    { agentId, houseId },
  );
}

/** Drive the walk tick-by-tick until the agent reaches a non-walk state,
 * with a generous safety cap. Returns `true` if the walk resolved. */
async function runUntilWalkFinished(
  page: Page,
  agentId: string,
  maxTicks = 4000,
): Promise<boolean> {
  const BATCH = 25;
  for (let i = 0; i < Math.ceil(maxTicks / BATCH); i++) {
    await advanceTicks(page, BATCH);
    const snap = await readAgent(page, agentId);
    if (snap && snap.state !== "walk") return true;
  }
  return false;
}

async function stageAgentFarFromPlayer(page: Page, agentId: string): Promise<boolean> {
  return page.evaluate((agentId) => {
    const store = (window as any).__pixelWorldStore;
    const s = store.getState();
    const agent = s.agents.get(agentId);
    const mask = s.walkabilityMask;
    if (!agent || !s.player || !mask || s.worldWidth <= 0 || s.worldHeight <= 0) {
      return false;
    }

    const tile = 16;
    const cols = Math.max(1, Math.floor(mask.width / tile));
    const rows = Math.max(1, Math.floor(mask.height / tile));
    const worldToMaskX = mask.width / s.worldWidth;
    const worldToMaskY = mask.height / s.worldHeight;
    const maskToWorldX = s.worldWidth / mask.width;
    const maskToWorldY = s.worldHeight / mask.height;
    const idxOf = (col: number, row: number) => row * cols + col;
    const isWalkable = (col: number, row: number) => {
      if (col < 0 || row < 0 || col >= cols || row >= rows) return false;
      const px = col * tile + tile / 2;
      const py = row * tile + tile / 2;
      const idx = (Math.floor(py) * mask.width + Math.floor(px)) * 4;
      return mask.pixels[idx + 3] >= 16
        && mask.pixels[idx] > 240
        && mask.pixels[idx + 1] > 240
        && mask.pixels[idx + 2] > 240;
    };
    const toWorld = (col: number, row: number) => ({
      x: (col * tile + tile / 2) * maskToWorldX,
      y: (row * tile + tile / 2) * maskToWorldY,
    });

    let startCol = Math.floor(agent.anim.x * worldToMaskX / tile);
    let startRow = Math.floor(agent.anim.y * worldToMaskY / tile);
    if (!isWalkable(startCol, startRow)) {
      let snapped: { col: number; row: number } | null = null;
      for (let radius = 1; radius <= 6 && !snapped; radius++) {
        for (let dy = -radius; dy <= radius && !snapped; dy++) {
          for (let dx = -radius; dx <= radius; dx++) {
            const col = startCol + dx;
            const row = startRow + dy;
            if (isWalkable(col, row)) {
              snapped = { col, row };
              break;
            }
          }
        }
      }
      if (!snapped) return false;
      startCol = snapped.col;
      startRow = snapped.row;
    }

    const queue = [{ col: startCol, row: startRow }];
    const seen = new Set([idxOf(startCol, startRow)]);
    let farthest = queue[0];
    for (let i = 0; i < queue.length; i++) {
      const cur = queue[i];
      const curDist = Math.abs(cur.col - startCol) + Math.abs(cur.row - startRow);
      const farDist = Math.abs(farthest.col - startCol) + Math.abs(farthest.row - startRow);
      if (curDist > farDist) farthest = cur;
      for (const [dx, dy] of [[1, 0], [-1, 0], [0, 1], [0, -1]]) {
        const col = cur.col + dx;
        const row = cur.row + dy;
        const key = idxOf(col, row);
        if (seen.has(key) || !isWalkable(col, row)) continue;
        seen.add(key);
        queue.push({ col, row });
      }
    }

    const farDist = Math.abs(farthest.col - startCol) + Math.abs(farthest.row - startRow);
    if (farDist < 4) return false;

    const start = toWorld(startCol, startRow);
    const target = toWorld(farthest.col, farthest.row);
    const next = new Map(s.agents);
    next.set(agentId, {
      ...agent,
      anim: {
        ...agent.anim,
        state: "idle",
        x: start.x,
        y: start.y,
        targetX: start.x,
        targetY: start.y,
        arrivalState: undefined,
        resumeState: undefined,
        bubble: null,
        bubbleTicks: 0,
        ticksInState: 0,
        frame: 0,
      },
    });
    store.setState({
      agents: next,
      player: {
        ...s.player,
        x: target.x,
        y: target.y,
      },
    });
    return true;
  }, agentId);
}

/* ========================================================================== */
/*  Tests — state machine mapping                                              */
/* ========================================================================== */

test.describe.serial("Pixel World — chat → animation state mapping", () => {
  test("handleMessage from a far agent starts a walk with the right arrivalState", async ({
    page,
  }) => {
    const ctx = await openPixelAndWaitForStore(page);
    if (!ctx) { test.skip(); return; }

    const { agentId, otherHouseId } = ctx;
    const staged = await stageAgentFarFromPlayer(page, agentId);
    test.skip(!staged, "could not stage a connected walkable player target");

    // Trigger the mapping: agent receives a message while far from the player.
    const arrival = await page.evaluate(
      ({ agentId, otherHouseId }) => {
        const store = (window as any).__pixelWorldStore;
        const s = store.getState();
        s.handleMessage(agentId, otherHouseId);
        const a = store.getState().agents.get(agentId);
        return {
          state: a.anim.state as string,
          arrivalState: a.anim.arrivalState as string | undefined,
          currentHouseId: a.currentHouseId as string,
        };
      },
      { agentId, otherHouseId },
    );

    // Walk must have kicked off.
    expect(arrival.state).toBe("walk");
    // arrivalState must be one of the allowed persistent states — never 'talking'.
    expect(["typing", "sitting", "idle"]).toContain(arrival.arrivalState ?? "idle");
    // And the agent is now bound to the target room.
    expect(arrival.currentHouseId).toBe(otherHouseId);
  });

  test("walk completes and transitions to the declared arrivalState", async ({
    page,
  }) => {
    const ctx = await openPixelAndWaitForStore(page);
    if (!ctx) { test.skip(); return; }

    const { agentId, otherHouseId } = ctx;
    const staged = await stageAgentFarFromPlayer(page, agentId);
    test.skip(!staged, "could not stage a connected walkable player target");

    await page.evaluate(
      ({ agentId, otherHouseId }) => {
        const store = (window as any).__pixelWorldStore;
        store.getState().handleMessage(agentId, otherHouseId);
      },
      { agentId, otherHouseId },
    );

    const before = await readAgent(page, agentId);
    expect(before?.state).toBe("walk");
    const declared = before?.arrivalState ?? "idle";

    const finished = await runUntilWalkFinished(page, agentId, 8000);
    expect(finished).toBeTruthy();

    const after = await readAgent(page, agentId);
    expect(after?.state).toBe(declared);
    // arrivalState is cleared by tickAgent when the walk resolves.
    expect(after?.arrivalState).toBeUndefined();
  });

  test("talking auto-decays back to typing after TALKING_DECAY_TICKS", async ({
    page,
  }) => {
    const ctx = await openPixelAndWaitForStore(page);
    if (!ctx) { test.skip(); return; }
    const { agentId, deskHouseId } = ctx;
    test.skip(!deskHouseId, "no desk-bearing room in this layout");
    const houseId = deskHouseId!;

    // Seat agent at the desk so the in-place branch fires the 'talking' flash
    // (non-desk rooms only pop a bubble and leave state alone).
    const ptType = await sendMessageAtRoomPoint(page, agentId, houseId);
    expect(ptType === "desk" || ptType === "reception_desk").toBeTruthy();

    const talking = await readAgent(page, agentId);
    expect(talking?.state).toBe("talking");

    // Advance past TALKING_DECAY_TICKS (=60) with plenty of slack.
    await advanceTicks(page, 70);

    const afterDecay = await readAgent(page, agentId);
    expect(afterDecay?.state).toBe("typing");
    expect(afterDecay?.ticksInState).toBeLessThan(70);
  });

  test("mention while walking preserves the walk (only surfaces the bubble)", async ({
    page,
  }) => {
    const ctx = await openPixelAndWaitForStore(page);
    if (!ctx) { test.skip(); return; }
    const { agentId, otherHouseId } = ctx;
    const staged = await stageAgentFarFromPlayer(page, agentId);
    test.skip(!staged, "could not stage a connected walkable player target");

    // Kick off a walk.
    await page.evaluate(
      ({ agentId, otherHouseId }) => {
        const store = (window as any).__pixelWorldStore;
        store.getState().handleMessage(agentId, otherHouseId);
      },
      { agentId, otherHouseId },
    );

    // Immediately @mention the agent.
    await page.evaluate((agentId) => {
      const store = (window as any).__pixelWorldStore;
      store.getState().handleMention(agentId);
    }, agentId);

    const snap = await readAgent(page, agentId);
    // Critical invariant: walk state is NOT blown away.
    expect(snap?.state).toBe("walk");
    // But the bubble surfaces the mention.
    expect(snap?.bubble).toBe("💡");
    expect(snap?.bubbleTicks).toBeGreaterThan(0);
  });

  test("mention while typing stashes resumeState so agent returns to desk", async ({
    page,
  }) => {
    const ctx = await openPixelAndWaitForStore(page);
    if (!ctx) { test.skip(); return; }
    const { agentId, deskHouseId } = ctx;
    test.skip(!deskHouseId, "no desk-bearing room in this layout");
    const houseId = deskHouseId!;

    // Teleport the agent to a desk in the target room and reset to idle.
    const ptType = await sendMessageAtRoomPoint(page, agentId, houseId);
    expect(ptType === "desk" || ptType === "reception_desk").toBeTruthy();

    // handleMessage in-place on a desk flips to 'talking' → auto-decays to 'typing'.
    await advanceTicks(page, 70);
    const seated = await readAgent(page, agentId);
    expect(seated?.state).toBe("typing");

    // Now @mention: expect 'thinking' with resumeState === 'typing'.
    await page.evaluate((agentId) => {
      const store = (window as any).__pixelWorldStore;
      store.getState().handleMention(agentId);
    }, agentId);

    const mentioned = await readAgent(page, agentId);
    expect(mentioned?.state).toBe("thinking");
    expect(mentioned?.resumeState).toBe("typing");
    expect(mentioned?.bubble).toBe("💡");
  });

  test("repeated mention while thinking preserves the original resumeState", async ({
    page,
  }) => {
    const ctx = await openPixelAndWaitForStore(page);
    if (!ctx) { test.skip(); return; }
    const { agentId, deskHouseId } = ctx;
    test.skip(!deskHouseId, "no desk-bearing room in this layout");
    const houseId = deskHouseId!;

    const ptType = await sendMessageAtRoomPoint(page, agentId, houseId);
    expect(ptType === "desk" || ptType === "reception_desk").toBeTruthy();

    // Seat agent at desk → typing.
    await advanceTicks(page, 70);

    // First mention: resumeState captured as 'typing'.
    await page.evaluate((agentId) => {
      const store = (window as any).__pixelWorldStore;
      store.getState().handleMention(agentId);
    }, agentId);
    const first = await readAgent(page, agentId);
    expect(first?.state).toBe("thinking");
    expect(first?.resumeState).toBe("typing");

    // Second mention while still thinking: must NOT clobber resumeState.
    await page.evaluate((agentId) => {
      const store = (window as any).__pixelWorldStore;
      store.getState().handleMention(agentId);
    }, agentId);
    const second = await readAgent(page, agentId);
    expect(second?.state).toBe("thinking");
    expect(second?.resumeState).toBe("typing");
  });

  test("wanderAgentInRoom only fires for idle agents and lands back at idle", async ({
    page,
  }) => {
    const ctx = await openPixelAndWaitForStore(page);
    if (!ctx) { test.skip(); return; }
    const { agentId, otherHouseId } = ctx;

    // Put the agent in a known non-idle state (walking) and try to wander —
    // the call must be a no-op so in-flight walks aren't cancelled.
    await page.evaluate(
      (agentId) => {
        const store = (window as any).__pixelWorldStore;
        const s = store.getState();
        const agent = s.agents.get(agentId);
        if (!agent) return;
        const next = new Map(s.agents);
        next.set(agentId, {
          ...agent,
          anim: {
            ...agent.anim,
            state: "walk",
            targetX: agent.anim.x + 16,
            targetY: agent.anim.y,
            arrivalState: "idle",
            ticksInState: 0,
            frame: 0,
          },
        });
        store.setState({ agents: next });
      },
      agentId,
    );
    const walkingBefore = await readAgent(page, agentId);
    expect(walkingBefore?.state).toBe("walk");
    const targetXBefore = walkingBefore?.targetX;

    await page.evaluate((agentId) => {
      const store = (window as any).__pixelWorldStore;
      store.getState().wanderAgentInRoom(agentId);
    }, agentId);

    const walkingAfter = await readAgent(page, agentId);
    expect(walkingAfter?.state).toBe("walk");
    // Same trip — the wander call did not retarget.
    expect(walkingAfter?.targetX).toBe(targetXBefore);

    // Force idle so the wander precondition is met.
    await page.evaluate((agentId) => {
      const store = (window as any).__pixelWorldStore;
      const s = store.getState();
      const a = s.agents.get(agentId);
      if (!a) return;
      const next = new Map(s.agents);
      next.set(agentId, {
        ...a,
        anim: { ...a.anim, state: "idle", ticksInState: 0, frame: 0 },
      });
      store.setState({ agents: next });
    }, agentId);

    await page.evaluate((agentId) => {
      const store = (window as any).__pixelWorldStore;
      store.getState().wanderAgentInRoom(agentId);
    }, agentId);

    const wandered = await readAgent(page, agentId);
    // Either no interaction points available (no-op → still idle) or
    // a fresh walk headed for an idle arrival.
    if (wandered?.state === "walk") {
      expect(wandered.arrivalState ?? "idle").toBe("idle");
      const finished = await runUntilWalkFinished(page, agentId, 8000);
      expect(finished).toBeTruthy();
      const final = await readAgent(page, agentId);
      expect(final?.state).toBe("idle");
    } else {
      expect(wandered?.state).toBe("idle");
    }
  });
});
