"use client";

import { useEffect, useState } from "react";
import { apiFetch } from "../../lib/api/choruz-api";

type Agent = { id: string; name: string; status: "pending" | "active" | "removed" | "error" | null; error: string | null };

/** Membership is confirmed by the host, not by the local POST response. */
export function OnlineAgents({ linkId, sessionToken }: { linkId: string; sessionToken: string }) {
  const [agents, setAgents] = useState<Agent[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    const abort = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    const load = async () => {
      try {
        const result = await apiFetch<Agent[]>(`/v1/online/groups/${linkId}/agents`, sessionToken, { signal: abort.signal });
        if (!abort.signal.aborted) { setAgents(result); setError(null); }
      } catch (cause) {
        if (!abort.signal.aborted) setError(cause instanceof Error ? cause.message : "Could not load your Agents");
      } finally {
        if (!abort.signal.aborted) { setLoading(false); timer = setTimeout(load, 1500); }
      }
    };
    void load();
    return () => { abort.abort(); clearTimeout(timer); };
  }, [linkId, sessionToken, revision]);

  const change = async (agent: Agent, remove: boolean) => {
    setBusy(agent.id);
    setError(null);
    try {
      await apiFetch(`/v1/online/groups/${linkId}/agents${remove ? `/${agent.id}` : ""}`, sessionToken, {
        method: remove ? "DELETE" : "POST",
        ...(remove ? {} : { body: JSON.stringify({ agent_id: agent.id }) }),
      });
      setRevision(value => value + 1);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Could not change Agent membership");
    } finally { setBusy(null); }
  };

  return <section aria-label="Your Agents in this group" className="modal-form">
    <h3>Your Agents</h3>
    <p className="text-muted">Add an Agent from this device. It runs here and shares its group replies with the other members.</p>
    {loading && <p role="status">Loading your Agents…</p>}
    {error && <p role="alert">{error}</p>}
    {!loading && !error && agents.length === 0 && <p>No eligible Agents. Create an Agent on this device first.</p>}
    {agents.map(agent => <div key={agent.id} className="modal-form" data-agent-id={agent.id}>
      <strong>{agent.name}</strong>
      <span>{agent.status === "active" ? "In this group" : agent.status === "pending" ? "Waiting for the group owner's device…" : "Not shared"}</span>
      {agent.error && <p role="alert">{agent.error}</p>}
      <button type="button" className="btn btn-secondary" disabled={busy !== null}
        onClick={() => void change(agent, agent.status === "active" || agent.status === "pending")}>
        {busy === agent.id ? "Saving…" : agent.status === "active" || agent.status === "pending" ? `Remove ${agent.name}` : `Add ${agent.name}`}
      </button>
    </div>)}
  </section>;
}
