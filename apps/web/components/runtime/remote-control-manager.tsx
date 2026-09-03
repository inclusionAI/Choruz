"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { Spinner } from "../ui/spinner";

import {
  createRemoteControlPairing,
  fetchRemoteControlSettings,
  listRemoteControlDevices,
  revokeRemoteControlDevice,
} from "../../lib/api/choruz-api";
import {
  type RemoteControlDevice,
  type RemoteControlPairing,
  type RemoteControlSettings,
  remoteDashboardPath,
} from "../../lib/remote/remote-control";
import { Modal } from "../ui/modal";

type Props = { sessionToken: string; onClose: () => void };

export function RemoteControlManager({ sessionToken, onClose }: Props) {
  const [settings, setSettings] = useState<RemoteControlSettings | null>(null);
  const [devices, setDevices] = useState<RemoteControlDevice[]>([]);
  const [pairing, setPairing] = useState<RemoteControlPairing | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pairingStatus, setPairingStatus] = useState<string | null>(null);
  const [remoteCredential, setRemoteCredential] = useState("");
  const pairingWatchRef = useRef<AbortController | null>(null);
  useEffect(() => () => pairingWatchRef.current?.abort(), []);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [nextSettings, nextDevices] = await Promise.all([
        fetchRemoteControlSettings(sessionToken),
        listRemoteControlDevices(sessionToken),
      ]);
      setSettings(nextSettings);
      setDevices(nextDevices);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setLoading(false);
    }
  }, [sessionToken]);

  useEffect(() => { void load(); }, [load]);

  const connectToRemoteHost = () => {
    setError(null);
    try {
      window.location.assign(remoteDashboardPath(
        settings?.gateway_url ?? "",
        remoteCredential,
        navigator.platform ? `${navigator.platform} browser` : "Choruz browser",
      ));
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  };

  const beginPairing = async () => {
    setSaving(true);
    setError(null);
    try {
      pairingWatchRef.current?.abort();
      const nextPairing = await createRemoteControlPairing(sessionToken);
      setPairing(nextPairing);
      setPairingStatus("Waiting for a remote browser…");
      const controller = new AbortController();
      pairingWatchRef.current = controller;
      const knownDevices = new Set(devices.map((device) => device.id));
      void (async () => {
        const expiresAt = new Date(nextPairing.expires_at).getTime();
        while (!controller.signal.aborted && Date.now() < expiresAt) {
          await new Promise((resolve) => setTimeout(resolve, 1_000));
          if (controller.signal.aborted) return;
          const nextDevices = await listRemoteControlDevices(sessionToken);
          const added = nextDevices.find((device) => !knownDevices.has(device.id));
          if (added) {
            setDevices(nextDevices);
            setPairingStatus(`${added.name} paired`);
            return;
          }
        }
        if (!controller.signal.aborted) {
          setPairingStatus("Pairing credential expired. Generate a new one to try again.");
        }
      })().catch((reason) => {
        if (!controller.signal.aborted) {
          setError(reason instanceof Error ? reason.message : String(reason));
        }
      });
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal
      title="Remote Control"
      description="Control this Choruz workspace from another browser without moving the agent runtime off this computer."
      onClose={onClose}
      className="remote-control-card"
    >
      {error ? <p className="server-manager-error-block" role="alert">{error}</p> : null}
      {loading || !settings ? <p><Spinner label="Loading remote-control settings…" /></p> : (
        <>
          <section className="remote-control-section" aria-labelledby="remote-network-title">
            <h3 id="remote-network-title">Connection</h3>
            <p className="server-manager-hint">Messages and essential control events are always end-to-end encrypted. Raw tools, terminal output, files, diffs, and system prompts stay local.</p>
            <p><strong>Cloud Gateway</strong></p>
            <p className="server-manager-hint">Connect through Choruz Cloud for reliable access from any network.</p>
            {!settings.gateway_url ? (
              <p className="server-manager-hint" role="status">Cloud Gateway is not configured yet.</p>
            ) : null}
          </section>

          <section className="remote-control-section" aria-labelledby="remote-pair-title">
            <div className="remote-control-row">
              <h3 id="remote-pair-title">Pair a Web browser</h3>
              <button className="server-manager-btn server-manager-btn--primary" disabled={saving} onClick={() => void beginPairing()}>
                Generate credential
              </button>
            </div>
            {pairing ? (
              <>
                <div className="remote-pairing-code" role="status" aria-live="polite">
                  <code>{pairing.credential}</code>
                  <small>Single use · expires {new Date(pairing.expires_at).toLocaleTimeString("en-US")}</small>
                  {pairingStatus ? <small>{pairingStatus}</small> : null}
                </div>
              </>
            ) : <p className="server-manager-hint" role={pairingStatus ? "status" : undefined}>{pairingStatus ?? "Paste this credential into the Remote Control Web Dashboard. The browser exchanges its encryption key during pairing."}</p>}
          </section>

          <section className="remote-control-section" aria-labelledby="remote-connect-title">
            <h3 id="remote-connect-title">Control another Choruz</h3>
            <p className="server-manager-hint">Paste the credential shown by Choruz on the other computer. The encrypted remote workspace opens in this browser.</p>
            <form className="remote-control-connect-form" onSubmit={(event) => {
              event.preventDefault();
              connectToRemoteHost();
            }}>
              <input
                className="remote-control-code-input"
                aria-label="Other computer pairing credential"
                autoComplete="off"
                placeholder="v1.…"
                maxLength={64}
                value={remoteCredential}
                onChange={(event) => setRemoteCredential(event.target.value)}
              />
              <button
                type="submit"
                className="server-manager-btn server-manager-btn--primary"
                disabled={!settings.gateway_url || !/^v1\.[A-Za-z0-9_-]{22}\.[A-Za-z0-9_-]{22}$/u.test(remoteCredential.trim())}
              >
                Connect
              </button>
            </form>
          </section>

          <section className="remote-control-section" aria-labelledby="remote-devices-title">
            <h3 id="remote-devices-title">Paired devices</h3>
            {devices.length === 0 ? <p className="server-manager-hint">No browsers paired yet.</p> : devices.map((device) => (
              <div className="remote-control-device" key={device.id}>
                <span><strong>{device.name}</strong><small>{device.last_seen_at ? `Last seen ${new Date(device.last_seen_at).toLocaleString("en-US")}` : "Not connected yet"}</small></span>
                <button className="server-manager-btn server-manager-btn--danger" onClick={async () => {
                  setError(null);
                  try {
                    await revokeRemoteControlDevice(sessionToken, device.id);
                    await load();
                  } catch (reason) {
                    setError(reason instanceof Error ? reason.message : String(reason));
                  }
                }}>Revoke</button>
              </div>
            ))}
          </section>

        </>
      )}
    </Modal>
  );
}
