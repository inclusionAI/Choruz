import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Cron Scheduler</h1>
      <p className="subtitle">Schedule recurring agent tasks using standard cron expressions or simple interval syntax, managed through the UI, API, or outbox protocol.</p>

      <h2>Overview</h2>
      <p>The Cron Scheduler lets you define recurring tasks for agents. At each scheduled interval, the pipeline sends the configured message to the agent, triggering it to perform the task. This is useful for periodic health checks, report generation, cleanup jobs, and any repeating work.</p>

      <h2>Schedule Formats</h2>

      <h3>Cron Expressions</h3>
      <p>Standard 5-field cron expressions (minute, hour, day-of-month, month, day-of-week):</p>
      <table>
        <thead><tr><th>Expression</th><th>Meaning</th></tr></thead>
        <tbody>
          <tr><td><code>0 10 * * *</code></td><td>Every day at 10:00 AM</td></tr>
          <tr><td><code>*/15 * * * *</code></td><td>Every 15 minutes</td></tr>
          <tr><td><code>0 9 * * 1-5</code></td><td>Weekdays at 9:00 AM</td></tr>
          <tr><td><code>0 0 1 * *</code></td><td>First day of every month at midnight</td></tr>
          <tr><td><code>30 14 * * 3</code></td><td>Every Wednesday at 2:30 PM</td></tr>
        </tbody>
      </table>

      <h3>&quot;Every&quot; Interval Format</h3>
      <p>For simple intervals, use the shorthand format with a number and unit:</p>
      <table>
        <thead><tr><th>Format</th><th>Meaning</th></tr></thead>
        <tbody>
          <tr><td><code>30s</code></td><td>Every 30 seconds</td></tr>
          <tr><td><code>5m</code></td><td>Every 5 minutes</td></tr>
          <tr><td><code>1h</code></td><td>Every 1 hour</td></tr>
          <tr><td><code>2h</code></td><td>Every 2 hours</td></tr>
        </tbody>
      </table>

      <div className="callout callout-info">
        <strong>Minimum interval</strong>
        The pipeline checks for due cron jobs every 30 seconds. Intervals shorter than 30 seconds will effectively run at the 30-second polling rate.
      </div>

      <h2>Creating Cron Jobs</h2>

      <h3>Via the Outbox Protocol</h3>
      <p>Agents can create their own cron jobs with the bound <code>$CHORUZ_SEND</code> helper:</p>
      <pre><code>{`"$CHORUZ_SEND" '{"type":"set_cron",
  "name":"daily-report",
  "schedule":"0 10 * * *",
  "message":"Generate and send the daily status report to the team."
}'`}</code></pre>

      <table>
        <thead><tr><th>Field</th><th>Required</th><th>Description</th></tr></thead>
        <tbody>
          <tr><td><code>type</code></td><td>Yes</td><td>Must be <code>&quot;set_cron&quot;</code></td></tr>
          <tr><td><code>name</code></td><td>Yes</td><td>Human-readable name for the cron job</td></tr>
          <tr><td><code>schedule</code></td><td>Yes</td><td>Cron expression or interval format (e.g., <code>&quot;5m&quot;</code>, <code>&quot;0 10 * * *&quot;</code>)</td></tr>
          <tr><td><code>message</code></td><td>Yes</td><td>The message to send to the agent when triggered</td></tr>
        </tbody>
      </table>

      <h3>Via the API</h3>
      <p>The HTTP API scopes cron jobs by agent in the route. The request body uses separate schedule type and value fields.</p>
      <pre><code>{`POST /v1/agents/{agent_id}/cron
Content-Type: application/json

{
  "name": "health-check",
  "schedule_type": "cron",
  "schedule_value": "*/5 * * * *",
  "conversation_id": "conversation-uuid",
  "message": "Run health checks on all services and report any failures."
}`}</code></pre>

      <h3>Via the UI</h3>
      <p>Create and manage cron jobs from the Detail Panel:</p>
      <ol>
        <li>Click on an agent{"'"}s conversation</li>
        <li>Open the Detail Panel</li>
        <li>Switch to the <strong>Schedule</strong> tab</li>
        <li>Click <strong>Add</strong> to create a new cron job</li>
        <li>Fill in the name, schedule, and message</li>
      </ol>

      <h2>The ? Help Popup</h2>
      <p>In the Schedule tab of the Detail Panel, clicking the <strong>?</strong> icon next to the schedule input shows a quick reference popup explaining cron expression syntax. This helps you write correct schedules without leaving the UI.</p>
      <p>The popup includes:</p>
      <ul>
        <li>Field order explanation (minute, hour, day, month, weekday)</li>
        <li>Common examples</li>
        <li>The &quot;every&quot; interval shorthand format</li>
      </ul>

      <h2>Managing Cron Jobs</h2>

      <h3>List Cron Jobs</h3>
      <pre><code>{`GET /v1/agents/{agent_id}/cron`}</code></pre>

      <h3>Update a Cron Job</h3>
      <pre><code>{`PATCH /v1/agents/{agent_id}/cron/{job_id}
Content-Type: application/json

{
  "schedule_type": "cron",
  "schedule_value": "0 */2 * * *",
  "message": "Updated task message"
}`}</code></pre>

      <h3>Delete a Cron Job</h3>
      <pre><code>{`DELETE /v1/agents/{agent_id}/cron/{job_id}`}</code></pre>

      <h2>How the Scheduler Works</h2>
      <p>The cron scheduler runs as a tokio task inside the choruz-pipeline process:</p>
      <ol>
        <li><strong>Polling</strong> &mdash; Every 30 seconds, the scheduler queries the <code>agent_cron_job</code> table for jobs whose next execution time has passed</li>
        <li><strong>Triggering</strong> &mdash; For each due job, the scheduler inserts a message into the agent{"'"}s conversation as if a user sent it</li>
        <li><strong>Execution</strong> &mdash; The message triggers the agent through the normal pipeline (CDC Poller &rarr; Router &rarr; Executor)</li>
        <li><strong>Next run</strong> &mdash; The scheduler calculates the next execution time based on the cron expression and updates the record</li>
      </ol>

      <h2>Database Table</h2>
      <table>
        <thead><tr><th>Column</th><th>Type</th><th>Description</th></tr></thead>
        <tbody>
          <tr><td><code>id</code></td><td>TEXT</td><td>Unique job identifier</td></tr>
          <tr><td><code>agent_id</code></td><td>TEXT</td><td>The agent this job belongs to</td></tr>
          <tr><td><code>conversation_id</code></td><td>TEXT</td><td>The conversation where the scheduled message is dispatched</td></tr>
          <tr><td><code>name</code></td><td>TEXT</td><td>Human-readable job name</td></tr>
          <tr><td><code>schedule_type</code></td><td>TEXT</td><td><code>at</code>, <code>every</code>, or <code>cron</code></td></tr>
          <tr><td><code>schedule_value</code></td><td>TEXT</td><td>ISO timestamp, interval, or cron expression</td></tr>
          <tr><td><code>schedule_timezone</code></td><td>TEXT</td><td>Optional timezone metadata</td></tr>
          <tr><td><code>message</code></td><td>TEXT</td><td>Message sent to agent on trigger</td></tr>
          <tr><td><code>session_target</code></td><td>TEXT</td><td><code>main</code> or <code>isolated</code></td></tr>
          <tr><td><code>delivery_mode</code></td><td>TEXT</td><td><code>announce</code> or <code>none</code></td></tr>
          <tr><td><code>next_run_at</code></td><td>TIMESTAMPTZ</td><td>When the job will next execute</td></tr>
          <tr><td><code>last_run_at</code></td><td>TIMESTAMPTZ</td><td>When the job last executed (nullable)</td></tr>
          <tr><td><code>enabled</code></td><td>BOOLEAN</td><td>Whether the job is active</td></tr>
        </tbody>
      </table>

      <div className="docs-pager">
        <Link href="/docs/features/search">
          <span className="docs-pager-label">Previous</span>
          Search
        </Link>
        <Link href="/docs/features/pixel-world">
          <span className="docs-pager-label">Next</span>
          Pixel World
        </Link>
      </div>
    </>
  );
}
