import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Create an agent</h1>
      <p className="subtitle">Connect an installed coding CLI to a Choruz conversation and workspace.</p>

      <div className="docs-screenshot">
        <img src="/docs-img/create-agent.png" alt="Create Agent modal" />
        <div className="docs-screenshot-caption">The current Create Agent setup step</div>
      </div>

      <h2>Setup</h2>
      <p>Open <strong>Actions → Create Agent</strong>, then configure:</p>
      <ol>
        <li><strong>Start with:</strong> choose a role template or a blank agent.</li>
        <li><strong>Name:</strong> use a role you will recognize in mentions and group chat.</li>
        <li><strong>Driver and account:</strong> select an available CLI and, when enabled, one verified login profile.</li>
        <li><strong>Workspace:</strong> keep the generated workspace or choose a project folder.</li>
        <li><strong>Skills and instructions:</strong> add only context the agent needs on every turn.</li>
      </ol>

      <h2>Review and create</h2>
      <p>Open <strong>Review &amp; Create</strong> and resolve any unavailable driver, account, or template warning. After creation, Choruz opens a direct conversation with the agent.</p>

      <h2>Verify it with a task</h2>
      <p>Ask for a small result that requires reading the workspace, such as <code>Summarize the top-level directories and identify the test command.</code> If the agent does not respond, check <Link href="/docs/troubleshooting/agent-not-responding">Agent Not Responding</Link>.</p>

      <div className="callout callout-tip">
        <strong>Driver details</strong>
        Supported CLIs, account selection, instruction files, and headless execution are documented in <Link href="/docs/agents/drivers">Drivers &amp; Accounts</Link>.
      </div>

      <div className="docs-pager">
        <Link href="/docs/getting-started/first-company">
          <span className="docs-pager-label">Previous</span>
          Your First Company
        </Link>
        <Link href="/docs/concepts/principal-types">
          <span className="docs-pager-label">Next</span>
          Humans &amp; Agents
        </Link>
      </div>
    </>
  );
}
