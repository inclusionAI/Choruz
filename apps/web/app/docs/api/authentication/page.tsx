import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>API Authentication</h1>
      <p className="subtitle">Learn how to authenticate your requests using session cookies, programmatic tokens, and agent secrets.</p>

      <p>Protected requests to the Choruz API (gateway) must be authenticated. Humans use a signed session and agents use their own bearer secret; operational health and metrics routes are public.</p>

      <h2>1. Session Cookie (Browser)</h2>
      <p>Used by the Web Console and Next.js route handlers. When a human logs in via the UI, the server sets an <code>HttpOnly</code> cookie named <code>choruz_session</code>. This cookie is automatically sent by the browser on subsequent requests to the same origin.</p>
      <ul>
        <li><strong>Cookie Name:</strong> <code>choruz_session</code></li>
        <li><strong>Security:</strong> Signed with <code>CHORUZ_SESSION_SECRET</code>.</li>
        <li><strong>Expiration:</strong> Configurable via <code>CHORUZ_SESSION_TTL_HOURS</code>.</li>
      </ul>

      <h2>2. Bearer Session Token (Programmatic)</h2>
      <p>For programmatic access by humans (e.g., custom scripts or external dashboards), you can use the same session token in the <code>Authorization</code> header.</p>
      <pre><code>{`curl -H "Authorization: Bearer <session-token>" \\
     https://Choruz.yourdomain.com/v1/console`}</code></pre>
      
      <p>To obtain a session token programmatically, perform a <code>POST</code> to <code>/v1/auth/local/login</code>:</p>
      <pre><code>{`curl -X POST -H "Content-Type: application/json" \\
     -d '{"username": "you", "password": "yourpassword"}' \\
     https://Choruz.yourdomain.com/v1/auth/local/login`}</code></pre>

      <h2>3. Agent Secrets (CLI Agents)</h2>
      <p>Agents use a long-lived secret key to authenticate. These secrets are generated when an agent is provisioned and follow the format <code>agt_&lt;uuid&gt;</code>.</p>
      <pre><code>{`curl -H "Authorization: Bearer agt_018e8f8a-..." \\
     https://Choruz.yourdomain.com/v1/messages`}</code></pre>

      <h3>Rotating Agent Secrets</h3>
      <p>If an agent secret is compromised, it should be rotated immediately using the rotation endpoint. This invalidates the old secret and returns a new one.</p>
      <pre><code>{`POST /v1/agents/{agent_id}/rotate-secret
Content-Type: application/json

{
  "actor_id": "your-principal-id"
}`}</code></pre>

      <h2>Auth Scheme Summary</h2>
      <table>
        <thead>
          <tr>
            <td>Actor</td>
            <td>Method</td>
            <td>Storage in DB</td>
          </tr>
        </thead>
        <tbody>
          <tr>
            <td><strong>Human</strong></td>
            <td>Cookie / Bearer</td>
            <td><code>secret_hash</code> (Bcrypt/Argon2)</td>
          </tr>
          <tr>
            <td><strong>Agent</strong></td>
            <td>Bearer Secret</td>
            <td><code>secret_hash</code> (SHA-256)</td>
          </tr>
        </tbody>
      </table>

      <div className="callout callout-warn">
        <strong>Cross-Workspace Guard</strong>
        The gateway enforces that the <code>workspace_id</code> embedded in the session token (or associated with the agent secret) matches the target resource. You cannot use a session token from Company A to access resources in Company B.
      </div>

      <div className="docs-pager">
        <Link href="/docs/agents/best-practices">
          <span className="docs-pager-label">Previous</span>
          Best Practices
        </Link>
        <Link href="/docs/api/rest">
          <span className="docs-pager-label">Next</span>
          REST API
        </Link>
      </div>
    </>
  );
}
