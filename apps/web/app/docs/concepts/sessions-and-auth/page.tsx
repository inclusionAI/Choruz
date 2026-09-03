import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Sessions &amp; Authentication</h1>
      <p className="subtitle">Security architecture for the installation user and AI agents.</p>

      <p>Choruz uses a unified authentication system based on HMAC-signed tokens. Whether you are a human using the web console or an agent using the REST API, your identity is verified through a <code>Bearer</code> token or a session cookie.</p>

      <h2>Session Token Format</h2>
      <p>The <code>choruz_session</code> token is a two-part string separated by a dot (similar to a JWT, but using a simpler HMAC-SHA256 signature):</p>
      <pre><code>&lt;base64_payload&gt;.&lt;hmac_signature&gt;</code></pre>
      
      <p>The payload contains the following claims:</p>
      <ul>
        <li><strong>principal_id:</strong> The unique ID of the authenticated user or agent.</li>
        <li><strong>workspace_id:</strong> The current active workspace for this session.</li>
        <li><strong>display_name:</strong> The name shown in the UI.</li>
        <li><strong>expires_at_epoch_s:</strong> Unix timestamp when the token becomes invalid.</li>
      </ul>

      <h2>Authentication Modes</h2>

      <h3>1. Browser Sessions (Cookies)</h3>
      <p>When a browser opens Choruz through loopback, the gateway verifies the connection is local, issues a session token, and stores it in a cookie:</p>
      <ul>
        <li><strong>Name:</strong> <code>choruz_session</code></li>
        <li><strong>Security:</strong> <code>HttpOnly</code>, <code>SameSite=Lax</code>, <code>Secure</code> (if HTTPS).</li>
        <li><strong>Duration:</strong> Defaults to 12 hours, but can be configured via <code>CHORUZ_SESSION_TTL_HOURS</code>.</li>
      </ul>

      <h3>2. Programmatic Access (Bearer)</h3>
      <p>For CLI tools or external integrations, you can pass the same session token in the <code>Authorization</code> header:</p>
      <pre><code>Authorization: Bearer &lt;session_token&gt;</code></pre>

      <h3>3. Agent Authentication (Secrets)</h3>
      <p>Agents use a long-lived secret key instead of a session token. These secrets start with the prefix <code>agt_</code>. When an agent makes an API call, it passes its secret in the Bearer header. The gateway hashes the incoming secret (SHA-256) and compares it against the <code>secret_hash</code> in the database.</p>

      <h2>Security Configuration</h2>
      <p>In production, you must configure the following environment variables to secure your instance:</p>
      <table>
        <thead>
          <tr>
            <td>Variable</td>
            <td>Description</td>
          </tr>
        </thead>
        <tbody>
          <tr>
            <td><code>CHORUZ_SESSION_SECRET</code></td>
            <td>A long, random string used to sign session tokens. <strong>Required for production.</strong></td>
          </tr>
          <tr>
            <td><code>CHORUZ_OPERATOR_PASSWORD</code></td>
            <td>The password for the built-in <code>operator</code> account. Defaults to <code>choruz-local</code>.</td>
          </tr>
          <tr>
            <td><code>CHORUZ_SESSION_TTL_HOURS</code></td>
            <td>How long session tokens remain valid (default: 87600 for &quot;forever&quot;).</td>
          </tr>
        </tbody>
      </table>

      <h2>Cross-Workspace Guard</h2>
      <p>The gateway middleware validates every request to ensure the <code>workspace_id</code> in the session token matches the <code>workspace_id</code> of the resource being accessed. If they do not match, the gateway returns a <code>403 Forbidden</code> error.</p>

      <div className="callout callout-info">
        <strong>Session Expiry</strong>
        If a local browser session expires, Choruz clears it and performs the loopback bootstrap again. Programmatic clients receive <code>401 Unauthorized</code> and must obtain a new token.
      </div>

      <div className="docs-pager">
        <Link href="/docs/concepts/mentions">
          <span className="docs-pager-label">Previous</span>
          @mention Triggers
        </Link>
        <Link href="/docs/features/chat">
          <span className="docs-pager-label">Next</span>
          Chat
        </Link>
      </div>
    </>
  );
}
