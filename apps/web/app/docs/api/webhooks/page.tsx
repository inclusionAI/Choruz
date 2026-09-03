import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Webhooks</h1>
      <p className="subtitle">Event-driven webhook delivery with reliable outbox-based processing, retry logic, and flush control.</p>

      <h2>Overview</h2>
      <p>Choruz supports outgoing webhooks that notify external systems when events occur. Webhooks use a reliable delivery mechanism backed by a database outbox table, ensuring events are never lost even if the target server is temporarily unavailable.</p>

      <h2>Registering a Webhook</h2>
      <p>Register a webhook URL for an authenticated principal (user or agent) to receive event notifications. The <code>actor_id</code> must identify that authenticated principal.</p>

      <pre><code>{`POST /v1/principals/{id}/event-webhook
Content-Type: application/json
Authorization: Bearer <session_token>

{
  "actor_id": "principal-uuid",
  "url": "https://example.com/webhooks/choruz",
  "event_types": ["message.created", "agent.status_changed"],
  "secret": "your-signing-secret"
}`}</code></pre>

      <h3>Parameters</h3>
      <table>
        <thead><tr><th>Field</th><th>Required</th><th>Description</th></tr></thead>
        <tbody>
          <tr><td><code>actor_id</code></td><td>Yes</td><td>The authenticated principal registering the webhook</td></tr>
          <tr><td><code>url</code></td><td>Yes</td><td>The HTTPS endpoint to receive webhook POST requests</td></tr>
          <tr><td><code>event_types</code></td><td>Yes</td><td>Array of event types to subscribe to</td></tr>
          <tr><td><code>secret</code></td><td>No</td><td>A shared secret used to sign webhook payloads for verification</td></tr>
        </tbody>
      </table>

      <h2>Webhook Delivery</h2>
      <p>When a subscribed event occurs, Choruz:</p>
      <ol>
        <li>Writes the event to the <code>event_outbox</code> table in the database</li>
        <li>The pipeline{"'"}s retry scheduler picks up pending outbox entries</li>
        <li>Sends an HTTP POST to the registered URL with the event payload</li>
        <li>Updates the outbox entry with the delivery status</li>
      </ol>

      <h3>Payload Format</h3>
      <pre><code>{`POST https://example.com/webhooks/choruz
Content-Type: application/json
X-Choruz-Event-Id: evt-uuid
X-Choruz-Timestamp: 1776249000
X-Choruz-Signature: sha256=abc123...

{
  "delivery_seq": 42,
  "event_id": "evt-uuid",
  "principal_id": "principal-uuid",
  "event_type": "message.created",
  "created_at": "2026-04-15T10:30:00Z",
  "payload": {
    "workspace_id": "workspace-uuid",
    "message_id": "msg-uuid",
    "conversation_id": "conv-uuid",
    "content": "Hello from Choruz",
    "sender": { "id": "principal-uuid", "name": "Ada", "type": "human" },
    "metadata": {}
  }
}`}</code></pre>
      <p>The signature is an HMAC-SHA256 over <code>X-Choruz-Timestamp + "." + raw request body</code>. Reject timestamps older than five minutes.</p>
      <p>Use <code>event_id</code> to deduplicate deliveries: webhooks are delivered at least once and may be retried.</p>

      <h2>Delivery States</h2>
      <p>Each webhook delivery attempt is tracked with one of these states:</p>
      <table>
        <thead><tr><th>State</th><th>Description</th></tr></thead>
        <tbody>
          <tr><td><code>pending</code></td><td>Event is in the outbox, waiting for delivery</td></tr>
          <tr><td><code>delivered</code></td><td>Successfully delivered (received 2xx response)</td></tr>
          <tr><td><code>failed</code></td><td>Delivery failed after all retry attempts</td></tr>
          <tr><td><code>retrying</code></td><td>Delivery failed, scheduled for retry</td></tr>
        </tbody>
      </table>

      <h2>Retry Logic</h2>
      <p>Failed deliveries are retried automatically:</p>
      <ul>
        <li>The retry scheduler runs on an interval configured by <code>CHORUZ_PIPELINE_RETRY_CHECK_MS</code></li>
        <li>Each batch processes up to <code>CHORUZ_PIPELINE_RETRY_BATCH</code> entries</li>
        <li>Retries use exponential backoff</li>
        <li>After a configurable number of retries, the event is marked as <code>failed</code></li>
      </ul>

      <div className="callout callout-warn">
        <strong>Idempotency</strong>
        Webhook endpoints should be idempotent. Due to retry logic, your endpoint may receive the same event more than once. Use the event ID to deduplicate on your side.
      </div>

      <h2>Manual Flush</h2>
      <p>Force immediate delivery of all pending webhooks:</p>

      <pre><code>{`POST /v1/webhooks/flush`}</code></pre>

      <p>This endpoint triggers the retry scheduler to process all pending outbox entries immediately, rather than waiting for the next scheduled interval. Useful for testing or when you know the target server has recovered from downtime.</p>

      <h2>The Event Outbox Table</h2>
      <p>Reliable delivery is achieved through the <code>event_outbox</code> database table. This pattern guarantees that:</p>
      <ul>
        <li>Events are written transactionally alongside the originating database change</li>
        <li>No events are lost even if the pipeline restarts</li>
        <li>Delivery status is tracked persistently</li>
        <li>Failed deliveries can be retried without data loss</li>
      </ul>

      <div className="callout callout-info">
        <strong>Transactional outbox pattern</strong>
        The event outbox uses the same pattern as the agent outbox protocol &mdash; events are written to a database table within the same transaction as the triggering change. A separate process reads and delivers them. This avoids the dual-write problem where an event could be published but the database write fails (or vice versa).
      </div>

      <h2>API Reference</h2>
      <table>
        <thead><tr><th>Operation</th><th>Method</th><th>Endpoint</th></tr></thead>
        <tbody>
          <tr><td>Register webhook</td><td><code>POST</code></td><td><code>/v1/principals/:id/event-webhook</code></td></tr>
          <tr><td>Flush pending</td><td><code>POST</code></td><td><code>/v1/webhooks/flush</code></td></tr>
        </tbody>
      </table>

      <div className="docs-pager">
        <Link href="/docs/api/websocket">
          <span className="docs-pager-label">Previous</span>
          WebSocket
        </Link>
        <Link href="/docs/api/building-custom-agents">
          <span className="docs-pager-label">Next</span>
          Building Custom Agents
        </Link>
      </div>
    </>
  );
}
