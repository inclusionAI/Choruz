import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Backup &amp; Restore</h1>
      <p className="subtitle">Protecting your Choruz data through automated database and filesystem backups.</p>

      <p>A complete Choruz backup consists of three parts: the <strong>PostgreSQL database</strong>, the <strong>shared attachments</strong>, and the <strong>agent secrets</strong> cache.</p>

      <p>For an existing-installation breaking change, verify these backups before conversion and follow the <Link href="/docs/operations/runtime-conversion">runtime conversion rehearsal guide</Link>; do not rely on application startup to migrate prior state.</p>

      <h2>Backup Strategy</h2>
      <p>Choruz provides a set of scripts in <code>infra/ops/bin/</code> to simplify the backup process. These scripts are designed to be run manually or via systemd timers.</p>

      <h3>Manual Backup</h3>
      <pre><code>./infra/ops/bin/backup.sh</code></pre>
      <p>The backup script performs the following actions:</p>
      <ol>
        <li>Dumps the database to <code>database.sql</code> using <code>pg_dump</code>.</li>
        <li>Copies the contents of <code>CHORUZ_ATTACHMENT_DIR</code> to the backup folder.</li>
        <li>Copies <code>agent_tokens.json</code> to the backup folder.</li>
        <li>Generates a <code>metadata.json</code> file with the timestamp and configuration details.</li>
      </ol>

      <h3>Automated Backups</h3>
      <p>Production installations should use the provided systemd timer to schedule daily backups:</p>
      <pre><code>{`sudo systemctl enable choruz-backup.timer
sudo systemctl start choruz-backup.timer`}</code></pre>
      <p>Backups are stored in the directory defined by the <code>CHORUZ_BACKUP_DIR</code> variable (defaults to <code>~/.choruz-backups</code>).</p>

      <h2>Restore Procedure</h2>
      <p>To restore from a backup, use the <code>restore.sh</code> script and provide the path to the backup folder:</p>
      <pre><code>./infra/ops/bin/restore.sh /path/to/backup/20260426120000</code></pre>

      <div className="callout callout-warn">
        <strong>Warning: Service Interruption</strong>
        The restore process will wipe the current database and overwrite the attachments directory. It is recommended to stop the <code>choruz-api-gateway</code> and <code>choruz-pipeline</code> services before performing a restore.
      </div>

      <h2>Backup Contents</h2>
      <table>
        <thead>
          <tr>
            <td>File</td>
            <td>Source</td>
            <td>Description</td>
          </tr>
        </thead>
        <tbody>
          <tr>
            <td><code>database.sql</code></td>
            <td>PostgreSQL</td>
            <td>All principals, conversations, and messages.</td>
          </tr>
          <tr>
            <td><code>attachments/</code></td>
            <td>Filesystem</td>
            <td>Shared files, images, and binary data.</td>
          </tr>
          <tr>
            <td><code>agent_tokens.json</code></td>
            <td>Filesystem</td>
            <td>Cached Bearer secrets for agents.</td>
          </tr>
          <tr>
            <td><code>metadata.json</code></td>
            <td>Script</td>
            <td>Backup metadata (date, version, etc.).</td>
          </tr>
        </tbody>
      </table>

      <div className="docs-pager">
        <Link href="/docs/operations/deployment">
          <span className="docs-pager-label">Previous</span>
          Production Deployment
        </Link>
        <Link href="/docs/troubleshooting/login-issues">
          <span className="docs-pager-label">Next</span>
          Login Issues
        </Link>
      </div>
    </>
  );
}
