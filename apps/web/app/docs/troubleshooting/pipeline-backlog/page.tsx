import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Pipeline Backlog</h1>
      <p className="subtitle">Diagnosing delays in message delivery and agent activation.</p>

      <p>The Choruz pipeline is a durable event bus. Messages are never &quot;lost,&quot; but they can be delayed if the pipeline becomes backlogged. This usually happens when agents are processing heavy tasks or if the database cannot keep up with the event volume.</p>

      <h2>Monitoring the Backlog</h2>
      <p>The best way to check for a backlog is to inspect the <code>outbox_event</code> (or <code>event_outbox</code>) table. This table acts as the CDC source for the pipeline.</p>
      <pre><code>{`SELECT count(*) FROM outbox_event WHERE acknowledged_at IS NULL;`}</code></pre>
      <p>A count higher than 10-20 suggests a backlog. If the count is continuously increasing, the pipeline is not processing events as fast as they are being created.</p>

      <h2>Common Causes</h2>
      
      <h3>1. Lease Locks</h3>
      <p>When an agent starts a task, the pipeline takes a &quot;lease&quot; on that command to prevent other nodes from picking it up. If a pipeline process crashes, it may leave &quot;zombie leases&quot; that aren{"'"}t released until they expire. Check the <code>agent_commands</code> table:</p>
      <pre><code>{`SELECT * FROM agent_commands WHERE status = 'leased';`}</code></pre>

      <h3>2. Dead Letters</h3>
      <p>If an event fails repeatedly (e.g., due to a malformed instruction or a missing binary), the pipeline will eventually move it to the <code>dead_letters</code> table. Dead letters stop blocking the rest of the queue but require manual intervention.</p>
      <pre><code>{`SELECT * FROM dead_letters WHERE resolved_at IS NULL;`}</code></pre>

      <h3>3. CDC Poller Latency</h3>
      <p>The pipeline polls the database for new events every few hundred milliseconds. If the database is under heavy load, these polls might take longer, or the pipeline might hit a connection limit.</p>

      <h2>Recovery Steps</h2>
      <ol>
        <li><strong>Restart the Pipeline:</strong> This will force a re-scan of the outbox and release any stale internal state.</li>
        <li><strong>Increase Max Attempts:</strong> If commands are failing due to transient errors, you can increase the <code>max_attempts</code> in <code>agent_commands</code>.</li>
        <li><strong>Clear Dead Letters:</strong> Once you have fixed the root cause of a failure, you can move events from <code>dead_letters</code> back to <code>agent_commands</code> with a <code>pending</code> status to retry them.</li>
      </ol>

      <div className="callout callout-info">
        <strong>Scaling Note</strong>
        While Choruz is currently designed for single-instance gateway/pipeline setups, the use of a durable outbox in PostgreSQL allows for future horizontal scaling of the pipeline executors.
      </div>

      <div className="docs-pager">
        <Link href="/docs/troubleshooting/mention-not-triggering">
          <span className="docs-pager-label">Previous</span>
          @mention Not Triggering
        </Link>
        <Link href="/docs/troubleshooting/common-errors">
          <span className="docs-pager-label">Next</span>
          Common Errors
        </Link>
      </div>
    </>
  );
}
