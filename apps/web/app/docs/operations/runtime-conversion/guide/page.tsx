import Link from "next/link";

export default function Page() {
  return <><h1>Offline Conversion Guide</h1><p>Stop all legacy writers, verify PostgreSQL and filesystem backups, and work only on a disposable copy. Choruz never converts live legacy state at startup.</p><ol><li>Record writers, ports, services, queues, database, runtime root, Git worktrees, and checksums.</li><li>Refuse source/target collisions before any queue or filesystem mutation.</li><li>Explicitly drain or discard every stopped Maildir queue.</li><li>Restore verified backups into a fresh disposable location before conversion; preserve them for rollback and never restore over the converted copy.</li></ol><p>From the repository root in a clean checkout with PostgreSQL command-line tools available, run: <code>bash ./infra/host/runtime_conversion_rehearsal.sh all</code>.</p><Link href="/docs/operations/runtime-conversion">Back to conversion safety summary</Link></>;
}
