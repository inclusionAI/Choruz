"use client";

import { useCallback, useEffect, useRef, useState } from "react";

import { fetchDashboardBootstrap } from "../../lib/api/choruz-api";
import {
  dashboardSnapshotFromBootstrap,
  type DashboardSnapshotProps,
} from "../../lib/api/dashboard-snapshot";
import { setActiveTransport } from "../../lib/api/transport";
import {
  clearRemoteCredentials,
  loadRemoteCredentials,
  pairWithHost,
  storeRemoteCredentials,
} from "../../lib/remote/relay-pairing";
import {
  RelaySession,
  type RelayStatus,
  type RemoteCredentials,
} from "../../lib/remote/relay-session";
import { parsePairingCredential } from "../../lib/remote/remote-control";
import { createRelayTransport, type RelayTransport } from "../../lib/remote/relay-transport";
import { ChatApp } from "../chat/chat-app";

// ---------------------------------------------------------------------------
// The remote dashboard: pair with (or reconnect to) a Choruz host through the
// Cloud Gateway, install the relay transport, fetch the bootstrap through it
// and render the same ChatApp the host's own browser renders.
// ---------------------------------------------------------------------------

/** The bearer the dashboard's client calls carry over the relay. The host
 *  bridge replaces it with a token it issues for the paired principal, so the
 *  value itself never reaches an API. */
export const RELAY_SESSION_TOKEN = "relay";

const CONNECT_TIMEOUT_MS = 30_000;

export type RemoteEntryParams = {
  gatewayUrl: string;
  credential: string;
  deviceName: string;
};

/** The launch credential lives in the URL fragment so it is never sent to the
 *  dashboard or gateway server in an HTTP request. */
export function parseRemoteEntry(search: string, hash = ""): RemoteEntryParams {
  const params = new URLSearchParams(search);
  const fragment = new URLSearchParams(hash.replace(/^#/u, ""));
  return {
    gatewayUrl: (params.get("gateway") ?? "").trim(),
    credential: (fragment.get("credential") ?? "").trim(),
    deviceName: (params.get("device_name") ?? "").trim(),
  };
}

export function defaultDeviceName(): string {
  if (typeof navigator === "undefined" || !navigator.platform) return "Choruz browser";
  return `${navigator.platform} browser`;
}

function statusLabel(status: RelayStatus): string {
  switch (status) {
    case "connected": return "Connected";
    case "waiting": return "Waiting for the host";
    case "connecting": return "Reconnecting";
    case "revoked": return "Access revoked";
    case "closed": return "Disconnected";
    default: return status;
  }
}

function waitForConnection(session: RelaySession, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (session.status === "connected") {
      resolve();
      return;
    }
    const timer = setTimeout(() => {
      stop();
      reject(new Error("The paired computer did not connect in time. Is Choruz running there?"));
    }, CONNECT_TIMEOUT_MS);
    const stop = () => {
      clearTimeout(timer);
      unsubscribe();
      signal.removeEventListener("abort", onAbort);
    };
    const onAbort = () => {
      stop();
      reject(new Error("Connection cancelled."));
    };
    const unsubscribe = session.onStatus((status) => {
      if (status === "connected") {
        stop();
        resolve();
      } else if (status === "revoked") {
        stop();
        reject(new Error("This browser's access was revoked on the host."));
      } else if (status === "closed") {
        stop();
        reject(new Error("Connection closed."));
      }
    });
    signal.addEventListener("abort", onAbort, { once: true });
  });
}

type Phase =
  | { kind: "form" }
  | { kind: "pairing" }
  | { kind: "connecting" }
  | { kind: "ready"; props: DashboardSnapshotProps };

