import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Create a company</h1>
      <p className="subtitle">A company groups one project folder, its agents, and its conversations.</p>

      <div className="docs-screenshot">
        <img src="/docs-img/new-company.png" alt="New Company modal" />
        <div className="docs-screenshot-caption">The current New Company form</div>
      </div>

      <h2>Open the form</h2>
      <p>From the dashboard, open <strong>Actions → New Company</strong>.</p>

      <h2>Choose the settings</h2>
      <table>
        <thead>
          <tr>
            <td>Field</td>
            <td>Description</td>
          </tr>
        </thead>
        <tbody>
          <tr>
            <td><strong>Name</strong></td>
            <td>The project or team name shown in the sidebar.</td>
          </tr>
          <tr>
            <td><strong>Folder Path</strong></td>
            <td>The project directory on this computer. When set, its file tree appears in the sidebar and is offered as the agent workspace.</td>
          </tr>
          <tr>
            <td><strong>Include AI Manager</strong></td>
            <td>Creates a coordinator that can design a team and provision more agents. Turn it off when you only need a manually configured agent.</td>
          </tr>
        </tbody>
      </table>

      <h2>After creation</h2>
      <p>Choruz switches to the new company. If AI Manager was enabled, open its direct message and describe the team you need. Otherwise use <strong>Actions → Create Agent</strong>.</p>

      <div className="callout callout-info">
        <strong>Local path</strong>
        The workspace folder belongs to the computer running Choruz. A remotely paired browser controls that computer; it does not copy the folder to the browser device.
      </div>

      <div className="docs-pager">
        <Link href="/docs/getting-started/quickstart">
          <span className="docs-pager-label">Previous</span>
          Quick Start
        </Link>
        <Link href="/docs/getting-started/first-agent">
          <span className="docs-pager-label">Next</span>
          Your First Agent
        </Link>
      </div>
    </>
  );
}
