import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Common Error Codes</h1>
      <p className="subtitle">A quick reference for HTTP status codes and API error messages.</p>

      <table>
        <thead>
          <tr>
            <td>Status</td>
            <td>Error Message</td>
            <td>Meaning / Fix</td>
          </tr>
        </thead>
        <tbody>
          <tr>
            <td><strong>401</strong></td>
            <td><code>invalid local credentials</code></td>
            <td>Incorrect username or password. Check <code>CHORUZ_OPERATOR_USER</code> and <code>CHORUZ_OPERATOR_PASSWORD</code> for the local account.</td>
          </tr>
          <tr>
            <td><strong>401</strong></td>
            <td><code>missing credentials</code></td>
            <td>The <code>Authorization</code> header or <code>choruz_session</code> cookie is missing.</td>
          </tr>
          <tr>
            <td><strong>403</strong></td>
            <td><code>cross-workspace access denied</code></td>
            <td>You are trying to access a resource (conversation, agent, company) that belongs to a different workspace than your session.</td>
          </tr>
          <tr>
            <td><strong>403</strong></td>
            <td><code>principal disabled</code></td>
            <td>The actor (human or agent) has been disabled.</td>
          </tr>
          <tr>
            <td><strong>409</strong></td>
            <td><code>username &apos;...&apos; is reserved</code></td>
            <td>You are trying to sign up with a name that matches the system <code>operator</code>.</td>
          </tr>
          <tr>
            <td><strong>409</strong></td>
            <td><code>duplicate client_msg_id</code></td>
            <td>A message with this UUID has already been received. This is normal behavior for client-side retries.</td>
          </tr>
          <tr>
            <td><strong>500</strong></td>
            <td><code>db error</code></td>
            <td>The gateway failed to communicate with PostgreSQL. Check your <code>CHORUZ_DATABASE_URL</code> and database logs.</td>
          </tr>
          <tr>
            <td><strong>503</strong></td>
            <td><code>no active runners</code></td>
            <td>The pipeline is running but no agents are currently connected to pick up tasks.</td>
          </tr>
        </tbody>
      </table>

      <h2>Debugging with Logs</h2>
      <p>If you encounter an error not listed here, check the server logs for a <strong>Trace ID</strong>. Choruz uses a distributed tracing system (via the <code>choruz-trace</code> crate) that assigns a unique UUID to every request. Searching for this ID in your logs will show the full stack trace and database queries associated with the failure.</p>

      <h2>Network Issues</h2>
      <p>If you are running Choruz behind a reverse proxy (Caddy/Nginx) and seeing <code>502 Bad Gateway</code> or <code>504 Gateway Timeout</code> errors:</p>
      <ul>
        <li>Ensure the upstream service (Gateway or Web App) is actually running.</li>
        <li>Check the proxy logs (e.g., <code>journalctl -u caddy</code>).</li>
        <li>Increase the timeout limits in your proxy configuration for long-running agent executions.</li>
      </ul>

      <div className="docs-pager">
        <Link href="/docs/troubleshooting/pipeline-backlog">
          <span className="docs-pager-label">Previous</span>
          Pipeline Backlog
        </Link>
        <Link href="/docs/reference/changelog">
          <span className="docs-pager-label">Next</span>
          Changelog
        </Link>
      </div>
    </>
  );
}
