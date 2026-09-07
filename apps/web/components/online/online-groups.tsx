"use client";

import { useEffect, useState } from "react";
import { apiFetch } from "../../lib/api/choruz-api";
import type { Conversation, Principal } from "../../lib/api/choruz-types";
import { onlineConversationId, type OnlineGroup as Link } from "./online-conversation";


export function OnlineGroups({ sessionToken, principal, conversations, onOpenConversation }: { sessionToken: string; principal: Principal; conversations: Conversation[]; onOpenConversation: (id: string) => void }) {
  const [links, setLinks] = useState<Link[]>([]);
  const [connection, setConnection] = useState("connecting");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [invitation, setInvitation] = useState("");
  const [generated, setGenerated] = useState("");
  const owned = conversations.filter(c => c.conversation_type === "group" && c.creator_id === principal.id);
  const [group, setGroup] = useState(owned[0]?.id ?? "");
  useEffect(() => {
    const abort = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    const load = async () => {
      try {
        const result = await apiFetch<{ groups: Link[]; connection: string }>("/v1/online/groups", sessionToken, { signal: abort.signal });
        if (!abort.signal.aborted) { setLinks(result.groups); setConnection(result.connection); }
      } catch (error) { if (!abort.signal.aborted) setError(error instanceof Error ? error.message : "Could not load Online groups"); }
      finally { if (!abort.signal.aborted) timer = setTimeout(load, 3000); }
    };
    void load();
    return () => { abort.abort(); clearTimeout(timer); };
  }, [sessionToken]);
  const act = async (operation: () => Promise<void>) => {
    setBusy(true); setError(null);
    try { await operation(); }
    catch (error) { setError(error instanceof Error ? error.message : "Online operation failed"); }
    finally { setBusy(false); }
  };
  return <section className="modal-form" aria-label="Online groups">
    <p aria-live="polite">Connection: {connection}</p>
    {connection !== "connected" && <p className="field-hint">Messages resume when this device reconnects. Keep Choruz running; the browser may be closed.</p>}
    {error && <p className="modal-form-error" role="alert">{error}</p>}
    <h3>Invite someone to your group</h3>
    {owned.length ? <>
      <label>Group to share<select value={group} onChange={event => { setGroup(event.target.value); setGenerated(""); }} disabled={busy}>{owned.map(c => <option key={c.id} value={c.id}>{c.name}</option>)}</select></label>
      <button className="btn-secondary" type="button" disabled={busy || !group} onClick={() => void act(async () => {
        const result = await apiFetch<{ invitation: string }>("/v1/online/groups/invite", sessionToken, { method: "POST", body: JSON.stringify({ conversation_id: group }) });
        setGenerated(result.invitation);
      })}>Create group invitation</button>
      {generated && <div><label>Group invitation<textarea aria-label="Group invitation" aria-describedby="online-invitation-hint" readOnly value={generated} onFocus={event => event.target.select()} /></label><p id="online-invitation-hint" className="field-hint">Share privately. One person can accept within 24 hours and read this group's history. No device-control access is granted.</p></div>}
    </> : <p className="field-hint">Create a group first, then invite another person here.</p>}
    <h3>Join a group</h3>
    <label>Invitation to join<textarea value={invitation} onChange={event => setInvitation(event.target.value)} maxLength={4096} disabled={busy} autoComplete="off" /></label>
    <button className="btn-primary" type="button" disabled={busy || !invitation.trim()} onClick={() => void act(async () => {
      const result = await apiFetch<Link>("/v1/online/groups/join", sessionToken, { method: "POST", body: JSON.stringify({ invitation }) });
      setInvitation(""); onOpenConversation(onlineConversationId(result.id));
    })}>Join group</button>
    <h3>Shared groups</h3>
    {!links.length && <p className="field-hint">No group invitations or joined groups yet.</p>}
    {links.map(link => <div className="modal-form" key={link.id}>
      <div><strong>{link.name}</strong> · {link.role === "host" ? "Your invitation" : "Joined group"} · {link.status}</div>
      <div className="modal-actions">
        <button className="btn-secondary" type="button" onClick={() => link.role === "host" ? onOpenConversation(link.conversation_id) : onOpenConversation(onlineConversationId(link.id))}>Open group</button>
        {link.status !== "revoked" && <button className="btn-secondary" type="button" disabled={busy} onClick={() => void act(async () => {
          await apiFetch(`/v1/online/groups/${link.id}`, sessionToken, { method: "DELETE" });
          setLinks(current => current.map(item => item.id === link.id ? { ...item, status: "revoked" } : item));
        })}>{link.role === "host" ? "Revoke invitation / remove member" : "Leave group"}</button>}
      </div>
    </div>)}
  </section>;
}
