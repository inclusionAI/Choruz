import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Deployment Guide</h1>
      <p className="subtitle">Running Choruz as a production service using systemd and Caddy.</p>

      <p>For production environments, we recommend running Choruz as a set of background services managed by <strong>systemd</strong>, with <strong>Caddy</strong> or <strong>Nginx</strong> serving as a reverse proxy for TLS termination and load balancing.</p>

      <div className="callout callout-warn">
        <strong>Breaking-change procedure:</strong> Before converting an existing installation, follow the <Link href="/docs/operations/runtime-conversion">runtime conversion rehearsal guide</Link>. Stop legacy writers and verify PostgreSQL and filesystem backups before any conversion.
      </div>

      <h2>Systemd Services</h2>
      <p>Choruz provides example systemd unit files in the <code>infra/ops/systemd/</code> directory. You should copy these to <code>/etc/systemd/system/</code> and customize the environment variables.</p>
      
      <ul>
        <li><strong>choruz-api-gateway.service:</strong> Manages the Rust API gateway process.</li>
        <li><strong>choruz-web-app.service:</strong> Manages the Next.js frontend (running on Node.js).</li>
        <li><strong>choruz-backup.service:</strong> A one-shot service for performing database backups.</li>
      </ul>

      <pre><code>{`# Example systemd enable and start
sudo systemctl daemon-reload
sudo systemctl enable choruz-api-gateway choruz-web-app
sudo systemctl start choruz-api-gateway choruz-web-app`}</code></pre>

      <h2>Reverse Proxy (Caddy)</h2>
      <p>Caddy is the simplest way to add HTTPS to your Choruz instance. A sample <code>Caddyfile</code> is provided in <code>infra/ops/caddy/Caddyfile</code>.</p>
      
      <p>The Caddy configuration handles the following routing:</p>
      <ul>
        <li><code>/v1/*</code> &rarr; Proxied to the API Gateway (port 3000).</li>
        <li><code>/healthz</code> &rarr; API Gateway process liveness.</li>
        <li><code>/readyz</code> &rarr; API Gateway dependency readiness.</li>
        <li><code>/*</code> &rarr; Proxied to the Next.js Web App (port 3100).</li>
      </ul>

      <div className="callout callout-warn">
        <strong>No Metrics Endpoint</strong>
        The production Caddy configuration explicitly does NOT expose the <code>/metrics</code> endpoint of the pipeline. Metrics are intended for internal monitoring only and should not be accessible over the public internet.
      </div>

      <h2>Security Hardening</h2>
      <ol>
        <li><strong>Bind Address:</strong> In production, set <code>CHORUZ_API_HOST</code> and <code>CHORUZ_PIPELINE_METRICS_HOST</code> to <code>127.0.0.1</code> to ensure they are only accessible via the local reverse proxy.</li>
        <li><strong>TLS:</strong> Use Caddy{"'"}s automatic TLS or provide your own certificates if using Nginx.</li>
        <li><strong>Firewall:</strong> Ensure only ports 80 (HTTP) and 443 (HTTPS) are open to the public. Ports 3000, 3020, and 3100 should be firewalled.</li>
      </ol>

      <h2>Architecture Diagram</h2>
      <pre><code>{`Internet -> Caddy (TLS) -> Gateway (3000)
                        -> Web App (3100)
                        -> Pipeline (3020) [Internal only]`}</code></pre>

      <div className="docs-pager">
        <Link href="/docs/operations/env-vars">
          <span className="docs-pager-label">Previous</span>
          Environment Variables
        </Link>
        <Link href="/docs/operations/backup">
          <span className="docs-pager-label">Next</span>
          Backup &amp; Restore
        </Link>
      </div>
    </>
  );
}
