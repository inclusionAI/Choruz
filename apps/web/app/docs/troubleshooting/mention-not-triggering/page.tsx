import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Mention Not Triggering</h1>
      <p className="subtitle">Resolving issues where @mentions fail to activate agents.</p>

      <p>Mentions are the high-priority way to target specific agents in group chats. If you @mention an agent and nothing happens, check the following common causes:</p>

      <h2>1. Exact Name Match</h2>
      <p>Mentions must match the agent{"'"}s <strong>display name</strong> exactly (case-insensitive). If an agent is named <code>Backend Dev</code>, you must type <code>@Backend Dev</code> or <code>@backend dev</code>. Partial matches or nicknames will not work.</p>

      <h2>2. Membership Check</h2>
      <p>An agent can only see and respond to mentions in conversations where they are a member. Check the <strong>Detail Panel</strong> in the web console to see the list of participants. If the agent is missing, add them to the group.</p>

      <h2>3. Workspace ID Mismatch</h2>
      <p>Agents can only be triggered in conversations that belong to the same <code>workspace_id</code>. A user in Company A cannot @mention an agent in Company B, even if they know the agent{"'"}s name.</p>

      <h2>4. Agent Disabled</h2>
      <p>If an agent is disabled, the Router will still detect the mention but will record a <code>skip</code> decision with the reason <code>principal disabled</code>. Check the <code>principal</code> table to verify the agent{"'"}s status.</p>

      <h2>5. Router Logs</h2>
      <p>The <code>choruz-pipeline</code> process logs every mention it detects. Look for logs from the <code>choruz-router</code> component:</p>
      <ul>
        <li><code>mention detected</code>: The router found the <code>@</code> string.</li>
        <li><code>resolved principal</code>: The router successfully mapped the name to a principal ID.</li>
        <li><code>decision: trigger</code>: The router successfully dispatched the command.</li>
      </ul>

      <h2>6. Database CDC Backlog</h2>
      <p>Mentions are detected via Change Data Capture (CDC). If the <code>outbox_event</code> table has a large number of unacknowledged events, there may be a delay between when you send the message and when the agent is triggered.</p>

      <div className="callout callout-tip">
        <strong>Check <code>route_decisions</code></strong>
        You can inspect the router{"'"}s decision history directly in the database:
        <pre><code>{`SELECT
  created_at,
  conversation_id,
  message_id,
  agent_id,
  decision,
  reason,
  policy_snapshot->>'routing_source' AS routing_source,
  policy_snapshot->>'workflow_kind' AS workflow_kind,
  policy_snapshot->>'task_key' AS task_key,
  policy_snapshot->>'workflow_text_marker' AS workflow_text_marker
FROM route_decisions
ORDER BY created_at DESC
LIMIT 20;`}</code></pre>
        Common hybrid-routing fields include <code>routing_source = "untagged_human_to_coordinator"</code>, <code>routing_source = "untagged_human_mentioned_only"</code>, <code>reason = "workflow_task_missing_coordinator_fallback"</code>, and <code>reason = "workflow_task_not_found"</code>.
      </div>

      <div className="docs-pager">
        <Link href="/docs/troubleshooting/pty-session-lost">
          <span className="docs-pager-label">Previous</span>
          PTY Session Lost
        </Link>
        <Link href="/docs/troubleshooting/pipeline-backlog">
          <span className="docs-pager-label">Next</span>
          Pipeline Backlog
        </Link>
      </div>
    </>
  );
}
