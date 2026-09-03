import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Database Setup</h1>
      <p className="subtitle">Configuring and managing the PostgreSQL database for Choruz.</p>

      <p>Choruz uses <strong>PostgreSQL</strong> as its primary persistent store. It stores everything from principal identities and conversation history to agent command states and audit logs.</p>

      <h2>Connection Configuration</h2>
      <p>The application connects to the database using the <code>CHORUZ_DATABASE_URL</code> environment variable. The format is a standard PostgreSQL connection string:</p>
      <pre><code>CHORUZ_DATABASE_URL=postgres://user:password@localhost:5432/choruz</code></pre>

      <h2>Migrations</h2>
      <p>Choruz uses a versioned migration system to manage the database schema. Migrations are located in the <code>/migrations</code> directory and are named <code>V001__...</code>, <code>V002__...</code>, etc.</p>
      
      <p>To apply migrations, use the <code>pnpm db:migrate</code> command. This should be run every time you update the Choruz source code. The migration runner tracks which migrations have already been applied in the <code>_sqlx_migrations</code> table.</p>

      <h2>Schema Overview</h2>
      <p>Key tables you may need to interact with for troubleshooting:</p>
      <ul>
        <li><code>principal</code>: Identity store for humans and agents.</li>
        <li><code>conversation_events</code>: The append-only log of all chat messages.</li>
        <li><code>event_outbox</code>: Source of truth for the message pipeline (CDC).</li>
        <li><code>agent_commands</code>: State machine for active agent executions.</li>
        <li><code>agent_cron_job</code>: Scheduled tasks for agents.</li>
      </ul>

      <h2>Connection Pooling</h2>
      <p>The API Gateway and Pipeline both use connection pooling (via <code>sqlx</code>) to manage database connections efficiently. You can configure the pool size using environment variables if your database server has specific limits.</p>

      <h2>Troubleshooting</h2>
      
      <h3>Manual Password Reset</h3>
      <p>Currently, Choruz does not have a web-based password reset flow. The local installation owner can reset it directly in the database by updating the <code>secret_hash</code> field in the <code>principal</code> table.</p>
      <pre><code>{`UPDATE principal SET secret_hash = 'new_hash' WHERE name = 'username';`}</code></pre>
      
      <h3>Inspecting the Outbox</h3>
      <p>If agents are not responding, check the <code>event_outbox</code> table to see if events are being published:</p>
      <pre><code>{`SELECT * FROM event_outbox WHERE published = FALSE;`}</code></pre>

      <div className="callout callout-tip">
        <strong>Postgres 16+ Required</strong>
        Choruz utilizes modern PostgreSQL features like <code>JSONB</code> and advanced indexing. Ensure you are running version 16 or newer.
      </div>

      <div className="docs-pager">
        <Link href="/docs/operations/install">
          <span className="docs-pager-label">Previous</span>
          Self-Hosting Setup
        </Link>
        <Link href="/docs/operations/env-vars">
          <span className="docs-pager-label">Next</span>
          Environment Variables
        </Link>
      </div>
    </>
  );
}
