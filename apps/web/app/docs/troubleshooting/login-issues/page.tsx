import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Local Session Issues</h1>
      <p className="subtitle">Resolving problems when opening the local Dashboard.</p>

      <h2>Dashboard Does Not Open</h2>
      <p>
        Open the web UI using <code>http://127.0.0.1:3100</code> and confirm the
        API gateway is listening on its configured loopback port. Local session
        bootstrap is intentionally rejected when the gateway sees a non-loopback
        peer; remote access must use Remote Control pairing.
      </p>

      <h2>Session Expiration</h2>
      <p>If the Dashboard repeatedly redirects, check <code>CHORUZ_SESSION_TTL_HOURS</code> and verify the web and gateway URLs use the same loopback hostname. The default lifetime is 87600 hours for local development.</p>

      <div className="docs-pager">
        <Link href="/docs/operations/backup">
          <span className="docs-pager-label">Previous</span>
          Backup &amp; Restore
        </Link>
        <Link href="/docs/troubleshooting/agent-not-responding">
          <span className="docs-pager-label">Next</span>
          Agent Not Responding
        </Link>
      </div>
    </>
  );
}
