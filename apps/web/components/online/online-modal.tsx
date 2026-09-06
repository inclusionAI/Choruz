"use client";

import { useEffect, useState } from "react";
import { apiFetch } from "../../lib/api/choruz-api";
import { Modal } from "../ui/modal";
import type { Conversation, Principal } from "../../lib/api/choruz-types";
import { OnlineGroups } from "./online-groups";

type Identity = { state: "signed_out" | "signed_in" | "reauth_required"; display_name?: string; device_id?: string };

export function OnlineModal({ sessionToken, onClose, principal, conversations, onOpenConversation }: { sessionToken: string; onClose: () => void; principal: Principal; conversations: Conversation[]; onOpenConversation: (id: string) => void }) {
  const [identity, setIdentity] = useState<Identity | null>(null);
  const [signup, setSignup] = useState(false);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    const controller = new AbortController();
    setError(null);
    apiFetch<Identity>("/v1/online/session", sessionToken, { signal: controller.signal }).then(setIdentity).catch(error => {
      if (!controller.signal.aborted) setError(error instanceof Error ? error.message : "Could not load Online account");
    });
    return () => controller.abort();
  }, [sessionToken, retry]);

  const signOut = async () => {
    setBusy(true); setError(null);
    try {
      await apiFetch("/v1/online/session", sessionToken, { method: "DELETE" });
      setIdentity({ state: "signed_out" });
    } catch (error) { setError(error instanceof Error ? error.message : "Could not sign out"); }
    finally { setBusy(false); }
  };

  return <Modal title="Online" className="online-modal" layout="flush" description="Use your Choruz account to collaborate with other people. Local use does not require sign-in." onClose={onClose} closeDisabled={busy}><div className="online-modal-body">
    {error && <p className="modal-form-error" role="alert">{error}</p>}
    {!identity ? <div aria-live="polite">
      {error ? <button type="button" className="btn-secondary" onClick={() => setRetry(value => value + 1)}>Retry</button> : <p>Checking Online account…</p>}
    </div> : identity.state !== "signed_out" ? <div className="modal-form">
      <p role="status">{identity.state === "signed_in" ? `Signed in as ${identity.display_name}` : "Your Online session expired. Sign out, then sign in again."}</p>
      <p className="field-hint">This account does not grant other people access to your device, private chats or Harness accounts.</p>
      {identity.state === "signed_in" && <OnlineGroups sessionToken={sessionToken} principal={principal} conversations={conversations} onOpenConversation={onOpenConversation} />}
      <div className="modal-actions"><button type="button" className="btn-secondary" disabled={busy} onClick={signOut}>{busy ? "Signing out…" : "Sign out of Online"}</button></div>
    </div> : <form className="modal-form" onSubmit={async event => {
      event.preventDefault(); setBusy(true); setError(null);
      try {
        const result = await apiFetch<Identity>(`/v1/online/${signup ? "sign-up" : "sign-in"}`, sessionToken, {
          method: "POST", headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ email, password, ...(signup ? { name } : {}) }),
        });
        setIdentity(result);
      } catch (error) { setError(error instanceof Error ? error.message : "Online sign-in failed"); }
      finally { setPassword(""); setBusy(false); }
    }}>
      {signup && <label>Display name<input value={name} onChange={event => setName(event.target.value)} required maxLength={80} autoComplete="nickname" disabled={busy} /></label>}
      <label>Email<input type="email" value={email} onChange={event => setEmail(event.target.value)} required maxLength={254} autoComplete="username" disabled={busy} /></label>
      <label>Password<input type="password" value={password} onChange={event => setPassword(event.target.value)} required minLength={signup ? 12 : undefined} maxLength={128} autoComplete={signup ? "new-password" : "current-password"} disabled={busy} /></label>
      {signup && <p className="field-hint">Use 12–128 characters. This is a Choruz account, not your Claude Code or Codex login.</p>}
      <div className="modal-actions">
        <button type="button" className="btn-secondary" disabled={busy} onClick={() => { setSignup(value => !value); setPassword(""); setError(null); }}>{signup ? "Use an existing account" : "Create account"}</button>
        <button type="submit" className="btn-primary" disabled={busy}>{busy ? "Connecting…" : signup ? "Create account and sign in" : "Sign in"}</button>
      </div>
    </form>}
  </div></Modal>;
}
