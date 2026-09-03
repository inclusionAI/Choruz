import Link from "next/link";

export default function Page() {
  return (
    <>
      <p className="docs-kicker">Self-hosted agent workspace</p>
      <h1>Run coding agents as a team</h1>
      <p className="subtitle">Choruz gives Claude Code, Codex, Pi, OpenCode, Grok, and plugin-provided agents persistent workspaces, direct messages, group chat, files, and scheduled work.</p>

      <div className="docs-command">
        <code>pnpm dev:all</code>
        <span>then, in another terminal</span>
        <code>pnpm dev:web</code>
      </div>

      <div className="docs-cards">
        <Link href="/docs/getting-started/installation" className="docs-card">
          <h4>Install and run &rarr;</h4>
          <p>Clone the repository, start the stack, and open the printed local URL.</p>
        </Link>
        <Link href="/docs/getting-started/quickstart" className="docs-card">
          <h4>First 10 minutes &rarr;</h4>
          <p>Create a company, add an agent, and send the first task.</p>
        </Link>
        <Link href="/docs/features/remote-control" className="docs-card">
          <h4>Use another computer &rarr;</h4>
          <p>Pair a browser without exposing your local Choruz service to the internet.</p>
        </Link>
        <Link href="/docs/agents/drivers" className="docs-card">
          <h4>Choose a driver &rarr;</h4>
          <p>See supported CLIs, login profiles, models, and execution modes.</p>
        </Link>
      </div>

      <h2>The working model</h2>
      <p>A <strong>Company</strong> points at a project folder and contains agents and conversations. An <strong>Agent</strong> runs an installed coding CLI on a selected computer. Direct messages keep a persistent terminal session; group messages wake an agent only when it is mentioned.</p>

      <h2>Go deeper when you need it</h2>
      <p>Use the task guides in the sidebar for normal operation. The <Link href="/docs/api/rest">REST API</Link>, <Link href="/docs/api/websocket">WebSocket API</Link>, <Link href="/docs/operations/env-vars">environment reference</Link>, and repository <a href="https://github.com/jcguo123/Choruz/tree/main/docs">engineering docs</a> hold implementation and operator detail.</p>

      <div className="docs-pager">
        <div />
        <Link href="/docs/getting-started/installation">
          <span className="docs-pager-label">Next</span>
          Install &amp; Run
        </Link>
      </div>
    </>
  );
}
