import {
  createPairingKey,
  createPairingNonce,
  decryptWithSessionKey,
  derivePairingCommitment,
  derivePairingProof,
  derivePairingSecret,
  exportPairingPublicKey,
} from "./remote-control-crypto";
import { gatewayConnectUrl, type RemoteCredentials, type SocketFactory } from "./relay-session";
import { parsePairingCredential } from "./remote-control";

// ---------------------------------------------------------------------------
// The device side of the committed ECDH pairing handshake. The host side is
// `components/runtime/remote-control-manager.tsx`; the protocol is described
// in docs/operations/remote-control.md. Both browsers commit to their
// ephemeral keys, reveal them, derive the same secret, and the host sends
// device credentials encrypted with that secret.
// ---------------------------------------------------------------------------

export type PairingOptions = {
  gatewayUrl: string;
  /** The single-use credential shown on the host. */
  credential: string;
  deviceName: string;
  createSocket?: SocketFactory;
  timeoutMs?: number;
  /** Aborting closes the pairing socket and rejects. */
  signal?: AbortSignal;
};

const HANDSHAKE_TIMEOUT_MS = 60_000;
const SOCKET_CLOSING = 2;

export async function pairWithHost(options: PairingOptions): Promise<RemoteCredentials> {
  const credential = parsePairingCredential(options.credential);
  if (options.signal?.aborted) throw new Error("Pairing cancelled.");
  const createSocket = options.createSocket ?? ((url) => new WebSocket(url));
  const keys = await createPairingKey();
  const devicePublicKey = await exportPairingPublicKey(keys.publicKey);
  const deviceNonce = createPairingNonce();
  const deviceCommitment = await derivePairingCommitment(devicePublicKey, deviceNonce);
  const socket = createSocket(gatewayConnectUrl(options.gatewayUrl, {
    pairing_id: credential.id,
    role: "pair_client",
  }));

  return new Promise<RemoteCredentials>((resolve, reject) => {
    let hostCommitment: string | null = null;
    let pairingSecret: string | null = null;
    let incoming = Promise.resolve();
    let settled = false;
    const timer = setTimeout(
      () => fail(new Error("Pairing handshake timed out.")),
      options.timeoutMs ?? HANDSHAKE_TIMEOUT_MS,
    );
    const onAbort = () => fail(new Error("Pairing cancelled."));
    const fail = (error: Error) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      options.signal?.removeEventListener("abort", onAbort);
      reject(error);
      if (socket.readyState < SOCKET_CLOSING) socket.close(4002, "Pairing failed");
    };
    options.signal?.addEventListener("abort", onAbort, { once: true });
    socket.onerror = () => fail(new Error("Pairing credential is invalid, expired, or not ready."));
    socket.onclose = (event) => fail(new Error(event.reason || "Pairing connection closed."));
    socket.onopen = () => {
      socket.send(JSON.stringify({
        kind: "pair.commit",
        device_name: options.deviceName.trim() || "Choruz browser",
        device_commitment: deviceCommitment,
      }));
    };
    socket.onmessage = (event) => {
      incoming = incoming.then(async () => {
        try {
          const value = JSON.parse(String(event.data)) as Record<string, unknown>;
          if (value.type === "gateway.ready") return;
          if (value.kind === "pair.commit") {
            if (typeof value.host_commitment !== "string" || hostCommitment) {
              throw new Error("Invalid host pairing commitment.");
            }
            hostCommitment = value.host_commitment;
            socket.send(JSON.stringify({
              kind: "pair.reveal",
              device_public_key: devicePublicKey,
              device_nonce: deviceNonce,
            }));
            return;
          }
          if (value.kind === "pair.reveal") {
            const { host_public_key: hostPublicKey, host_nonce: hostNonce, host_proof: hostProof } = value;
            if (
              !hostCommitment
              || typeof hostPublicKey !== "string"
              || typeof hostNonce !== "string"
              || typeof hostProof !== "string"
              || await derivePairingCommitment(hostPublicKey, hostNonce) !== hostCommitment
            ) {
              throw new Error("Host pairing commitment did not match.");
            }
            pairingSecret = await derivePairingSecret(keys.privateKey, hostPublicKey, credential.secret);
            if (await derivePairingProof(pairingSecret, "host", hostPublicKey, devicePublicKey) !== hostProof) {
              throw new Error("Host did not prove possession of the pairing credential.");
            }
            socket.send(JSON.stringify({
              kind: "pair.proof",
              device_proof: await derivePairingProof(
                pairingSecret,
                "device",
                hostPublicKey,
                devicePublicKey,
              ),
            }));
            return;
          }
          if (value.kind !== "pair.complete") return;
          if (!pairingSecret) throw new Error("Pairing transcript was not verified.");
          if (typeof value.iv !== "string" || typeof value.ciphertext !== "string") {
            throw new Error("Pairing completion was not encrypted.");
          }
          const credentials = await decryptWithSessionKey<Record<string, unknown>>(
            pairingSecret, value.iv, value.ciphertext,
          );
          const deviceId = credentials.device_id;
          const gatewayUrl = credentials.gateway_url;
          const gatewayTicket = credentials.gateway_ticket;
          if (
            typeof deviceId !== "string"
            || typeof gatewayUrl !== "string"
            || typeof gatewayTicket !== "string"
          ) {
            throw new Error("Pairing did not return device credentials.");
          }
          if (settled) return;
          settled = true;
          clearTimeout(timer);
          options.signal?.removeEventListener("abort", onAbort);
          socket.close(1000, "Pairing complete");
          resolve({
            device_id: deviceId,
            gateway_url: gatewayUrl,
            gateway_ticket: gatewayTicket,
            session_key: typeof credentials.session_key === "string"
              ? credentials.session_key
              : pairingSecret,
          });
        } catch (error) {
          fail(error instanceof Error ? error : new Error(String(error)));
        }
      });
    };
  });
}

// ---------------------------------------------------------------------------
// Credentials persist per gateway origin so the browser reconnects without
// pairing again. Revocation clears them (the session reports `revoked`).
// ---------------------------------------------------------------------------

const STORAGE_PREFIX = "choruz.remote-control.credentials:";

export function credentialsStorageKey(gatewayUrl: string): string {
  return `${STORAGE_PREFIX}${new URL(gatewayUrl).origin}`;
}

export function loadRemoteCredentials(gatewayUrl: string): RemoteCredentials | null {
  try {
    const raw = globalThis.localStorage?.getItem(credentialsStorageKey(gatewayUrl));
    if (!raw) return null;
    const value = JSON.parse(raw) as Partial<RemoteCredentials>;
    if (
      typeof value.device_id !== "string"
      || typeof value.gateway_url !== "string"
      || typeof value.gateway_ticket !== "string"
      || typeof value.session_key !== "string"
    ) {
      return null;
    }
    return {
      device_id: value.device_id,
      gateway_url: value.gateway_url,
      gateway_ticket: value.gateway_ticket,
      session_key: value.session_key,
    };
  } catch {
    return null;
  }
}

export function storeRemoteCredentials(gatewayUrl: string, credentials: RemoteCredentials): void {
  try {
    globalThis.localStorage?.setItem(credentialsStorageKey(gatewayUrl), JSON.stringify(credentials));
  } catch {
    // Storage may be unavailable (private mode); the session still works until reload.
  }
}

export function clearRemoteCredentials(gatewayUrl: string): void {
  try {
    globalThis.localStorage?.removeItem(credentialsStorageKey(gatewayUrl));
  } catch {
    // Nothing to clear.
  }
}
