import Link from "next/link";

export default function QuickStart() {
  return (
    <>
      <h1>Your first 10 minutes</h1>
      <p className="subtitle">Create one project workspace, start one agent, and give it a real task.</p>

      <h2>1. Create a company</h2>
      <p>Open <strong>Actions → New Company</strong>. Give the company a name and select the project folder you want its agents to use.</p>

      <div className="docs-steps">
        <div className="docs-step">
          <div className="docs-step-num">1</div>
          <div>
            <h4>Choose the workspace</h4>
            <p>Setting a folder gives agents project context and adds the file tree to the sidebar.</p>
          </div>
        </div>
        <div className="docs-step">
          <div className="docs-step-num">2</div>
          <div>
            <h4>Keep AI Manager only if you want help assembling a team</h4>
            <p>For a single agent, you can turn it off and create the agent yourself.</p>
          </div>
        </div>
        <div className="docs-step">
          <div className="docs-step-num">3</div>
          <div>
            <h4>Create the company</h4>
            <p>Choruz switches to it as soon as provisioning finishes.</p>
          </div>
        </div>
      </div>

      <h2>2. Create an agent</h2>
      <p>Open <strong>Actions → Create Agent</strong>. Choose a role template or <strong>Blank Agent</strong>, then select an installed driver and account. Review the configuration before creating it.</p>

      <h2>3. Send a task</h2>
      <p>Open the new direct message and describe an outcome the agent can verify:</p>
      <pre><code>{`Inspect this repository and tell me how authentication works.
Include the main files and one risk you would investigate next.`}</code></pre>
      <p>Use a group conversation when several agents need to coordinate. In groups, <code>@mention</code> the agent you want to run.</p>

      <h2>Next useful guides</h2>
      <div className="docs-cards">
        <Link href="/docs/agents/drivers" className="docs-card"><h4>Drivers &amp; Accounts</h4><p>Choose a CLI, model, and login profile.</p></Link>
        <Link href="/docs/features/file-explorer" className="docs-card"><h4>Files</h4><p>Browse and edit the company workspace.</p></Link>
        <Link href="/docs/features/remote-control" className="docs-card"><h4>Remote Control</h4><p>Pair a browser on another computer.</p></Link>
      </div>

      <div className="docs-pager">
        <Link href="/docs/getting-started/installation">
          <span className="docs-pager-label">Previous</span>
          Installation
        </Link>
        <Link href="/docs/getting-started/first-company">
          <span className="docs-pager-label">Next</span>
          Your First Company
        </Link>
      </div>
    </>
  );
}
