import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Runtime Conversion Safety Summary</h1>
      <p className="subtitle">A disposable, offline safety checklist for an existing installation. Read the <Link href="/docs/operations/runtime-conversion/guide">full conversion guide</Link> before conversion.</p>

      <div className="callout callout-warn">
        <strong>Do not run this against a live installation.</strong> Stop every writer, verify PostgreSQL and filesystem backups, and work only from a disposable copy. Choruz does not migrate prior runtime state at startup.
      </div>

      <h2>Before the rehearsal</h2>
      <ol>
        <li>Record processes, listening ports, service-manager state, queue ownership, database identity, runtime root, Git worktrees, and backup checksums.</li>
        <li>Stop all writers. Refuse the rehearsal if another service, queue writer, or runtime owner is still active.</li>
        <li>Create and verify PostgreSQL and filesystem backups, then restore them into a fresh disposable location.</li>
      </ol>

      <h2>Safety procedure</h2>
      <ol>
        <li>Use the full conversion rehearsal record to inventory environment files, database/schema state, runtime paths, Git/worktree markers, browser or desktop state, service units, HTTP contracts, SDKs, bridge configuration, helper binaries, telemetry, logs, and queues.</li>
        <li>Refuse collisions. If both source and Choruz identities exist for the same resource, restore the backup and resolve the collision manually.</li>
        <li>Convert only the disposable copy. Do not add a fallback read, redirect, symlink, dual writer, or compatibility export.</li>
        <li>Drain or intentionally discard stopped source queues before conversion. Never rename an active queue.</li>
        <li>Rebootstrap representative terminal, agent, webhook, SDK, and bridge workspaces; confirm only Choruz names and paths are produced.</li>
        <li>Verify the original backup remains intact. Roll back by restoring the verified backups, not by reverse-renaming a partially converted copy.</li>
      </ol>

      <h2>Required evidence</h2>
      <p>Record the stopped-writer state, backup checksums and restoration, collision refusals, Choruz-only startup, rejected legacy probes, and results for conversations, attachments, webhooks, SDKs, bridges, realtime fanout, and available terminal drivers.</p>

      <div className="docs-pager">
        <Link href="/docs/operations/deployment">
          <span className="docs-pager-label">Previous</span>
          Production Deployment
        </Link>
        <Link href="/docs/operations/backup">
          <span className="docs-pager-label">Next</span>
          Backup &amp; Restore
        </Link>
      </div>
    </>
  );
}
