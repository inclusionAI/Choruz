import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Environment Variables</h1>
      <p className="subtitle">Full reference for configuring the Choruz API Gateway, Pipeline, and Web App.</p>

      <p>Choruz is configured entirely through environment variables. These can be set in your shell, via a <code>.env</code> file, or within your systemd service definitions.</p>

      <h2>Core Configuration</h2>
      <table>
        <thead>
          <tr>
            <td>Variable</td>
            <td>Default</td>
            <td>Description</td>
          </tr>
        </thead>
        <tbody>
          <tr>
            <td><code>CHORUZ_DATABASE_URL</code></td>
            <td>(Required)</td>
            <td>PostgreSQL connection string. Example: <code>postgres://localhost/choruz</code></td>
          </tr>
          <tr>
            <td><code>CHORUZ_ENV</code></td>
            <td><code>development</code></td>
            <td>Set to <code>production</code> to enforce strict secret validation.</td>
          </tr>
          <tr>
            <td><code>CHORUZ_SESSION_SECRET</code></td>
            <td><code>choruz-local...</code></td>
            <td>HMAC secret for signing session tokens. <strong>Must be changed in production.</strong></td>
          </tr>
          <tr>
            <td><code>CHORUZ_PLUGINS</code></td>
            <td>(unset: all built-ins)</td>
            <td>Comma-separated built-in plugin allowlist. Unset enables all built-ins; an empty value disables all plugins.</td>
          </tr>
        </tbody>
      </table>

      <h2>Administrative Auth</h2>
      <table>
        <thead>
          <tr>
            <td>Variable</td>
            <td>Default</td>
            <td>Description</td>
          </tr>
        </thead>
        <tbody>
          <tr>
            <td><code>CHORUZ_OPERATOR_USER</code></td>
            <td><code>operator</code></td>
            <td>The display name for the local installation user.</td>
          </tr>
          <tr>
            <td><code>CHORUZ_OPERATOR_PASSWORD</code></td>
            <td><code>choruz-local</code></td>
            <td>The password for the local installation user. <strong>Must be changed in production.</strong></td>
          </tr>
          <tr>
            <td><code>CHORUZ_OPERATOR_WORKSPACE</code></td>
            <td><code>ws-local</code></td>
            <td>The workspace ID assigned to the local installation user.</td>
          </tr>
          <tr>
            <td><code>CHORUZ_SESSION_TTL_HOURS</code></td>
            <td><code>87600</code></td>
            <td>How long session tokens remain valid (hours). Default is ~10 years.</td>
          </tr>
        </tbody>
      </table>

      <h2>Network &amp; Ports</h2>
      <table>
        <thead>
          <tr>
            <td>Variable</td>
            <td>Default</td>
            <td>Description</td>
          </tr>
        </thead>
        <tbody>
          <tr>
            <td><code>CHORUZ_API_HOST</code></td>
            <td><code>127.0.0.1</code></td>
            <td>The bind address for the API Gateway. Use <code>0.0.0.0</code> for LAN access.</td>
          </tr>
          <tr>
            <td><code>CHORUZ_API_PORT</code></td>
            <td><code>3000</code></td>
            <td>The HTTP port for the API Gateway.</td>
          </tr>
          <tr>
            <td><code>CHORUZ_PIPELINE_METRICS_HOST</code></td>
            <td><code>127.0.0.1</code></td>
            <td>The bind address for the Pipeline WebSocket server.</td>
          </tr>
          <tr>
            <td><code>CHORUZ_PIPELINE_METRICS_PORT</code></td>
            <td><code>3020</code></td>
            <td>The port for the Pipeline WebSocket server.</td>
          </tr>
        </tbody>
      </table>

      <h2>Filesystem &amp; Drivers</h2>
      <table>
        <thead>
          <tr>
            <td>Variable</td>
            <td>Default</td>
            <td>Description</td>
          </tr>
        </thead>
        <tbody>
          <tr>
            <td><code>CHORUZ_ATTACHMENT_DIR</code></td>
            <td><code>.runtime/attachments</code></td>
            <td>Directory where shared files and binary attachments are stored.</td>
          </tr>
          <tr>
            <td><code>CHORUZ_AGENT_TOKENS_FILE</code></td>
            <td><code>.runtime/agent_tokens.json</code></td>
            <td>Path to the JSON file where agent Bearer secrets are cached.</td>
          </tr>
          <tr>
            <td><code>CHORUZ_CLAUDE_BINARY</code></td>
            <td><code>claude</code></td>
            <td>Path to the Claude Code CLI binary.</td>
          </tr>
          <tr>
            <td><code>CHORUZ_CODEX_BINARY</code></td>
            <td><code>codex</code></td>
            <td>Path to the Codex CLI binary.</td>
          </tr>
          <tr><td><code>CHORUZ_PI_BINARY</code></td><td><code>pi</code></td><td>Path to the Pi Agent CLI binary.</td></tr>
          <tr><td><code>CHORUZ_GROK_BINARY</code></td><td><code>grok</code></td><td>Path to the Grok Build CLI binary.</td></tr>
          <tr><td><code>CHORUZ_OPENCODE_BINARY</code></td><td><code>opencode</code></td><td>Path to the OpenCode CLI binary.</td></tr>
          <tr><td><code>CHORUZ_MATHCODE_BINARY</code></td><td><code>mathcode</code></td><td>Path to the MathCode CLI binary when the <code>mathcode</code> plugin is enabled.</td></tr>
          <tr><td><code>CHORUZ_HARNESS_ACCOUNT_ROOT</code></td><td><code>~/.choruz/accounts</code></td><td>Device-local root for isolated Claude Code and Codex login profiles. Keep it private and writable only by the runtime user.</td></tr>
        </tbody>
      </table>

      <div className="callout callout-info">
        <strong>Runtime Directory</strong>
        Many defaults point to the <code>.runtime</code> directory in the project root. Ensure this directory exists and is writable by the user running the Choruz processes.
      </div>

      <div className="docs-pager">
        <Link href="/docs/operations/database">
          <span className="docs-pager-label">Previous</span>
          Database Configuration
        </Link>
        <Link href="/docs/operations/deployment">
          <span className="docs-pager-label">Next</span>
          Production Deployment
        </Link>
      </div>
    </>
  );
}
