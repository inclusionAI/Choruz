import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Server Management</h1>
      <p className="subtitle">Open an SSH port-forward from the UI to browse a remote Choruz install in a local tab. Deployment of Choruz itself is managed outside this UI.</p>

      <p>This feature is provided by the <code>remote-ssh</code> plugin. Excluding it from <code>CHORUZ_PLUGINS</code> removes both the Servers UI and all <code>/v1/ssh/*</code> routes.</p>

      <h2>Overview</h2>
      <p>Choruz&apos;s Server Manager reads your local <code>~/.ssh/config</code>, lists hosts, and — when you click <strong>Connect</strong> — spawns an <code>ssh -L</code> child process that forwards a remote port back to your local browser. It does not clone, build, migrate, or start Choruz on the remote; you run those steps yourself using your own tooling (e.g. git, <code>pnpm dev:all</code>, tmux/systemd).</p>

      <h2>How It Works</h2>
      <p>The Server Manager parses <code>~/.ssh/config</code> and presents each host entry as a row. Each host has <strong>Connect</strong> (which opens a tunnel form) and <strong>Hide</strong> (which removes it from the UI — this does not modify your SSH config).</p>

      <h3>SSH Config Discovery</h3>
      <pre><code>{`# Example ~/.ssh/config
Host my-server
  HostName 192.168.1.100
  User deploy
  IdentityFile ~/.ssh/id_rsa

Host staging
  HostName staging.example.com
  User Choruz
  IdentityFile ~/.ssh/deploy_key`}</code></pre>

      <h2>API Endpoints</h2>
      <table>
        <thead><tr><th>Endpoint</th><th>Method</th><th>Description</th></tr></thead>
        <tbody>
          <tr><td><code>/v1/ssh/hosts</code></td><td><code>GET</code></td><td>List hosts from <code>~/.ssh/config</code></td></tr>
          <tr><td><code>/v1/ssh/tunnel</code></td><td><code>POST</code></td><td>Start an SSH port-forward (body: <code>{`{host, local_port?, remote_port?}`}</code>)</td></tr>
          <tr><td><code>/v1/ssh/tunnels</code></td><td><code>GET</code></td><td>List tracked ready or disconnected SSH sessions</td></tr>
          <tr><td><code>/v1/ssh/tunnel/&#123;id&#125;</code></td><td><code>DELETE</code></td><td>Kill the ssh child process for a tunnel</td></tr>
        </tbody>
      </table>

      <h2>Connecting to a Remote Choruz</h2>
      <ol>
        <li>Click <strong>Connect</strong> on the desired host.</li>
        <li>Adjust <strong>Local port</strong> / <strong>Remote port</strong> (defaults to <code>3100</code>, matching <code>pnpm dev</code>).</li>
        <li>Click <strong>Start tunnel</strong>. Choruz spawns <code>ssh -N -L &lt;local&gt;:localhost:&lt;remote&gt; &lt;host&gt;</code> in the background and opens <code>http://localhost:&lt;local&gt;</code> in a new tab.</li>
        <li>When done, click <strong>Disconnect</strong> to terminate the tunnel.</li>
      </ol>

      <div className="callout callout-warn">
        <strong>Tunnel lifetime</strong>
        Tunnels are tracked in memory on the API gateway. Restarting the gateway terminates all active tunnels. If you need auto-reconnect, wrap <code>ssh -N -L ...</code> with <code>autossh</code> yourself at the shell.
      </div>

      <h2>Deploying Choruz on the Remote</h2>
      <p>Server Manager does not deploy Choruz. To set up Choruz on a remote host:</p>
      <ol>
        <li>SSH in and clone the repo: <code>git clone &lt;repo&gt; &amp;&amp; cd choruz/non_docker</code></li>
        <li>Install deps and run <code>pnpm dev:all</code>, or use your own orchestration.</li>
        <li>Once the remote web UI is listening on port 3100, come back to the Server Manager and click <strong>Connect</strong>.</li>
      </ol>

      <h2>Prerequisites</h2>
      <ul>
        <li><strong>SSH config</strong> &mdash; Hosts must be defined in <code>~/.ssh/config</code></li>
        <li><strong>Passwordless SSH</strong> &mdash; The spawned <code>ssh</code> child cannot answer password prompts; use keys + ssh-agent.</li>
        <li><strong>Remote Choruz already running</strong> &mdash; The tunnel just forwards a port; something must be listening on the remote side.</li>
      </ul>

      <div className="docs-pager">
        <Link href="/docs/features/pixel-world">
          <span className="docs-pager-label">Previous</span>
          Pixel World
        </Link>
        <Link href="/docs/agents/drivers">
          <span className="docs-pager-label">Next</span>
          Agent Drivers
        </Link>
      </div>
    </>
  );
}
