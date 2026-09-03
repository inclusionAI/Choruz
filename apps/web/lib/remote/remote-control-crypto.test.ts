import { describe, expect, it } from "vitest";

import {
  createPairingKey,
  createPairingNonce,
  decryptWithSessionKey,
  derivePairingCommitment,
  derivePairingProof,
  derivePairingSecret,
  encryptWithSessionKey,
  exportPairingPublicKey,
} from "./remote-control-crypto";

describe("remote-control E2E encryption", () => {
  it("derives the same session key in both paired browsers", async () => {
    const host = await createPairingKey();
    const remote = await createPairingKey();
    const credentialSecret = "BBBBBBBBBBBBBBBBBBBBBB";
    const hostPublic = await exportPairingPublicKey(host.publicKey);
    const remotePublic = await exportPairingPublicKey(remote.publicKey);
    const [hostSecret, remoteSecret] = await Promise.all([
      derivePairingSecret(host.privateKey, remotePublic, credentialSecret),
      derivePairingSecret(remote.privateKey, hostPublic, credentialSecret),
    ]);

    expect(hostSecret).toBe(remoteSecret);
    expect(hostSecret).toMatch(/^[A-Za-z0-9_-]{43}$/u);
    const remotePublicKey = await exportPairingPublicKey(remote.publicKey);
    const nonce = createPairingNonce();
    const commitment = await derivePairingCommitment(remotePublicKey, nonce);
    await expect(derivePairingCommitment(remotePublicKey, nonce)).resolves.toBe(commitment);
    await expect(derivePairingProof(hostSecret, "host", hostPublic, remotePublic))
      .resolves.toMatch(/^[A-Za-z0-9_-]{43}$/u);
  });

  it("binds the derived key to the credential secret", async () => {
    const host = await createPairingKey();
    const remote = await createPairingKey();
    const remotePublic = await exportPairingPublicKey(remote.publicKey);
    const [left, right] = await Promise.all([
      derivePairingSecret(host.privateKey, remotePublic, "AAAAAAAAAAAAAAAAAAAAAA"),
      derivePairingSecret(host.privateKey, remotePublic, "BBBBBBBBBBBBBBBBBBBBBB"),
    ]);
    expect(left).not.toBe(right);
  });

  it("round-trips an opaque control envelope", async () => {
    const key = "a".repeat(43);
    const payload = {
      kind: "message.send",
      payload: { conversation_id: "conversation-1", content: "hello" },
    };
    const encrypted = await encryptWithSessionKey(key, payload);

    expect(encrypted.ciphertext).not.toContain("hello");
    await expect(
      decryptWithSessionKey(key, encrypted.iv, encrypted.ciphertext),
    ).resolves.toEqual(payload);
  });
});
