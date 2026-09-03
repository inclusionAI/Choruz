import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Pixel World</h1>
      <p className="subtitle">A fun pixel-art office visualization where your agents walk around as animated characters in a tile-based virtual office.</p>

      <h2>Overview</h2>
      <p>Pixel World is a built-in Host/Client plugin that renders your agents as pixel-art characters in a virtual office environment. The Host advertises its roster and activity capabilities, and a compatible Client contributes the sidebar action and overlay.</p>

      <h2>The Virtual Office</h2>
      <p>The office is a tile-based map drawn on an HTML Canvas element. Each tile represents a position in the office grid. The map includes:</p>
      <ul>
        <li><strong>Desks</strong> &mdash; Where agents sit when idle</li>
        <li><strong>Walkways</strong> &mdash; Paths between desks where agents walk</li>
        <li><strong>Meeting areas</strong> &mdash; Open spaces for group interactions</li>
      </ul>

      <h2>Agent Characters</h2>
      <p>Each agent in your workspace is represented as a pixel-art character. The characters are rendered using Canvas drawing primitives &mdash; no external sprite images are needed.</p>

      <h3>Character States</h3>
      <table>
        <thead><tr><th>State</th><th>Visual</th><th>Meaning</th></tr></thead>
        <tbody>
          <tr><td>Idle</td><td>Character at desk with a speech bubble</td><td>Agent is not actively processing a message</td></tr>
          <tr><td>Working</td><td>Character with active animation</td><td>Agent is currently processing a task</td></tr>
          <tr><td>Walking</td><td>Character moving between tiles with dust particles</td><td>Agent is transitioning between states</td></tr>
        </tbody>
      </table>

      <h3>Visual Indicators</h3>
      <ul>
        <li><strong>Sleep bubbles</strong> &mdash; Idle agents display animated sleep bubbles above their character</li>
        <li><strong>Walking dust</strong> &mdash; Small particle effects appear at the character{"'"}s feet while walking</li>
        <li><strong>Name labels</strong> &mdash; Each character has their agent name displayed below</li>
      </ul>

      <h2>Interaction</h2>
      <ul>
        <li><strong>Click to select</strong> &mdash; Click on an agent character to switch to their conversation in the sidebar</li>
        <li><strong>Hover</strong> &mdash; Hovering over a character shows additional information about the agent</li>
      </ul>

      <h2>Technical Implementation</h2>

      <h3>Canvas Rendering</h3>
      <p>Pixel World uses the HTML Canvas API for rendering. All characters, tiles, and effects are drawn programmatically using Canvas primitives (rectangles, arcs, lines). This keeps the component lightweight with no external asset dependencies.</p>

      <h3>Lazy Loading</h3>
      <p>The Pixel World component is lazy-loaded to avoid impacting the initial page load time. It is only rendered when the user navigates to the Pixel World view.</p>

      <pre><code>{`// Lazy-loaded component
const PixelWorld = dynamic(() => import("./pixel-world"), {
  ssr: false,
  loading: () => <div>Loading Pixel World...</div>,
});`}</code></pre>

      <h3>Tile System</h3>
      <p>The office map is a 2D grid of tiles. Each tile has a type (floor, desk, wall, walkway) that determines its appearance and whether agents can walk on it. The tile map is defined as a simple 2D array:</p>

      <pre><code>{`// Tile types
const FLOOR = 0;
const DESK = 1;
const WALL = 2;
const WALKWAY = 3;

// Example tile map (simplified)
const map = [
  [2, 2, 2, 2, 2, 2],
  [2, 0, 1, 0, 1, 2],
  [2, 3, 3, 3, 3, 2],
  [2, 0, 1, 0, 1, 2],
  [2, 2, 2, 2, 2, 2],
];`}</code></pre>

      <h3>Character Variety</h3>
      <p>Pixel World supports a variety of character designs to visually distinguish agents. Characters are differentiated by color palette, drawn entirely with Canvas primitives.</p>

      <h2>When to Use Pixel World</h2>
      <ul>
        <li><strong>Team overview</strong> &mdash; Get a quick visual sense of which agents are busy vs idle</li>
        <li><strong>Demos</strong> &mdash; Fun way to showcase your agent team to stakeholders</li>
        <li><strong>Monitoring</strong> &mdash; Visual monitoring of agent activity at a glance</li>
      </ul>

      <div className="callout callout-info">
        <strong>Optional feature</strong>
        Pixel World is purely cosmetic and does not affect agent functionality. Exclude <code>pixel-world</code> from <code>CHORUZ_PLUGINS</code> to remove both its Host manifest and Client UI contribution.
      </div>

      <div className="callout callout-tip">
        <strong>Performance</strong>
        Because Pixel World is lazy-loaded and rendered on Canvas, it has minimal impact on overall application performance. The component only renders when visible and uses requestAnimationFrame for smooth animations.
      </div>

      <div className="docs-pager">
        <Link href="/docs/features/cron-scheduler">
          <span className="docs-pager-label">Previous</span>
          Cron Scheduler
        </Link>
        <Link href="/docs/features/server-management">
          <span className="docs-pager-label">Next</span>
          Remote Servers (SSH)
        </Link>
      </div>
    </>
  );
}
