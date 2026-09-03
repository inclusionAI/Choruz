import { describe, expect, it } from "vitest";
import {
  findMostRecentHouse,
  findPath,
  generateOfficeLayout,
} from "./pixel-world-store";
import type { TileGrid } from "./pixel-tiles";

// Helpers ---------------------------------------------------------------------

function makeTileGrid(walkableValue: number, blockedValue: number, walkable: boolean[][]): TileGrid {
  const rows = walkable.length;
  const cols = walkable[0].length;
  const layer = new Int8Array(cols * rows);
  for (let r = 0; r < rows; r++) {
    for (let c = 0; c < cols; c++) {
      layer[r * cols + c] = walkable[r][c] ? walkableValue : blockedValue;
    }
  }
  return { cols, rows, layers: [layer] };
}

// findPath (TileGrid variant) -------------------------------------------------
// Walkable tile codes: 1, 2, 3, 4, 7, 9. Anything else is blocked.

describe("findPath (TileGrid)", () => {
  it("returns single-tile-centre path when start cell == dest cell", () => {
    const grid = makeTileGrid(1, 0, [[true, true]]);
    // Same cell (col=0, row=0): start (4,4), dest (8,8) — both in cell (0,0).
    const path = findPath(grid, 4, 4, 8, 8);
    expect(path).toEqual([{ x: 8, y: 8 }]);
  });

  it("finds a corridor path on a 5×1 walkable grid (tile=1)", () => {
    const grid = makeTileGrid(1, 0, [[true, true, true, true, true]]);
    const path = findPath(grid, 8, 8, 72, 8);
    expect(path.length).toBeGreaterThan(0);
    // Final waypoint must land on cell-centre (col*16 + 8) for col=4 → x=72.
    expect(path[path.length - 1]).toEqual({ x: 72, y: 8 });
  });

  it("falls back to a single dest waypoint when no path exists", () => {
    // Surround (0,0) with non-walkable tiles. tile=5 (WALL) is blocked.
    const grid = makeTileGrid(1, 5, [
      [true, false],
      [false, false],
    ]);
    // Start (8,8) is walkable cell, dest (24,24) is blocked cell, no neighbour
    // is walkable → no path → fallback.
    const path = findPath(grid, 8, 8, 24, 24);
    expect(path).toEqual([{ x: 24, y: 24 }]);
  });

  it("treats DOOR (7) and HALLWAY (4) as walkable, WALL (5) as blocked", () => {
    // Layout: [HALLWAY, WALL, DOOR] — col0 reachable to col2 only via going
    // around, but on 1 row the wall blocks everything → fallback.
    const grid: TileGrid = {
      cols: 3,
      rows: 1,
      layers: [new Int8Array([4, 5, 7])],
    };
    const path = findPath(grid, 8, 8, 40, 8);
    expect(path).toEqual([{ x: 40, y: 8 }]); // wall blocks → fallback
  });
});

// findMostRecentHouse ---------------------------------------------------------

describe("findMostRecentHouse", () => {
  it("returns null when the agent has no messages in any house", () => {
    const got = findMostRecentHouse(
      "agent-1",
      { c1: [{ sender_id: "alice", created_at: "2026-05-01T00:00:00Z" }] },
      new Set(["c1"]),
    );
    expect(got).toBeNull();
  });

  it("returns the conv where the agent most recently spoke", () => {
    const got = findMostRecentHouse(
      "agent-1",
      {
        c1: [{ sender_id: "agent-1", created_at: "2026-05-01T00:00:00Z" }],
        c2: [{ sender_id: "agent-1", created_at: "2026-05-05T00:00:00Z" }],
        c3: [{ sender_id: "agent-1", created_at: "2026-05-03T00:00:00Z" }],
      },
      new Set(["c1", "c2", "c3"]),
    );
    expect(got).toBe("c2");
  });

  it("ignores convs that aren't in houseIds", () => {
    const got = findMostRecentHouse(
      "agent-1",
      {
        c1: [{ sender_id: "agent-1", created_at: "2026-05-01T00:00:00Z" }],
        c2: [{ sender_id: "agent-1", created_at: "2026-05-05T00:00:00Z" }],
      },
      new Set(["c1"]),
    );
    expect(got).toBe("c1");
  });

  it("accepts senderId / timestamp aliases (camelCase + numeric ts)", () => {
    const got = findMostRecentHouse(
      "agent-1",
      {
        c1: [{ senderId: "agent-1", timestamp: 1000 }],
        c2: [{ sender_id: "agent-1", timestamp: 5000 }],
      },
      new Set(["c1", "c2"]),
    );
    expect(got).toBe("c2");
  });

  it("only inspects the last message-by-this-agent in each conv", () => {
    // c1's most recent message is from someone else; the agent's last (older)
    // message in c1 still establishes "agent's most recent in c1".
    const got = findMostRecentHouse(
      "agent-1",
      {
        c1: [
          { sender_id: "agent-1", created_at: "2026-05-01T00:00:00Z" },
          { sender_id: "alice", created_at: "2026-05-09T00:00:00Z" },
        ],
        c2: [
          { sender_id: "agent-1", created_at: "2026-05-03T00:00:00Z" },
        ],
      },
      new Set(["c1", "c2"]),
    );
    expect(got).toBe("c2"); // agent's last in c1 was 05-01; in c2 was 05-03
  });
});

