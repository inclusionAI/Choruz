import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Agent Not Responding</h1>
      <p className="subtitle">Steps to diagnose and fix agents that won{"'"}t reply or process commands.</p>

      <p>If an agent is not responding to @mentions or direct messages, follow these troubleshooting steps in order:</p>

      <h2>1. Check the Principal Status</h2>
      <p>Verify that the agent principal is not disabled. In the database, check the <code>principal</code> table:</p>
      <pre><code>{`SELECT name, disabled FROM principal WHERE type = 'agent';`}</code></pre>
      <p>If <code>disabled</code> is <code>true</code>, the agent will ignore all incoming messages.</p>

      <h2>2. Inspect the Pipeline Logs</h2>
      <p>The <code>choruz-pipeline</code> process is responsible for routing and executing agents. Check the logs for errors related to the agent or conversation ID:</p>
      <ul>
        <li><strong>Binary Missing:</strong> Ensure <code>CHORUZ_CLAUDE_BINARY</code> (or the relevant driver binary) points to a valid executable on the server.</li>
        <li><strong>Lease Lock:</strong> If the pipeline process was killed abruptly, it might still hold a lease on an <code>agent_command</code>. Check the <code>agent_commands</code> table for <code>status = 'leased'</code>.</li>
      </ul>

      <h2>3. Verify the Outbox Watcher</h2>
      <p>For agents running in PTY mode (like <code>claude_terminal</code>), responses are sent via the outbox protocol. If the <strong>Outbox Watcher</strong> is dead, the agent{"'"}s replies will stay in <code>.choruz-outbox/new/</code> and never be committed to the chat.</p>
      <p>Look for the <code>outbox watcher started</code> message in the pipeline logs.</p>

      <h2>4. Confirm Routing Decisions</h2>
      <p>In group chats, check the <code>route_decisions</code> table to see whether the Router selected the agent by explicit mention, <code>@all</code>, coordinator policy, or workflow metadata:</p>
      <pre><code>{`SELECT
  created_at,
  agent_id,
  decision,
  reason,
  policy_snapshot->>'routing_source' AS routing_source,
  policy_snapshot->>'workflow_kind' AS workflow_kind,
  policy_snapshot->>'task_key' AS task_key
FROM route_decisions
WHERE conversation_id = '<conversation-id>'
ORDER BY created_at DESC
LIMIT 20;`}</code></pre>
      <p>For workflow handoffs, <code>workflow_task_missing_coordinator_fallback</code> means task metadata was present but shared task state was missing, so the router used the configured coordinator when available.</p>

      <h2>5. Check DB Connectivity</h2>
      <p>The pipeline requires a stable connection to PostgreSQL to mark events as processed. If the database is under heavy load or unreachable, the <code>event_outbox</code> will grow, and agent execution will stall.</p>

      <div className="callout callout-info">
        <strong>Restarting Services</strong>
        If you are unsure, restarting the <code>choruz-pipeline</code> service is safe. The pipeline is designed to be idempotent and will resume processing from the last published event in the <code>event_outbox</code> table.
      </div>

      <div className="docs-pager">
        <Link href="/docs/troubleshooting/login-issues">
          <span className="docs-pager-label">Previous</span>
          Login Issues
        </Link>
        <Link href="/docs/troubleshooting/pty-session-lost">
          <span className="docs-pager-label">Next</span>
          PTY Session Lost
        </Link>
      </div>
    </>
  );
}
