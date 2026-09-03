import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>REST Endpoints</h1>
      <p className="subtitle">Gateway HTTP endpoint inventory for local development and API integration work.</p>

      <div className="callout callout-info">
        <strong>Base URL</strong>
        All endpoints below are served from the API Gateway at <code>http://localhost:3000</code> by default. Most endpoints require a session or bearer token.
      </div>

      <div className="callout callout-info">
        <strong>Route classes</strong>
        This inventory includes product integration routes plus local console and operator surfaces. Descriptions marked <strong>Human-only/internal</strong> are backed by <code>require_human_operator</code>. Descriptions marked <strong>Operational/internal</strong> support local health, metrics, console, or terminal-control behavior.
      </div>

      <div className="callout callout-warn">
        <strong>Realtime APIs</strong>
        Use authenticated <code>/v1/ws/sync</code> for acknowledged dashboard changes and <code>{"/v1/ws/terminals/{binding_id}"}</code> for PTY sessions. The old unauthenticated principal and fanout sockets are retired.
      </div>

      <h2>System And Auth</h2>
      <table>
        <thead><tr><th>Method</th><th>Path</th><th>Description</th></tr></thead>
        <tbody>
          <tr><td>GET</td><td><code>/healthz</code></td><td>Operational/internal process liveness check</td></tr>
          <tr><td>GET</td><td><code>/readyz</code></td><td>Operational/internal dependency readiness check</td></tr>
          <tr><td>GET</td><td><code>/metrics</code></td><td>Operational/internal Prometheus metrics</td></tr>
          <tr><td>GET</td><td><code>/v1/status</code></td><td>Operational/internal product phase status</td></tr>
          <tr><td>POST</td><td><code>/v1/auth/local/login</code></td><td>Create local session</td></tr>
          <tr><td>POST</td><td><code>/v1/auth/local/signup</code></td><td>Create local account</td></tr>
          <tr><td>GET</td><td><code>/v1/me</code></td><td>Return current principal</td></tr>
          <tr><td>GET</td><td><code>/v1/bootstrap</code></td><td>Bounded dashboard bootstrap with cursor pagination</td></tr>
          <tr><td>GET</td><td><code>/v1/sync</code></td><td>Durable dashboard change replay after a cursor</td></tr>
          <tr><td>WS</td><td><code>/v1/ws/sync</code></td><td>Authenticated replay, live changes, and per-device ACKs</td></tr>
          <tr><td>GET</td><td><code>/v1/console</code></td><td>Operational/internal console snapshot</td></tr>
        </tbody>
      </table>

      <h2>Principals And Agents</h2>
      <table>
        <thead><tr><th>Method</th><th>Path</th><th>Description</th></tr></thead>
        <tbody>
          <tr><td>POST</td><td><code>{"/v1/principals/{principal_id}/disable"}</code></td><td>Soft-disable a principal</td></tr>
          <tr><td>PATCH</td><td><code>{"/v1/principals/{principal_id}/workspace"}</code></td><td>Human-only/internal: move principal workspace</td></tr>
          <tr><td>POST</td><td><code>/v1/agents</code></td><td>Create an agent</td></tr>
          <tr><td>POST</td><td><code>/v1/agents/batch-disable</code></td><td>Human-only/internal: disable multiple agents</td></tr>
          <tr><td>POST</td><td><code>{"/v1/agents/{agent_id}/rotate-secret"}</code></td><td>Rotate agent secret</td></tr>
          <tr><td>GET</td><td><code>{"/v1/agents/{agent_id}/tasks"}</code></td><td>Human-only/internal: list agent tasks</td></tr>
        </tbody>
      </table>

      <h2>Runtime And Cron</h2>
      <table>
        <thead><tr><th>Method</th><th>Path</th><th>Description</th></tr></thead>
        <tbody>
          <tr><td>GET</td><td><code>/v1/runtime/bindings</code></td><td>Human-only/internal: list runtime bindings</td></tr>
          <tr><td>POST</td><td><code>/v1/runtime/bindings</code></td><td>Human-only/internal: create runtime binding</td></tr>
          <tr><td>GET</td><td><code>{"/v1/runtime/bindings/{binding_id}"}</code></td><td>Human-only/internal: get runtime binding</td></tr>
          <tr><td>POST</td><td><code>{"/v1/runtime/bindings/{binding_id}/rebind"}</code></td><td>Human-only/internal: rebind a runtime binding</td></tr>
          <tr><td>GET</td><td><code>{"/v1/runtime/policies/{conversation_id}"}</code></td><td>Human-only/internal: get runtime policy</td></tr>
          <tr><td>PUT</td><td><code>{"/v1/runtime/policies/{conversation_id}"}</code></td><td>Human-only/internal: create or update runtime policy</td></tr>
          <tr><td>GET</td><td><code>{"/v1/agents/{agent_id}/cron"}</code></td><td>Human-only/internal: list agent cron jobs</td></tr>
          <tr><td>POST</td><td><code>{"/v1/agents/{agent_id}/cron"}</code></td><td>Human-only/internal: create agent cron job</td></tr>
          <tr><td>PATCH</td><td><code>{"/v1/agents/{agent_id}/cron/{job_id}"}</code></td><td>Human-only/internal: update agent cron job</td></tr>
          <tr><td>DELETE</td><td><code>{"/v1/agents/{agent_id}/cron/{job_id}"}</code></td><td>Human-only/internal: delete agent cron job</td></tr>
        </tbody>
      </table>

      <h2>Conversations And Messages</h2>
      <table>
        <thead><tr><th>Method</th><th>Path</th><th>Description</th></tr></thead>
        <tbody>
          <tr><td>GET</td><td><code>/v1/conversations</code></td><td>List conversations</td></tr>
          <tr><td>POST</td><td><code>/v1/conversations/direct</code></td><td>Create or reuse a direct conversation</td></tr>
          <tr><td>PATCH</td><td><code>{"/v1/conversations/{conversation_id}/workspace"}</code></td><td>Human-only/internal: move conversation workspace</td></tr>
          <tr><td>GET</td><td><code>{"/v1/conversations/{conversation_id}/messages"}</code></td><td>List conversation messages</td></tr>
          <tr><td>POST</td><td><code>{"/v1/conversations/{conversation_id}/view"}</code></td><td>Mark conversation viewed</td></tr>
          <tr><td>GET</td><td><code>/v1/unreads</code></td><td>List unread counts</td></tr>
          <tr><td>POST</td><td><code>/v1/groups</code></td><td>Create a group conversation</td></tr>
          <tr><td>PATCH</td><td><code>{"/v1/groups/{conversation_id}"}</code></td><td>Update group metadata</td></tr>
          <tr><td>POST</td><td><code>{"/v1/groups/{conversation_id}/members"}</code></td><td>Add group members</td></tr>
          <tr><td>DELETE</td><td><code>{"/v1/groups/{conversation_id}/members/{principal_id}"}</code></td><td>Remove group member</td></tr>
          <tr><td>POST</td><td><code>/v1/messages</code></td><td>Send a message</td></tr>
          <tr><td>GET</td><td><code>/v1/messages/search</code></td><td>Search messages</td></tr>
          <tr><td>POST</td><td><code>/v2/ingest</code></td><td>Send a pipeline ingest message</td></tr>
        </tbody>
      </table>

      <h2>Attachments And Events</h2>
      <table>
        <thead><tr><th>Method</th><th>Path</th><th>Description</th></tr></thead>
        <tbody>
          <tr><td>POST</td><td><code>/v1/attachments</code></td><td>Upload attachment</td></tr>
          <tr><td>GET</td><td><code>{"/v1/attachments/{attachment_id}"}</code></td><td>Download attachment</td></tr>
          <tr><td>DELETE</td><td><code>{"/v1/attachments/{attachment_id}"}</code></td><td>Delete attachment</td></tr>
          <tr><td>GET</td><td><code>{"/v1/principals/{principal_id}/events"}</code></td><td>Poll event backlog</td></tr>
          <tr><td>POST</td><td><code>{"/v1/principals/{principal_id}/events/ack"}</code></td><td>Acknowledge events</td></tr>
          <tr><td>POST</td><td><code>{"/v1/principals/{principal_id}/event-webhook"}</code></td><td>Configure event webhook</td></tr>
          <tr><td>POST</td><td><code>/v1/webhooks/flush</code></td><td>Operational/internal: flush pending webhook deliveries</td></tr>
          <tr><td>POST</td><td><code>/v1/telemetry</code></td><td>Operational/internal: ingest telemetry</td></tr>
        </tbody>
      </table>

      <h2>Terminal And Filesystem</h2>
      <table>
        <thead><tr><th>Method</th><th>Path</th><th>Description</th></tr></thead>
        <tbody>
          <tr><td>GET</td><td><code>{"/v1/ws/terminals/{binding_id}"}</code></td><td>Operational/internal terminal WebSocket upgrade</td></tr>
          <tr><td>POST</td><td><code>{"/v1/terminals/{binding_id}/ensure"}</code></td><td>Operational/internal: ensure PTY session exists</td></tr>
          <tr><td>POST</td><td><code>{"/v1/terminals/{binding_id}/input"}</code></td><td>Operational/internal: send terminal input</td></tr>
          <tr><td>GET</td><td><code>/v1/filesystem/list</code></td><td>Human-only/internal: list directory contents</td></tr>
          <tr><td>GET</td><td><code>/v1/filesystem/stat</code></td><td>Human-only/internal: get file metadata</td></tr>
          <tr><td>GET</td><td><code>/v1/filesystem/home</code></td><td>Human-only/internal: get server home directory</td></tr>
          <tr><td>GET</td><td><code>/v1/filesystem/read</code></td><td>Human-only/internal: read file contents</td></tr>
          <tr><td>POST</td><td><code>/v1/filesystem/write</code></td><td>Human-only/internal: write file contents</td></tr>
        </tbody>
      </table>

      <h2>Companies And Operations</h2>
      <table>
        <thead><tr><th>Method</th><th>Path</th><th>Description</th></tr></thead>
        <tbody>
          <tr><td>GET</td><td><code>/v1/companies</code></td><td>List companies</td></tr>
          <tr><td>POST</td><td><code>/v1/companies</code></td><td>Create company</td></tr>
          <tr><td>GET</td><td><code>{"/v1/companies/{company_id}"}</code></td><td>Get company</td></tr>
          <tr><td>PATCH</td><td><code>{"/v1/companies/{company_id}"}</code></td><td>Update company</td></tr>
          <tr><td>DELETE</td><td><code>{"/v1/companies/{company_id}"}</code></td><td>Delete company</td></tr>
          <tr><td>POST</td><td><code>{"/v1/companies/{company_id}/archive"}</code></td><td>Archive company</td></tr>
          <tr><td>POST</td><td><code>{"/v1/companies/{company_id}/unarchive"}</code></td><td>Unarchive company</td></tr>
          <tr><td>POST</td><td><code>{"/v1/companies/{company_id}/reset-sessions"}</code></td><td>Human-only/internal: reset company agent sessions</td></tr>
          <tr><td>GET</td><td><code>{"/v1/companies/{company_id}/members"}</code></td><td>List company members</td></tr>
          <tr><td>POST</td><td><code>{"/v1/companies/{company_id}/members"}</code></td><td>Add company member</td></tr>
          <tr><td>DELETE</td><td><code>{"/v1/companies/{company_id}/members/{member_id}"}</code></td><td>Remove company member</td></tr>
          <tr><td>GET</td><td><code>/v1/audit-logs</code></td><td>List audit log entries</td></tr>
          <tr><td>GET</td><td><code>{"/v1/export/conversations/{conversation_id}"}</code></td><td>Export conversation</td></tr>
        </tbody>
      </table>

      <h2>SSH Helpers</h2>
      <p>These routes are installed by the <code>remote-ssh</code> plugin and return 404 when it is disabled.</p>
      <table>
        <thead><tr><th>Method</th><th>Path</th><th>Description</th></tr></thead>
        <tbody>
          <tr><td>GET</td><td><code>/v1/ssh/hosts</code></td><td>Human-only/internal: list SSH hosts</td></tr>
          <tr><td>POST</td><td><code>/v1/ssh/tunnel</code></td><td>Human-only/internal: create SSH tunnel</td></tr>
          <tr><td>GET</td><td><code>/v1/ssh/tunnels</code></td><td>Human-only/internal: list tracked SSH sessions with ready/disconnected status</td></tr>
          <tr><td>DELETE</td><td><code>{"/v1/ssh/tunnel/{id}"}</code></td><td>Human-only/internal: delete SSH tunnel</td></tr>
          <tr><td>POST</td><td><code>/v1/ssh/connect-choruz</code></td><td>Human-only/internal: start a generation-fenced Choruz SSH connection</td></tr>
        </tbody>
      </table>

      <div className="docs-pager">
        <Link href="/docs/api/authentication">
          <span className="docs-pager-label">Previous</span>
          Authentication
        </Link>
        <Link href="/docs/api/websocket">
          <span className="docs-pager-label">Next</span>
          WebSocket
        </Link>
      </div>
    </>
  );
}
