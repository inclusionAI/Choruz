import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Remote Control</h1>
      <p className="subtitle">Open your running Choruz workspace from another browser through an end-to-end encrypted relay.</p>

      <h2>Pair a browser</h2>
      <ol>
        <li>On the host computer, open <strong>Actions → Remote Control</strong>.</li>
        <li>Confirm that a Cloud Gateway is configured, then select <strong>Generate credential</strong>.</li>
        <li>Open the Remote Control Web Dashboard on the other computer.</li>
        <li>Paste the complete <code>v1.…</code> credential, name the browser, and select <strong>Connect</strong>.</li>
      </ol>
      <p>The credential is single-use and expires. Pairing completes without a second confirmation code.</p>

      <h2>Reconnect or revoke</h2>
      <p>The browser stores its pairing locally and can reconnect from the same dashboard. On the host, use <strong>Paired devices</strong> to revoke a browser that should no longer connect.</p>

      <h2>What leaves the host</h2>
      <p>Messages and essential control events pass through the Cloud Gateway as end-to-end encrypted payloads. Raw tool output, terminal output, files, diffs, and system prompts stay on the host.</p>

      <div className="callout callout-warn">
        <strong>Treat the credential like a temporary password</strong>
        Paste it only into the intended browser. Generate a new credential if the old one expires or may have been copied elsewhere.
      </div>

      <div className="docs-pager">
        <Link href="/docs/features/search"><span className="docs-pager-label">Previous</span>Search</Link>
        <Link href="/docs/features/server-management"><span className="docs-pager-label">Next</span>Remote Servers (SSH)</Link>
      </div>
    </>
  );
}
