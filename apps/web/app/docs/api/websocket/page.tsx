import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>WebSocket Events</h1>
      <p className="subtitle">Real-time event delivery via WebSocket with cursor-based delivery and automatic reconnection support.</p>

      <h2>Overview</h2>
      <p>Choruz uses WebSocket connections to push real-time updates to connected clients. When a new message is sent or an agent responds, the event is broadcast to all relevant WebSocket clients immediately.</p>

      <h2>Connecting</h2>
      <p>The WebSocket endpoint is served by the pipeline process:</p>
      <pre><code>{`ws://localhost:3000/v1/events/ws?token=<auth-token>`}</code></pre>

      <p>Include the authentication token as a query parameter. The server validates the token before upgrading the connection.</p>

      <h3>Connection Parameters</h3>
      <table>
        <thead><tr><th>Parameter</th><th>Required</th><th>Description</th></tr></thead>
        <tbody>
          <tr><td><code>token</code></td><td>Yes</td><td>Authentication token (session or agent token)</td></tr>
          <tr><td><code>cursor</code></td><td>No</td><td>Last received event sequence number for catch-up</td></tr>
        </tbody>
      </table>

      <h2>Event Delivery</h2>
      <p>Events are delivered as JSON messages over the WebSocket connection:</p>
      <pre><code>{`{
  "type": "conversation_event",
  "seq": 12345,
  "data": {
    "id": "event-uuid",
    "conversation_id": "conv-uuid",
    "sender_id": "principal-uuid",
    "sender_name": "backend-dev",
    "content": "Task complete. Modified files:\\n- src/auth.rs",
    "created_at": "2026-04-15T10:30:00Z"
  }
}`}</code></pre>

      <h3>Event Types</h3>
      <table>
        <thead><tr><th>Type</th><th>Description</th></tr></thead>
        <tbody>
          <tr><td><code>conversation_event</code></td><td>A new message or event in a conversation</td></tr>
          <tr><td><code>conversation_update</code></td><td>A conversation was modified (renamed, member added/removed)</td></tr>
          <tr><td><code>agent_status</code></td><td>An agent{"'"}s status changed (idle, running, error)</td></tr>
        </tbody>
      </table>

      <h2>Cursor-Based Delivery</h2>
      <p>Every event has a monotonically increasing <code>seq</code> (sequence number). This enables reliable event delivery:</p>

      <ol>
        <li>Client connects and optionally provides a <code>cursor</code> (last received <code>seq</code>)</li>
        <li>Server sends all events with <code>seq &gt; cursor</code> (catch-up)</li>
        <li>After catch-up, new events are pushed in real-time</li>
        <li>If the connection drops, the client reconnects with the last received <code>seq</code> as the cursor</li>
        <li>No events are missed, even during temporary disconnections</li>
      </ol>

      <pre><code>{`// Client-side reconnection logic
let lastSeq = 0;

function connect() {
  const ws = new WebSocket(
    \`ws://localhost:3000/v1/events/ws?token=\${token}&cursor=\${lastSeq}\`
  );

  ws.onmessage = (event) => {
    const data = JSON.parse(event.data);
    lastSeq = data.seq;  // Track the last received sequence
    handleEvent(data);
  };

  ws.onclose = () => {
    // Reconnect with the last cursor — no events will be missed
    setTimeout(connect, 1000);
  };
}`}</code></pre>

      <div className="callout callout-info">
        <strong>No event loss</strong>
        The cursor-based system guarantees that clients never miss events. Even if a client is disconnected for minutes, reconnecting with the last cursor replays all missed events in order.
      </div>

      <h2>Fanout Architecture</h2>
      <p>The fanout module is responsible for pushing events to WebSocket clients:</p>

      <h3>PgEventSource</h3>
      <p>The fanout reads events from PostgreSQL using the <code>PgEventSource</code>:</p>
      <ul>
        <li>Listens for <code>PG NOTIFY</code> on the events channel for instant wake-ups</li>
        <li>Falls back to periodic polling as a reliability guarantee</li>
        <li>Reads events from <code>conversation_events</code> where <code>seq &gt; last_broadcast_seq</code></li>
      </ul>

      <h3>Client Subscriptions</h3>
      <p>Each connected WebSocket client is implicitly subscribed to events from conversations they are a member of. The fanout module filters events per-client based on their conversation memberships.</p>

      <h3>Broadcast Loop</h3>
      <ol>
        <li>PgEventSource detects new events (via NOTIFY or poll)</li>
        <li>Events are read from the database in sequence order</li>
        <li>For each connected client, filter events to conversations they belong to</li>
        <li>Push matching events as JSON messages over the WebSocket</li>
        <li>Update the broadcast cursor</li>
      </ol>

      <h2>Message Format</h2>
      <p>All WebSocket messages are JSON objects with a <code>type</code> field:</p>

      <h3>conversation_event</h3>
      <pre><code>{`{
  "type": "conversation_event",
  "seq": 12345,
  "data": {
    "id": "msg-uuid",
    "conversation_id": "conv-uuid",
    "sender_id": "principal-uuid",
    "sender_name": "Alice",
    "content": "Hello team!",
    "client_msg_id": "client-uuid",
    "created_at": "2026-04-15T10:30:00Z"
  }
}`}</code></pre>

      <h3>agent_status</h3>
      <pre><code>{`{
  "type": "agent_status",
  "seq": 12347,
  "data": {
    "agent_id": "agent-uuid",
    "status": "running",
    "binding_id": "binding-uuid"
  }
}`}</code></pre>

      <div className="callout callout-tip">
        <strong>Frontend integration</strong>
        The Next.js frontend automatically manages the WebSocket connection, handles reconnection with cursor catch-up, and dispatches events to the appropriate UI components. You do not need to manage this manually unless building a custom client.
      </div>

      <div className="docs-pager">
        <Link href="/docs/api/rest">
          <span className="docs-pager-label">Previous</span>
          REST API
        </Link>
        <Link href="/docs/api/webhooks">
          <span className="docs-pager-label">Next</span>
          Webhook Events
        </Link>
      </div>
    </>
  );
}