// generateOfficeLayout --------------------------------------------------------

describe("generateOfficeLayout — output shape", () => {
  it("returns a Uint8Array grid of size gridWidth × gridHeight", () => {
    const layout = generateOfficeLayout([
      { id: "g1", memberCount: 3, name: "Engineering" },
      { id: "g2", memberCount: 8, name: "Sales" },
    ]);
    expect(layout.grid).toBeInstanceOf(Uint8Array);
    expect(layout.grid.length).toBe(layout.gridWidth * layout.gridHeight);
    expect(layout.gridWidth).toBeGreaterThan(0);
    expect(layout.gridHeight).toBeGreaterThan(0);
  });

  it("scales grid dimensions with group count (more groups → bigger world)", () => {
    const small = generateOfficeLayout([
      { id: "g1", memberCount: 3, name: "A" },
    ]);
    const large = generateOfficeLayout(
      Array.from({ length: 16 }, (_, i) => ({
        id: `g${i}`,
        memberCount: 5,
        name: `Group ${i}`,
      })),
    );
    expect(large.gridWidth).toBeGreaterThanOrEqual(small.gridWidth);
    expect(large.gridHeight).toBeGreaterThan(small.gridHeight);
  });

  it("classifies room type by member count", () => {
    const layout = generateOfficeLayout([
      { id: "g1", memberCount: 2, name: "Tiny" },     // < 5 → lounge
      { id: "g2", memberCount: 7, name: "Medium" },   // 5..10 → workspace
      { id: "g3", memberCount: 20, name: "Large" },   // > 10 → conference
      { id: "g4", memberCount: 3, name: "Conference Room" }, // name override
    ]);
    const roomById = (id: string) => layout.rooms.find((r) => r.id === id);
    // The layout always emits one room per group at minimum.
    expect(layout.rooms.length).toBeGreaterThanOrEqual(4);
    expect(roomById("g1")?.roomType).toBe("lounge");
    expect(roomById("g2")?.roomType).toBe("workspace");
    expect(roomById("g3")?.roomType).toBe("conference");
    // 'conference' name override beats memberCount=3 default of lounge
    expect(roomById("g4")?.roomType).toBe("conference");
  });

  it("places every group room within grid bounds", () => {
    const layout = generateOfficeLayout([
      { id: "g1", memberCount: 5, name: "A" },
      { id: "g2", memberCount: 5, name: "B" },
      { id: "g3", memberCount: 5, name: "C" },
    ]);
    const groupRooms = layout.rooms.filter((r) =>
      ["lounge", "workspace", "conference"].includes(r.roomType),
    );
    for (const room of groupRooms) {
      expect(room.tileX).toBeGreaterThanOrEqual(0);
      expect(room.tileY).toBeGreaterThanOrEqual(0);
      expect(room.tileX + room.width).toBeLessThanOrEqual(layout.gridWidth);
      expect(room.tileY + room.height).toBeLessThanOrEqual(layout.gridHeight);
    }
  });

  it("handles a single-group input without throwing", () => {
    expect(() =>
      generateOfficeLayout([{ id: "solo", memberCount: 3, name: "Solo" }]),
    ).not.toThrow();
  });
});
