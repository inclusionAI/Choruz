"use client";

import { useCallback, useEffect, useState } from "react";
import { trace } from "../../lib/api/choruz-trace";
import { Modal } from "../ui/modal";
import {
  connectChoruzSshTunnel,
  deleteSshTunnel,
  listSshHosts,
  listSshTunnels,
  type SshHost,
  type SshTunnel,
} from "../../lib/api/choruz-api";
import { REMOTE_SERVER_INSTALL_COMMAND } from "../../lib/remote/remote-server-install";

// ── Component ─────────────────────────────────────────────────────────

export type ServerManagerProps = {
  sessionToken: string;
  onClose: () => void;
};

export function ServerManager({ sessionToken, onClose }: ServerManagerProps) {
  const [hosts, setHosts] = useState<SshHost[]>([]);
  const [loadingHosts, setLoadingHosts] = useState(false);
  const [hostsError, setHostsError] = useState<string | null>(null);
  const [hiddenHosts, setHiddenHosts] = useState<string[]>([]);

  // Tracked SSH sessions (ready or disconnected) from the server.
  const [tunnels, setTunnels] = useState<SshTunnel[]>([]);

  // Connect-in-progress state. No more port inputs — backend picks
  // the local port from the OS-assigned free range and handshakes with
  // choruz-server on the remote for the remote port.
  const [connectingHosts, setConnectingHosts] = useState<Set<string>>(() => new Set());
  const [connectErrors, setConnectErrors] = useState<Record<string, string>>({});

  useEffect(() => {
    try {
      const saved = JSON.parse(localStorage.getItem("choruz_hidden_ssh_hosts") ?? "[]");
      if (Array.isArray(saved)) setHiddenHosts(saved);
    } catch {}
  }, []);

  const toggleHideHost = useCallback((name: string) => {
    setHiddenHosts(prev => {
      const hidden = !prev.includes(name);
      trace.event("server_hide", { host: name, hidden });
      const next = prev.includes(name) ? prev.filter(n => n !== name) : [...prev, name];
      try { localStorage.setItem("choruz_hidden_ssh_hosts", JSON.stringify(next)); } catch {}
      return next;
    });
  }, []);

  // ── Load SSH hosts ──
  const loadHosts = useCallback(async () => {
    setLoadingHosts(true);
    setHostsError(null);
    try {
      const data = await listSshHosts(sessionToken);
      setHosts(data);
    } catch (e) {
      setHostsError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoadingHosts(false);
    }
  }, [sessionToken]);

  const loadTunnels = useCallback(async () => {
    try {
      const data = await listSshTunnels(sessionToken);
      setTunnels(data);
    } catch { /* ignore */ }
  }, [sessionToken]);

  useEffect(() => {
    loadHosts();
    loadTunnels();
  }, [loadHosts, loadTunnels]);

  // ── Connect (VS-Code-Remote-SSH-style, one click) ──
  //
  // Backend runs `ssh <host> 'choruz-server'` + parses the handshake line +
  // picks a free local port + opens the tunnel. We just say `host.name`
  // and wait for the returned `{local_port}`.
  const handleConnect = useCallback(async (host: SshHost) => {
    setConnectingHosts((previous) => new Set(previous).add(host.name));
    setConnectErrors((previous) => {
      const next = { ...previous };
      delete next[host.name];
      return next;
    });
    const span = trace.start("ssh_tunnel_connect", { host: host.name });
    try {
      const tunnel = await connectChoruzSshTunnel(sessionToken, { host: host.name });
      span.end({
        status: "ok",
        tunnel_id: tunnel.id,
        local_port: tunnel.local_port,
        remote_port: tunnel.remote_port,
      });
      await loadTunnels();
      if (typeof window !== "undefined") {
        window.open(`http://localhost:${tunnel.local_port}`, "_blank", "noopener");
      }
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      span.end({ error: msg });
      setConnectErrors((previous) => ({ ...previous, [host.name]: msg }));
    } finally {
      setConnectingHosts((previous) => {
        const next = new Set(previous);
        next.delete(host.name);
        return next;
      });
    }
  }, [sessionToken, loadTunnels]);

  // ── Disconnect tunnel ──
  const handleDisconnect = useCallback(async (tunnelId: string) => {
    try {
      await deleteSshTunnel(sessionToken, tunnelId);
      await loadTunnels();
    } catch (e) {
      // Even if the delete fails, refresh so a stale entry gets cleaned up.
      await loadTunnels();
      alert(`Disconnect failed: ${e instanceof Error ? e.message : e}`);
    }
  }, [sessionToken, loadTunnels]);

  const tunnelForHost = useCallback(
    (hostName: string) =>
      tunnels.find(
        (t) => t.host === hostName && t.generation !== null && t.status === "ready",
      ) ??
      tunnels.find((t) => t.host === hostName && t.generation !== null) ??
      tunnels.find((t) => t.host === hostName && t.status === "ready") ??
      tunnels.find((t) => t.host === hostName) ??
      null,
    [tunnels],
  );

  // ── Render ──
  return (
    <Modal
      title="Remote Servers"
      onClose={onClose}
      className="server-manager-card"
      headerActions={
        <button type="button" className="server-manager-btn" onClick={() => { loadHosts(); loadTunnels(); }}>
          Refresh
        </button>
      }
    >
      <p className="server-manager-intro">
        Connect to a remote Choruz instance over SSH — click <em>Connect</em> and we SSH in, start <code>choruz-server</code>, pick a free local port, and open the UI.
        <br />
        <strong>Prerequisite:</strong> the remote must already have <code>choruz-server</code> on its <code>$PATH</code>. Install it yourself on the remote, e.g.:
        <br />
        <span>Use an existing authorized checkout until the public URL is approved, then run:</span>
        <br />
        <code style={{ display: "inline-block", marginTop: 4 }}>
          {REMOTE_SERVER_INSTALL_COMMAND}
        </code>
        <br />
        <strong>To disconnect:</strong> come back to this modal and click <em>Disconnect</em>. Closing the remote tab does <em>not</em> tear down the tunnel or the remote <code>choruz-server</code> process — those keep running until you explicitly Disconnect.
      </p>

      {loadingHosts && <p className="server-manager-hint">Scanning ~/.ssh/config…</p>}
      {hostsError && (
        <div className="server-manager-error-block">
          <p>{hostsError}</p>
          <button className="server-manager-btn" onClick={loadHosts} style={{ marginTop: "var(--space-2)" }}>
            Retry
          </button>
        </div>
      )}
      {!loadingHosts && !hostsError && hosts.length === 0 && (
        <p className="server-manager-hint">
          No hosts found in ~/.ssh/config. Add SSH host entries and try again.
        </p>
      )}

      {hosts.filter(h => !hiddenHosts.includes(h.name)).map((host) => {
        const tunnel = tunnelForHost(host.name);
        const readyTunnel = tunnel?.status === "ready" ? tunnel : null;
        const disconnectedTunnel = tunnel?.status === "disconnected" ? tunnel : null;
        const connecting = connectingHosts.has(host.name);
        const connectError = connectErrors[host.name];
        return (
          <div key={host.name}>
            <div className="server-manager-host-card" data-ssh-host={host.name}>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div className="host-name">{host.name}</div>
                <div className="host-meta">
                  {host.user ? `${host.user}@` : ""}
                  {host.hostname ?? host.name}
                  {host.port ? `:${host.port}` : ""}
                </div>
                {readyTunnel && (
                  <div className="host-status">
                    Tunnel active: localhost:{readyTunnel.local_port} → {host.name} (remote {readyTunnel.remote_port})
                    {readyTunnel.pid ? ` (pid ${readyTunnel.pid})` : ""}
                  </div>
                )}
                {disconnectedTunnel && (
                  <div className="server-manager-error" role="status">
                    Disconnected{disconnectedTunnel.last_error ? `: ${disconnectedTunnel.last_error}` : ""}
                  </div>
                )}
                {connectError && (
                  <div className="server-manager-error" role="alert">
                    {connectError}
                  </div>
                )}
              </div>
              <button
                className="server-manager-hide-btn"
                onClick={() => toggleHideHost(host.name)}
                title="Hide this server"
              >
                Hide
              </button>
              {readyTunnel ? (
                <>
                  <button
                    className="server-manager-btn"
                    onClick={() => {
                      if (typeof window !== "undefined") {
                        window.open(`http://localhost:${readyTunnel.local_port}`, "_blank", "noopener");
                      }
                    }}
                  >
                    Open
                  </button>
                  <button
                    className="server-manager-btn server-manager-btn--danger"
                    onClick={() => handleDisconnect(readyTunnel.id)}
                  >
                    Disconnect
                  </button>
                </>
              ) : disconnectedTunnel ? (
                <>
                  <button
                    className="server-manager-btn server-manager-btn--primary"
                    aria-label={`Reconnect to ${host.name}`}
                    aria-busy={connecting}
                    onClick={() => void handleConnect(host)}
                    disabled={connecting}
                  >
                    {connecting ? "Reconnecting…" : "Reconnect"}
                  </button>
                  <button
                    className="server-manager-btn"
                    onClick={() => handleDisconnect(disconnectedTunnel.id)}
                  >
                    Dismiss
                  </button>
                </>
              ) : (
                <button
                  className="server-manager-btn server-manager-btn--primary"
                  aria-label={`Connect to ${host.name}`}
                  aria-busy={connecting}
                  onClick={() => {
                    void handleConnect(host);
                  }}
                  disabled={connecting}
                  title="Runs choruz-server on the remote via SSH, tunnels its port to a free local port, and opens the UI."
                >
                  {connecting ? "Connecting…" : "Connect"}
                </button>
              )}
            </div>
          </div>
        );
      })}

      {hiddenHosts.length > 0 && (
        <button
          className="server-manager-btn"
          onClick={() => setHiddenHosts([])}
          style={{ marginTop: "var(--space-1)" }}
        >
          Show {hiddenHosts.length} hidden server{hiddenHosts.length > 1 ? "s" : ""}
        </button>
      )}
    </Modal>
  );
}
