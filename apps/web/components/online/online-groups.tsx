"use client";

import { useEffect, useRef, useState } from "react";
import { apiFetch } from "../../lib/api/choruz-api";
import type { Conversation, Principal, ChatMessage } from "../../lib/api/choruz-types";
import { MessageBubble } from "../chat/message-bubble";

type Link = { id: string; role: "host" | "guest"; name: string; status: "pending" | "active" | "revoked"; conversation_id: string };
type SharedMessage = { id: string; seq: number; sender_id: string; sender_name: string; agent: boolean; own: boolean; content: string; created_at: string };
type Page = { messages: SharedMessage[]; pending: { id: string; content: string }[]; status: Link["status"] };
const idle = () => {};

export function OnlineGroups({ sessionToken, principal, conversations, onOpenConversation }: { sessionToken: string; principal: Principal; conversations: Conversation[]; onOpenConversation: (id: string) => void }) {
  const [links, setLinks] = useState<Link[]>([]);
  const [connection, setConnection] = useState("connecting");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [invitation, setInvitation] = useState("");
  const [generated, setGenerated] = useState("");
  const [selected, setSelected] = useState<Link | null>(null);
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
  if (selected) return <OnlineGroupChat link={selected} principal={principal} sessionToken={sessionToken} onBack={() => setSelected(null)} />;
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
      setInvitation(""); setSelected(result);
    })}>Join group</button>
    <h3>Shared groups</h3>
    {!links.length && <p className="field-hint">No group invitations or joined groups yet.</p>}
    {links.map(link => <div className="modal-form" key={link.id}>
      <div><strong>{link.name}</strong> · {link.role === "host" ? "Your invitation" : "Joined group"} · {link.status}</div>
      <div className="modal-actions">
        <button className="btn-secondary" type="button" onClick={() => link.role === "host" ? onOpenConversation(link.conversation_id) : setSelected(link)}>Open group</button>
        {link.status !== "revoked" && <button className="btn-secondary" type="button" disabled={busy} onClick={() => void act(async () => {
          await apiFetch(`/v1/online/groups/${link.id}`, sessionToken, { method: "DELETE" });
          setLinks(current => current.map(item => item.id === link.id ? { ...item, status: "revoked" } : item));
        })}>{link.role === "host" ? "Revoke invitation / remove member" : "Leave group"}</button>}
      </div>
    </div>)}
  </section>;
}

function OnlineGroupChat({ link, principal, sessionToken, onBack }: { link: Link; principal: Principal; sessionToken: string; onBack: () => void }) {
  const [messages, setMessages] = useState<SharedMessage[]>([]);
  const [pending, setPending] = useState<Page["pending"]>([]);
  const [status, setStatus] = useState(link.status);
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const retryId = useRef<string | null>(null);
  useEffect(() => {
    const abort = new AbortController(); let cursor = 0; let timer: ReturnType<typeof setTimeout>;
    const load = async () => {
      try {
        const result = await apiFetch<Page>(`/v1/online/groups/${link.id}/messages?after_seq=${cursor}`, sessionToken, { signal: abort.signal });
        if (!abort.signal.aborted) {
          setStatus(result.status); setPending(result.pending);
          setMessages(current => { const byId = new Map(current.map(m => [m.id, m])); for (const m of result.messages) byId.set(m.id, m); return [...byId.values()].sort((a, b) => a.seq - b.seq); });
          if (result.messages.length) cursor = result.messages.at(-1)!.seq;
        }
      } catch (error) { if (!abort.signal.aborted) setError(error instanceof Error ? error.message : "Could not load messages"); }
      finally { if (!abort.signal.aborted) timer = setTimeout(load, 1500); }
    };
    void load(); return () => { abort.abort(); clearTimeout(timer); };
  }, [link.id, sessionToken]);
  const chat: ChatMessage[] = messages.map(m => ({ id: m.id, workspace_id: principal.workspace_id, conversation_id: link.conversation_id, sender_id: m.own ? principal.id : m.sender_id, content: m.content, content_type: "text", metadata: {}, server_seq: m.seq, idempotency_key: m.id, created_at: m.created_at, edited_at: null, edited_by: null }));
  const people = messages.filter(m => !m.own).map(m => ({ ...principal, id: m.sender_id, name: m.sender_name, principal_type: m.agent ? "agent" as const : "human" as const }));
  return <section className="modal-form" aria-label="Shared group chat">
    <button className="btn-secondary" type="button" onClick={onBack}>Back to Online groups</button>
    <h3>{link.name}</h3>
    <p className="field-hint">Shared text and Agent replies. Files, terminals and private chats remain on the owner's device.</p>
    {status === "pending" && <p aria-live="polite">Waiting for the group owner's device…</p>}
    {status === "revoked" && <p>You left this group or your invitation was revoked. Saved history remains readable.</p>}
    {error && <p className="modal-form-error" role="alert">{error}</p>}
    <div className="online-group-history" aria-label="Shared messages">
      {chat.map((msg, idx) => <MessageBubble key={msg.id} msg={msg} idx={idx} allMsgs={chat} principal={principal} principals={people} isTerminalChat={false} scrollToMessage={idle} touchActiveId={null} onTouchStart={idle} onTouchEnd={idle} onTouchMove={idle} />)}
      {pending.map(m => <p key={m.id}>{m.content} <small>— queued for delivery</small></p>)}
      {!chat.length && !pending.length && <p className="field-hint">No shared messages received yet.</p>}
    </div>
    <form className="modal-form" onSubmit={async event => {
      event.preventDefault(); setBusy(true); setError(null);
      retryId.current ??= crypto.randomUUID();
      try {
        await apiFetch(`/v1/online/groups/${link.id}/messages`, sessionToken, { method: "POST", body: JSON.stringify({ id: retryId.current, content: text }) });
        setPending(current => [...current, { id: retryId.current!, content: text }]); setText(""); retryId.current = null;
      } catch (error) { setError(error instanceof Error ? error.message : "Could not queue message"); }
      finally { setBusy(false); }
    }}>
      <label>Message to shared group<textarea value={text} onChange={event => { setText(event.target.value); retryId.current = null; }} maxLength={32000} disabled={busy || status !== "active"} /></label>
      <button type="submit" className="btn-primary" disabled={busy || !text.trim() || status !== "active"}>{busy ? "Queueing…" : "Send to group"}</button>
    </form>
  </section>;
}