export function RemoteDashboard() {
  const [gatewayUrl, setGatewayUrl] = useState("");
  const [credential, setCredential] = useState("");
  const [deviceName, setDeviceName] = useState("");
  const [hasStoredPairing, setHasStoredPairing] = useState(false);
  const [phase, setPhase] = useState<Phase>({ kind: "form" });
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<RelayStatus>("closed");
  const sessionRef = useRef<RelaySession | null>(null);
  const transportRef = useRef<RelayTransport | null>(null);

  const disconnect = useCallback(() => {
    transportRef.current?.dispose();
    transportRef.current = null;
    sessionRef.current?.close();
    sessionRef.current = null;
    setActiveTransport(null);
  }, []);

  const refreshStoredPairing = useCallback((url: string) => {
    try {
      setHasStoredPairing(Boolean(url && loadRemoteCredentials(url)));
    } catch {
      setHasStoredPairing(false);
    }
  }, []);

  const connect = useCallback(async (credentials: RemoteCredentials, signal: AbortSignal) => {
    disconnect();
    setError(null);
    setPhase({ kind: "connecting" });
    const session = new RelaySession(credentials);
    const transport = createRelayTransport(session);
    sessionRef.current = session;
    transportRef.current = transport;
    setActiveTransport(transport);
    session.onStatus((next) => {
      if (sessionRef.current !== session) return;
      setStatus(next);
      if (next === "revoked") {
        clearRemoteCredentials(credentials.gateway_url);
        refreshStoredPairing(credentials.gateway_url);
        disconnect();
        setPhase({ kind: "form" });
        setError("This browser's access was revoked on the host. Pair again to reconnect.");
      }
    });
    session.start();
    try {
      await waitForConnection(session, signal);
      const bootstrap = await fetchDashboardBootstrap(RELAY_SESSION_TOKEN, { limit: 100 });
      if (sessionRef.current !== session || signal.aborted) return;
      setPhase({ kind: "ready", props: dashboardSnapshotFromBootstrap(bootstrap) });
    } catch (reason) {
      if (sessionRef.current !== session || signal.aborted) return;
      disconnect();
      setPhase({ kind: "form" });
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  }, [disconnect, refreshStoredPairing]);

  const pair = useCallback(async (params: RemoteEntryParams, signal: AbortSignal) => {
    setError(null);
    setPhase({ kind: "pairing" });
    try {
      const credentials = await pairWithHost({
        gatewayUrl: params.gatewayUrl,
        credential: params.credential,
        deviceName: params.deviceName,
        signal,
      });
      if (signal.aborted) return;
      storeRemoteCredentials(credentials.gateway_url, credentials);
      refreshStoredPairing(credentials.gateway_url);
      await connect(credentials, signal);
    } catch (reason) {
      if (signal.aborted) return;
      setPhase({ kind: "form" });
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  }, [connect, refreshStoredPairing]);

  useEffect(() => {
    const controller = new AbortController();
    const entry = parseRemoteEntry(window.location.search, window.location.hash);
    const initialDeviceName = entry.deviceName || defaultDeviceName();
    setGatewayUrl(entry.gatewayUrl);
    setCredential(entry.credential);
    if (entry.credential) window.history.replaceState(null, "", window.location.search);
    setDeviceName(initialDeviceName);
    refreshStoredPairing(entry.gatewayUrl);
    if (entry.gatewayUrl) {
      let stored: RemoteCredentials | null = null;
      try {
        stored = loadRemoteCredentials(entry.gatewayUrl);
      } catch {
        stored = null;
      }
      if (stored && !entry.credential) {
        void connect(stored, controller.signal);
      } else if (entry.credential) {
        void pair({ ...entry, deviceName: initialDeviceName }, controller.signal);
      }
    }
    return () => {
      controller.abort();
      disconnect();
    };
  }, [connect, disconnect, pair, refreshStoredPairing]);

  const submitRef = useRef<AbortController | null>(null);
  const submit = () => {
    submitRef.current?.abort();
    const controller = new AbortController();
    submitRef.current = controller;
    let normalized: string;
    try {
      normalized = parsePairingCredential(credential).value;
      new URL(gatewayUrl);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Enter the Cloud Gateway URL.");
      return;
    }
    void pair({ gatewayUrl: gatewayUrl.trim(), credential: normalized, deviceName }, controller.signal);
  };

  const reconnectStored = () => {
    submitRef.current?.abort();
    const controller = new AbortController();
    submitRef.current = controller;
    const stored = loadRemoteCredentials(gatewayUrl);
    if (!stored) {
      setHasStoredPairing(false);
      return;
    }
    void connect(stored, controller.signal);
  };

  const forget = () => {
    submitRef.current?.abort();
    disconnect();
    clearRemoteCredentials(gatewayUrl);
    setHasStoredPairing(false);
    setPhase({ kind: "form" });
    setError(null);
  };

  if (phase.kind === "ready") {
    return (
      <div className="remote-dashboard">
        <ChatApp
          initialSnapshot={phase.props.initialSnapshot}
          sessionToken={RELAY_SESSION_TOKEN}
          runtimeBindings={phase.props.runtimeBindings}
          initialCompanies={phase.props.initialCompanies}
          initialSyncCursor={phase.props.initialSyncCursor}
          initialBootstrapNextCursor={phase.props.initialBootstrapNextCursor}
          initialBootstrapHasMore={phase.props.initialBootstrapHasMore}
        />
        <div className="remote-dashboard-status" role="status" aria-live="polite">
          <span className={`remote-dashboard-dot remote-dashboard-dot--${status}`} aria-hidden="true" />
          <span>Remote · {statusLabel(status)}</span>
          <button type="button" className="remote-dashboard-link" onClick={forget}>
            Disconnect
          </button>
        </div>
      </div>
    );
  }

  const busy = phase.kind === "pairing" || phase.kind === "connecting";
  let credentialValid = false;
  try { parsePairingCredential(credential); credentialValid = true; } catch { /* incomplete input */ }

  return (
    <main className="remote-entry">
      <form
        className="remote-entry-card"
        onSubmit={(event) => {
          event.preventDefault();
          submit();
        }}
      >
        <p className="remote-entry-eyebrow">Choruz Remote</p>
        <h1>Open your Choruz from here</h1>
        <p className="remote-entry-hint">
          Paste the credential shown by Choruz on the computer you want to control.
          Everything you see and send is end-to-end encrypted between this browser and that computer.
        </p>
        <label className="remote-entry-field">
          <span>Cloud Gateway URL</span>
          <input
            type="url"
            aria-label="Cloud Gateway URL"
            value={gatewayUrl}
            disabled={busy}
            onChange={(event) => {
              setGatewayUrl(event.target.value);
              refreshStoredPairing(event.target.value.trim());
            }}
            placeholder="https://choruz-remote-control-gateway.example.workers.dev"
            required
          />
        </label>
        <label className="remote-entry-field">
          <span>Pairing credential</span>
          <input
            className="remote-entry-code"
            aria-label="Pairing credential"
            autoComplete="off"
            placeholder="v1.…"
            maxLength={64}
            value={credential}
            disabled={busy}
            onChange={(event) => setCredential(event.target.value)}
          />
        </label>
        <label className="remote-entry-field">
          <span>This device's name</span>
          <input
            type="text"
            aria-label="Device name"
            value={deviceName}
            disabled={busy}
            onChange={(event) => setDeviceName(event.target.value)}
            maxLength={64}
          />
        </label>
        <div className="remote-entry-actions">
          <button
            type="submit"
            className="server-manager-btn server-manager-btn--primary"
            disabled={busy || !credentialValid || !gatewayUrl.trim()}
          >
            {phase.kind === "pairing" ? "Pairing…" : phase.kind === "connecting" ? "Connecting…" : "Connect"}
          </button>
          {hasStoredPairing && !busy ? (
            <>
              <button type="button" className="server-manager-btn" onClick={reconnectStored}>
                Reconnect
              </button>
              <button type="button" className="remote-dashboard-link" onClick={forget}>
                Forget this pairing
              </button>
            </>
          ) : null}
        </div>
        {phase.kind === "pairing" ? (
          <p className="remote-entry-status" role="status" aria-live="polite">
            Pairing with the host…
          </p>
        ) : null}
        {phase.kind === "connecting" ? (
          <p className="remote-entry-status" role="status" aria-live="polite">
            Connecting to the paired computer…
          </p>
        ) : null}
        {error ? <p className="remote-entry-error" role="alert">{error}</p> : null}
      </form>
    </main>
  );
}
