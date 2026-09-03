import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>PTY Session Lost</h1>
      <p className="subtitle">What to do when an agent{"'"}s terminal session disconnects or fails to resume.</p>

      <p>Choruz uses <strong>PTY (Pseudo-Terminal)</strong> sessions to run AI CLIs. These sessions are persistent, allowing you to watch the agent work in real-time. However, network issues or server restarts can sometimes cause these sessions to be lost.</p>

      <h2>Common Symptoms</h2>
      <ul>
        <li>The terminal view in the Web Console shows a &quot;Connection Lost&quot; message.</li>
        <li>The agent stops responding even though the pipeline is running.</li>
        <li>Opening a direct chat shows a blank terminal or an initialization error.</li>
      </ul>

      <h2>Why Sessions are Lost</h2>
      <p>Choruz manages a <code>PtyPool</code> in the API Gateway. Sessions can be lost if:</p>
      <ul>
        <li><strong>Stale Session Eviction:</strong> The gateway periodically evicts sessions that haven{"'"}t seen activity for a long time.</li>
        <li><strong>Gateway Restart:</strong> Since the <code>PtyPool</code> is in-memory, restarting the <code>choruz-api-gateway</code> process terminates all active PTY sessions.</li>
        <li><strong>Binary Crashes:</strong> The underlying CLI (e.g., <code>claude</code>) might crash due to an unhandled error or resource exhaustion.</li>
      </ul>

      <h2>Resuming a Session</h2>
      <p>When you reconnect to a terminal, the client sends a <code>resume_session_id</code>. The gateway attempts to find the existing session in the pool. If found, it attaches the WebSocket to the existing PTY. If not, it starts a new session.</p>
      
      <div className="callout callout-warn">
        <strong>Resuming Claude Code</strong>
        If a Claude Code session is lost and <code>--resume</code> fails, it usually means the local state in <code>~/.claude/projects/</code> was cleared or the session ID expired. In this case, the agent will start a fresh session, and you may need to provide context again.
      </div>

      <h2>Troubleshooting Steps</h2>
      <ol>
        <li><strong>Refresh the Browser:</strong> This triggers a new WebSocket connection and attempts to resume the session.</li>
        <li><strong>Check Process List:</strong> On the server, check if the CLI processes are still running:
          <pre><code>{`ps aux | grep -E 'claude|codex|gemini'`}</code></pre>
        </li>
        <li><strong>Check Workspace Permissions:</strong> Ensure the API Gateway process has permission to read/write to the agent{"'"}s <code>.choruz-runtime/workspaces/</code> folder.</li>
        <li><strong>Inspect <code>handlers_terminals.rs</code>:</strong> If you are a developer, look for &quot;stale session&quot; logs in the gateway output to see if eviction is triggering too frequently.</li>
      </ol>

      <div className="docs-pager">
        <Link href="/docs/troubleshooting/agent-not-responding">
          <span className="docs-pager-label">Previous</span>
          Agent Not Responding
        </Link>
        <Link href="/docs/troubleshooting/mention-not-triggering">
          <span className="docs-pager-label">Next</span>
          @mention Not Triggering
        </Link>
      </div>
    </>
  );
}
