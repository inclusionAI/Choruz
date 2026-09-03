import Link from "next/link";

export default function Installation() {
  return (
    <>
      <h1>Install and run Choruz</h1>
      <p className="subtitle">Start a local development stack from the public repository.</p>

      <h2>Prerequisites</h2>
      <table>
        <thead><tr><th>Tool</th><th>Version</th><th>Purpose</th></tr></thead>
        <tbody>
          <tr><td><code>Rust</code></td><td><code>rust-toolchain.toml</code></td><td>API gateway and pipeline</td></tr>
          <tr><td><code>Node.js</code></td><td>24</td><td>Web application and scripts</td></tr>
          <tr><td><code>pnpm</code></td><td>10</td><td>Workspace package manager</td></tr>
          <tr><td><code>PostgreSQL</code></td><td>16</td><td>Local data store</td></tr>
        </tbody>
      </table>

      <div className="callout callout-info">
        <strong>Agent CLIs (optional)</strong>
        To run an agent, install and sign in to at least one supported CLI. See <Link href="/docs/agents/drivers">Drivers &amp; Accounts</Link> for the current list.
      </div>

      <h2>Clone and install</h2>
      <pre><code>{`git clone https://github.com/jcguo123/Choruz.git
cd Choruz
corepack enable
pnpm install`}</code></pre>

      <h2>Start the stack</h2>
      <p>Start PostgreSQL, migrations, the API gateway, and the message pipeline:</p>
      <pre><code>pnpm dev:all</code></pre>
      <p>Then start the web app in another terminal:</p>
      <pre><code>pnpm dev:web</code></pre>
      <p>Open the Web URL printed by the second command. The standard port is <code>3100</code>; local checkouts automatically choose and remember another free port when a standard port is busy.</p>

      <h2>Stop or reload</h2>
      <pre><code>{`pnpm stop:all
pnpm reload:local`}</code></pre>
      <p><code>reload:local</code> pulls no code. Run <code>git pull --ff-only</code> first when you want to restart on the latest <code>main</code>.</p>

      <div className="callout callout-info">
        <strong>Production deployment</strong>
        The commands above are for local use. Follow the <Link href="/docs/operations/deployment">deployment guide</Link> before exposing Choruz on a server.
      </div>

      <div className="docs-pager">
        <Link href="/docs">
          <span className="docs-pager-label">Previous</span>
          Welcome
        </Link>
        <Link href="/docs/getting-started/quickstart">
          <span className="docs-pager-label">Next</span>
          Quick Start
        </Link>
      </div>
    </>
  );
}
