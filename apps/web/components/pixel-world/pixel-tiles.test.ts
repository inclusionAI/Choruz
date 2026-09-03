import { describe, expect, it } from "vitest";
import {
  generateWorldGrid,
  type TileGrid,
} from "./pixel-tiles";
import type { OfficeLayout } from "./pixel-tiles";

// Tile values — duplicated from the const enum because TypeScript's const-enum
// inlining is unreliable across compilers. Keep these in sync with pixel-tiles.ts.
const T = {
  VOID: 0,
  CARPET_A: 1,
  CARPET_B: 2,
  WOOD_FLOOR: 3,
  KITCHEN_TILE: 4,
  HALLWAY: 5,
  WALL: 6,
  GLASS_WALL: 7,
  DOOR: 8,
  WINDOW: 9,
  CONCRETE: 10,

  // auto-tiled wall variants
  WALL_SINGLE: 11,
  WALL_TOP_END: 12,
  WALL_BOTTOM_END: 13,
  WALL_LEFT_END: 14,
  WALL_RIGHT_END: 15,
  WALL_VERTICAL: 16,
  WALL_HORIZONTAL: 17,
  WALL_TOP_LEFT_CORNER: 18,
  WALL_TOP_RIGHT_CORNER: 19,
  WALL_BOTTOM_LEFT_CORNER: 20,
  WALL_BOTTOM_RIGHT_CORNER: 21,
  WALL_T_DOWN: 22,
  WALL_T_UP: 23,
  WALL_T_LEFT: 24,
  WALL_T_RIGHT: 25,
  WALL_CROSS: 26,
} as const;

// LAYOUT_TILE byte codes used in OfficeLayout.grid
const L = {
  VOID: 0,
  CARPET: 1,
  WOOD_FLOOR: 2,
  KITCHEN_TILE: 3,
  HALLWAY: 4,
  WALL: 5,
  GLASS_WALL: 6,
  DOOR: 7,
  WINDOW: 8,
  CONCRETE: 9,
  EXTERIOR_GROUND: 10,
  EXTERIOR_WALL: 11,
} as const;

// Build a layout from a 2-D byte matrix.
function makeLayout(rows: number[][]): OfficeLayout {
  const gridHeight = rows.length;
  const gridWidth = rows[0].length;
  const grid = new Uint8Array(gridHeight * gridWidth);
  for (let r = 0; r < gridHeight; r++) {
    for (let c = 0; c < gridWidth; c++) {
      grid[r * gridWidth + c] = rows[r][c];
    }
  }
  return { rooms: [], grid, gridWidth, gridHeight };
}

function tileAt(grid: TileGrid, c: number, r: number): number {
  return grid.layers[0][r * grid.cols + c];
}

// generateWorldGrid: layout-byte → Tile mapping ------------------------------

