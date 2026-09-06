import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>WebSocket Events</h1>
      <p className="subtitle">Receive authenticated dashboard changes with a cursor for each device.</p>
      <h2>Connecting</h2>
      <p>The API gateway serves <code>/v1/ws/sync?device_id=DEVICE&amp;cursor=CURSOR</code> using the authenticated session. Each client keeps a stable device identifier and its last applied cursor. The pipeline port serves health, readiness and metrics.</p>
      <h2>Applying changes</h2>
      <p>The server begins with <code>sync_ready</code>, containing the device identifier, accepted cursor and feed head. It then sends <code>sync_changes</code> pages with <code>changes</code>, <code>next_cursor</code>, <code>head_cursor</code> and <code>has_more</code>.</p>
      <ol>
        <li>Apply each page to local state in order.</li>
        <li>Persist the page&apos;s <code>next_cursor</code> after applying it.</li>
        <li>Send <code>{'{"type":"sync_ack","cursor":123}'}</code> with that applied cursor.</li>
        <li>Reconnect with the saved cursor after a disconnection.</li>
      </ol>
      <p>The server replies with <code>sync_acked</code> after recording an acknowledgement. It rejects acknowledgements beyond what the connection has received. A failure can produce <code>sync_error</code> with a <code>detail</code> message.</p>
      <h2>Feed boundaries</h2>
      <p>Changes are private to the authenticated principal. The sync cursor belongs to the change feed; it is not a conversation&apos;s message sequence number. Device acknowledgements are independent, so one browser cannot acknowledge another browser&apos;s unapplied changes.</p>
      <p>The dashboard manages the socket, local persistence and reconnection. Custom clients can also read the same feed through <code>GET /v1/sync</code>. A cursor ahead of the current feed is rejected; refresh bootstrap state and register a fresh device when local state is incompatible.</p>
      <div className="docs-pager">
        <Link href="/docs/api/rest"><span className="docs-pager-label">Previous</span>REST API</Link>
        <Link href="/docs/api/webhooks"><span className="docs-pager-label">Next</span>Webhook Events</Link>
      </div>
    </>
  );
}
