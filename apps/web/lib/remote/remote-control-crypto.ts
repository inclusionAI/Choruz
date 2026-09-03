const encoder = new TextEncoder();
const PAIRING_CONTEXT = encoder.encode("choruz.remote-control.pairing.v1");

export function bytesToBase64Url(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/u, "");
}

export function base64UrlToBytes(value: string): ArrayBuffer {
  const padded = value.replace(/-/g, "+").replace(/_/g, "/") + "===".slice((value.length + 3) % 4);
  const binary = atob(padded);
  return Uint8Array.from(binary, (character) => character.charCodeAt(0)).buffer;
}

export function createPairingNonce(): string {
  return bytesToBase64Url(crypto.getRandomValues(new Uint8Array(16)));
}

export async function derivePairingCommitment(
  publicKey: string,
  nonce: string,
): Promise<string> {
  const transcript = encoder.encode([
    "choruz.remote-control.commit.v1",
    publicKey,
    nonce,
  ].join("\0"));
  return bytesToBase64Url(new Uint8Array(await crypto.subtle.digest("SHA-256", transcript)));
}

export async function createPairingKey(): Promise<CryptoKeyPair> {
  return crypto.subtle.generateKey({ name: "ECDH", namedCurve: "P-256" }, true, ["deriveBits"]);
}

export async function exportPairingPublicKey(key: CryptoKey): Promise<string> {
  return JSON.stringify(await crypto.subtle.exportKey("jwk", key));
}

async function importPairingPublicKey(serialized: string): Promise<CryptoKey> {
  const parsed = JSON.parse(serialized) as JsonWebKey;
  return crypto.subtle.importKey(
    "jwk",
    parsed,
    { name: "ECDH", namedCurve: "P-256" },
    false,
    [],
  );
}

export async function derivePairingSecret(
  privateKey: CryptoKey,
  peerPublicKey: string,
  credentialSecret: string,
): Promise<string> {
  const bits = await crypto.subtle.deriveBits(
    { name: "ECDH", public: await importPairingPublicKey(peerPublicKey) },
    privateKey,
    256,
  );
  const inputKey = await crypto.subtle.importKey("raw", bits, "HKDF", false, ["deriveBits"]);
  const derived = await crypto.subtle.deriveBits(
    {
      name: "HKDF",
      hash: "SHA-256",
      salt: base64UrlToBytes(credentialSecret),
      info: PAIRING_CONTEXT,
    },
    inputKey,
    256,
  );
  return bytesToBase64Url(new Uint8Array(derived));
}

export async function derivePairingProof(
  pairingSecret: string,
  role: "host" | "device",
  hostPublicKey: string,
  devicePublicKey: string,
): Promise<string> {
  const key = await crypto.subtle.importKey(
    "raw",
    base64UrlToBytes(pairingSecret),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const proof = await crypto.subtle.sign(
    "HMAC",
    key,
    encoder.encode(["choruz.remote-control.proof.v1", role, hostPublicKey, devicePublicKey].join("\0")),
  );
  return bytesToBase64Url(new Uint8Array(proof));
}

async function importSessionKey(sessionKey: string): Promise<CryptoKey> {
  return crypto.subtle.importKey(
    "raw",
    base64UrlToBytes(sessionKey),
    { name: "AES-GCM" },
    false,
    ["encrypt", "decrypt"],
  );
}

export async function encryptWithSessionKey(
  sessionKey: string,
  payload: unknown,
): Promise<{ iv: string; ciphertext: string }> {
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const ciphertext = await crypto.subtle.encrypt(
    { name: "AES-GCM", iv },
    await importSessionKey(sessionKey),
    encoder.encode(JSON.stringify(payload)),
  );
  return {
    iv: bytesToBase64Url(iv),
    ciphertext: bytesToBase64Url(new Uint8Array(ciphertext)),
  };
}

export async function decryptWithSessionKey<T>(
  sessionKey: string,
  iv: string,
  ciphertext: string,
): Promise<T> {
  const plaintext = await crypto.subtle.decrypt(
    { name: "AES-GCM", iv: base64UrlToBytes(iv) },
    await importSessionKey(sessionKey),
    base64UrlToBytes(ciphertext),
  );
  return JSON.parse(new TextDecoder().decode(plaintext)) as T;
}