describe("generateWorldGrid — direct layout-byte mapping", () => {
  it("maps each LAYOUT_TILE byte to the corresponding Tile", () => {
    const layout = makeLayout([
      [L.VOID, L.CARPET, L.WOOD_FLOOR, L.KITCHEN_TILE, L.HALLWAY],
    ]);
    const g = generateWorldGrid(layout);
    expect(tileAt(g, 0, 0)).toBe(T.VOID);
    // CARPET alternates A/B by parity (c+r)%2
    expect(tileAt(g, 1, 0)).toBe(T.CARPET_B); // c+r=1 odd → CARPET_B
    expect(tileAt(g, 2, 0)).toBe(T.WOOD_FLOOR);
    expect(tileAt(g, 3, 0)).toBe(T.KITCHEN_TILE);
    expect(tileAt(g, 4, 0)).toBe(T.HALLWAY);
  });

  it("alternates CARPET_A and CARPET_B in a checker pattern", () => {
    const layout = makeLayout([
      [L.CARPET, L.CARPET],
      [L.CARPET, L.CARPET],
    ]);
    const g = generateWorldGrid(layout);
    expect(tileAt(g, 0, 0)).toBe(T.CARPET_A); // 0+0 even
    expect(tileAt(g, 1, 0)).toBe(T.CARPET_B); // 1+0 odd
    expect(tileAt(g, 0, 1)).toBe(T.CARPET_B); // 0+1 odd
    expect(tileAt(g, 1, 1)).toBe(T.CARPET_A); // 1+1 even
  });

  it("maps GLASS_WALL, DOOR, WINDOW, CONCRETE", () => {
    const layout = makeLayout([
      [L.GLASS_WALL, L.DOOR, L.WINDOW, L.CONCRETE],
    ]);
    const g = generateWorldGrid(layout);
    expect(tileAt(g, 0, 0)).toBe(T.GLASS_WALL);
    expect(tileAt(g, 1, 0)).toBe(T.DOOR);
    expect(tileAt(g, 2, 0)).toBe(T.WINDOW);
    expect(tileAt(g, 3, 0)).toBe(T.CONCRETE);
  });

  it("falls through unknown layout bytes to EXTERIOR_GROUND", () => {
    const layout = makeLayout([[100]]);
    const g = generateWorldGrid(layout);
    expect(tileAt(g, 0, 0)).toBe(29); // T.EXTERIOR_GROUND = 29
  });

  it("output dimensions match input dimensions", () => {
    const layout = makeLayout([
      [L.CARPET, L.CARPET, L.CARPET],
      [L.CARPET, L.CARPET, L.CARPET],
    ]);
    const g = generateWorldGrid(layout);
    expect(g.cols).toBe(3);
    expect(g.rows).toBe(2);
    expect(g.layers[0].length).toBe(6);
  });
});

// Auto-tiled wall variants ----------------------------------------------------
//
// Mask = up | (down<<1) | (left<<2) | (right<<3) where each neighbour bit is
// set when adjacent tile is "wall-like" (WALL itself, any auto-tiled WALL_*,
// WINDOW, or EXTERIOR_WALL). The mask determines which of 16 auto-tile
// variants is selected.

describe("generateWorldGrid — auto-tiled wall variants", () => {
  it("isolated wall (no neighbours) → WALL_SINGLE (mask=0)", () => {
    // 3×3 of CARPET with a single WALL in the centre.
    const layout = makeLayout([
      [L.CARPET, L.CARPET, L.CARPET],
      [L.CARPET, L.WALL,   L.CARPET],
      [L.CARPET, L.CARPET, L.CARPET],
    ]);
    const g = generateWorldGrid(layout);
    expect(tileAt(g, 1, 1)).toBe(T.WALL_SINGLE);
  });

  it("wall with only up neighbour → WALL_BOTTOM_END (mask=1)", () => {
    const layout = makeLayout([
      [L.CARPET, L.WALL,   L.CARPET],
      [L.CARPET, L.WALL,   L.CARPET],
      [L.CARPET, L.CARPET, L.CARPET],
    ]);
    const g = generateWorldGrid(layout);
    // The lower wall sees up=wall, others not → mask=1 → WALL_BOTTOM_END
    expect(tileAt(g, 1, 1)).toBe(T.WALL_BOTTOM_END);
  });

  it("wall with only down neighbour → WALL_TOP_END (mask=2)", () => {
    const layout = makeLayout([
      [L.CARPET, L.CARPET, L.CARPET],
      [L.CARPET, L.WALL,   L.CARPET],
      [L.CARPET, L.WALL,   L.CARPET],
    ]);
    const g = generateWorldGrid(layout);
    expect(tileAt(g, 1, 1)).toBe(T.WALL_TOP_END);
  });

  it("vertical run (up+down neighbours) → WALL_VERTICAL (mask=3)", () => {
    const layout = makeLayout([
      [L.CARPET, L.WALL,   L.CARPET],
      [L.CARPET, L.WALL,   L.CARPET],
      [L.CARPET, L.WALL,   L.CARPET],
    ]);
    const g = generateWorldGrid(layout);
    expect(tileAt(g, 1, 1)).toBe(T.WALL_VERTICAL);
  });

  it("horizontal run (left+right) → WALL_HORIZONTAL (mask=12)", () => {
    const layout = makeLayout([
      [L.CARPET, L.CARPET, L.CARPET, L.CARPET, L.CARPET],
      [L.WALL,   L.WALL,   L.WALL,   L.WALL,   L.WALL  ],
      [L.CARPET, L.CARPET, L.CARPET, L.CARPET, L.CARPET],
    ]);
    const g = generateWorldGrid(layout);
    expect(tileAt(g, 2, 1)).toBe(T.WALL_HORIZONTAL);
  });

  it("top-left corner (down + right) → WALL_TOP_LEFT_CORNER (mask=10)", () => {
    // Mask: down=1, right=1 → bit2=down(1) → 2, bit4=right(1) → 8 → 10.
    const layout = makeLayout([
      [L.WALL,   L.WALL,   L.CARPET],
      [L.WALL,   L.CARPET, L.CARPET],
      [L.CARPET, L.CARPET, L.CARPET],
    ]);
    const g = generateWorldGrid(layout);
    expect(tileAt(g, 0, 0)).toBe(T.WALL_TOP_LEFT_CORNER);
  });

  it("top-right corner (down + left) → WALL_TOP_RIGHT_CORNER (mask=6)", () => {
    const layout = makeLayout([
      [L.CARPET, L.WALL,   L.WALL  ],
      [L.CARPET, L.CARPET, L.WALL  ],
      [L.CARPET, L.CARPET, L.CARPET],
    ]);
    const g = generateWorldGrid(layout);
    expect(tileAt(g, 2, 0)).toBe(T.WALL_TOP_RIGHT_CORNER);
  });

  it("bottom-left corner (up + right) → WALL_BOTTOM_LEFT_CORNER (mask=9)", () => {
    const layout = makeLayout([
      [L.WALL,   L.CARPET, L.CARPET],
      [L.WALL,   L.WALL,   L.CARPET],
      [L.CARPET, L.CARPET, L.CARPET],
    ]);
    const g = generateWorldGrid(layout);
    expect(tileAt(g, 0, 1)).toBe(T.WALL_BOTTOM_LEFT_CORNER);
  });

  it("bottom-right corner (up + left) → WALL_BOTTOM_RIGHT_CORNER (mask=5)", () => {
    const layout = makeLayout([
      [L.CARPET, L.CARPET, L.WALL  ],
      [L.CARPET, L.WALL,   L.WALL  ],
      [L.CARPET, L.CARPET, L.CARPET],
    ]);
    const g = generateWorldGrid(layout);
    expect(tileAt(g, 2, 1)).toBe(T.WALL_BOTTOM_RIGHT_CORNER);
  });

  it("4-way junction (all 4 neighbours) → WALL_CROSS (mask=15)", () => {
    const layout = makeLayout([
      [L.CARPET, L.WALL,   L.CARPET],
      [L.WALL,   L.WALL,   L.WALL  ],
      [L.CARPET, L.WALL,   L.CARPET],
    ]);
    const g = generateWorldGrid(layout);
    expect(tileAt(g, 1, 1)).toBe(T.WALL_CROSS);
  });

  // T-junction naming reflects which side has the gap, not the stem direction.
  // mask layout: bit0=up, bit1=down, bit2=left, bit3=right.

  it("up missing (down+left+right, mask=14) → WALL_T_RIGHT", () => {
    const layout = makeLayout([
      [L.CARPET, L.CARPET, L.CARPET],
      [L.WALL,   L.WALL,   L.WALL  ],
      [L.CARPET, L.WALL,   L.CARPET],
    ]);
    const g = generateWorldGrid(layout);
    expect(tileAt(g, 1, 1)).toBe(T.WALL_T_RIGHT);
  });

  it("down missing (up+left+right, mask=13) → WALL_T_DOWN", () => {
    const layout = makeLayout([
      [L.CARPET, L.WALL,   L.CARPET],
      [L.WALL,   L.WALL,   L.WALL  ],
      [L.CARPET, L.CARPET, L.CARPET],
    ]);
    const g = generateWorldGrid(layout);
    expect(tileAt(g, 1, 1)).toBe(T.WALL_T_DOWN);
  });

  it("right missing (up+down+left, mask=7) → WALL_T_LEFT", () => {
    const layout = makeLayout([
      [L.CARPET, L.WALL,   L.CARPET],
      [L.WALL,   L.WALL,   L.CARPET],
      [L.CARPET, L.WALL,   L.CARPET],
    ]);
    const g = generateWorldGrid(layout);
    expect(tileAt(g, 1, 1)).toBe(T.WALL_T_LEFT);
  });

  it("left missing (up+down+right, mask=11) → WALL_T_UP", () => {
    const layout = makeLayout([
      [L.CARPET, L.WALL,   L.CARPET],
      [L.CARPET, L.WALL,   L.WALL  ],
      [L.CARPET, L.WALL,   L.CARPET],
    ]);
    const g = generateWorldGrid(layout);
    expect(tileAt(g, 1, 1)).toBe(T.WALL_T_UP);
  });

  it("WINDOW counts as wall-like for the purposes of the auto-tile mask", () => {
    // A WALL with WINDOW above → up neighbour is wall-like → mask includes up.
    const layout = makeLayout([
      [L.CARPET, L.WINDOW, L.CARPET],
      [L.CARPET, L.WALL,   L.CARPET],
      [L.CARPET, L.CARPET, L.CARPET],
    ]);
    const g = generateWorldGrid(layout);
    // mask=1 (only up) → WALL_BOTTOM_END
    expect(tileAt(g, 1, 1)).toBe(T.WALL_BOTTOM_END);
  });

  it("EXTERIOR_WALL also counts as wall-like for the mask", () => {
    const layout = makeLayout([
      [L.CARPET,        L.EXTERIOR_WALL, L.CARPET],
      [L.CARPET,        L.WALL,          L.CARPET],
      [L.CARPET,        L.CARPET,        L.CARPET],
    ]);
    const g = generateWorldGrid(layout);
    expect(tileAt(g, 1, 1)).toBe(T.WALL_BOTTOM_END);
  });

  it("DOOR does NOT count as wall-like (gaps in wall runs)", () => {
    // Wall row with a door interrupting it: WALL DOOR WALL.
    // The middle WALLs each have only one wall-like neighbour (the other side
    // is DOOR which is not wall-like).
    const layout = makeLayout([
      [L.CARPET, L.CARPET, L.CARPET, L.CARPET, L.CARPET],
      [L.WALL,   L.DOOR,   L.WALL,   L.DOOR,   L.WALL  ],
      [L.CARPET, L.CARPET, L.CARPET, L.CARPET, L.CARPET],
    ]);
    const g = generateWorldGrid(layout);
    // Wall at (0,1): right is DOOR → no wall neighbour → WALL_SINGLE
    expect(tileAt(g, 0, 1)).toBe(T.WALL_SINGLE);
    // Wall at (2,1): both left and right are DOOR → no wall neighbour
    expect(tileAt(g, 2, 1)).toBe(T.WALL_SINGLE);
  });
});

// Boundary handling -----------------------------------------------------------

describe("generateWorldGrid — boundary cases", () => {
  it("treats out-of-bounds neighbours as VOID (not wall-like)", () => {
    // Wall in the corner — its up and left neighbours don't exist.
    const layout = makeLayout([
      [L.WALL,   L.WALL  ],
      [L.WALL,   L.CARPET],
    ]);
    const g = generateWorldGrid(layout);
    // (0,0) — up=void, down=WALL, left=void, right=WALL → mask=10 → corner
    expect(tileAt(g, 0, 0)).toBe(T.WALL_TOP_LEFT_CORNER);
  });

  it("does not crash on a 1×1 grid containing a single wall", () => {
    expect(() => generateWorldGrid(makeLayout([[L.WALL]]))).not.toThrow();
    const g = generateWorldGrid(makeLayout([[L.WALL]]));
    expect(tileAt(g, 0, 0)).toBe(T.WALL_SINGLE);
  });
});
