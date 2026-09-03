"use client";

import { Check, Copy, Laptop, Pencil, Plus, Server, Trash2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Spinner } from "../ui/spinner";

import {
  createRuntimeHostPairing,
  listRuntimeHosts,
  renameRuntimeHost,
  revokeRuntimeHost,
} from "../../lib/api/choruz-api";
import type { RuntimeBindingInfo } from "../../lib/api/choruz-types";
import type { RuntimeHost, RuntimeHostPairing } from "../../lib/remote/remote-control";
import { Modal } from "../ui/modal";

type Props = {
  sessionToken: string;
  companyId: string;
  companyName: string;
  runtimeBindings: RuntimeBindingInfo[];
  onHostsChanged?: (hosts: RuntimeHost[]) => void;
  onClose: () => void;
};

function lastSeenLabel(host: RuntimeHost): string {
  if (host.status === "online") return "Online now";
  if (!host.last_seen_at) return "Not connected yet";
  return `Last seen ${new Date(host.last_seen_at).toLocaleString("en-US")}`;
}

export function RuntimeHostManager({
  sessionToken,
  companyId,
  companyName,
  runtimeBindings,
  onHostsChanged,
  onClose,
}: Props) {
  const [hosts, setHosts] = useState<RuntimeHost[]>([]);
  const [pairing, setPairing] = useState<RuntimeHostPairing | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");

  const companyBindings = useMemo(
    () => runtimeBindings.filter((binding) => binding.workspace_id === companyId),
    [companyId, runtimeBindings],
  );
  const localAgentCount = useMemo(
    () => new Set(
      companyBindings
        .filter((binding) => !binding.runtime_host_id && binding.state !== "disabled")
        .map((binding) => binding.agent_principal_id),
    ).size,
    [companyBindings],
  );
  const agentCountByHost = useMemo(() => {
    const ids = new Map<string, Set<string>>();
    for (const binding of companyBindings) {
      if (!binding.runtime_host_id || binding.state === "disabled") continue;
      const agents = ids.get(binding.runtime_host_id) ?? new Set<string>();
      agents.add(binding.agent_principal_id);
      ids.set(binding.runtime_host_id, agents);
    }
    return new Map([...ids].map(([hostId, agents]) => [hostId, agents.size]));
  }, [companyBindings]);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const nextHosts = await listRuntimeHosts(sessionToken, companyId);
      setHosts(nextHosts);
      onHostsChanged?.(nextHosts);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setLoading(false);
    }
  }, [companyId, onHostsChanged, sessionToken]);

  useEffect(() => { void load(); }, [load]);

  const beginPairing = async () => {
    setSaving(true);
    setError(null);
    try {
      setPairing(await createRuntimeHostPairing(sessionToken, companyId));
      setCopied(false);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setSaving(false);
    }
  };

  const saveName = async (host: RuntimeHost) => {
    const name = renameValue.trim();
    if (!name || name === host.name) {
      setRenamingId(null);
      return;
    }
    setSaving(true);
    setError(null);
    try {
      const updated = await renameRuntimeHost(sessionToken, host.id, name);
      const next = hosts.map((item) => item.id === host.id ? updated : item);
      setHosts(next);
      onHostsChanged?.(next);
      setRenamingId(null);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setSaving(false);
    }
  };

  const removeHost = async (host: RuntimeHost) => {
    if (!window.confirm(`Disconnect ${host.name}? Its Agents will stop receiving new work.`)) return;
    setSaving(true);
    setError(null);
    try {
      await revokeRuntimeHost(sessionToken, host.id);
      const next = hosts.filter((item) => item.id !== host.id);
      setHosts(next);
      onHostsChanged?.(next);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal
        eyebrow={companyName}
        title="Machines"
      description="Choose where this Company's Agents run. Code and tools stay on the machine that owns each Agent."
      onClose={onClose}
      closeLabel="Close Machines"
      className="machines-card"
      layout="flush"
    >
      {error ? <p className="server-manager-error-block" role="alert">{error}</p> : null}

      <section className="machines-overview" aria-label="Machine summary">
        <div><strong>{hosts.length + 1}</strong><span>Machines</span></div>
        <div><strong>{companyBindings.length}</strong><span>Runtime bindings</span></div>
        <div><strong>{hosts.filter((host) => host.status === "online").length + 1}</strong><span>Online</span></div>
      </section>

      <div className="machines-toolbar">
        <div>
          <h3>Company compute</h3>
          <p>New Agents run on this computer unless you choose another machine.</p>
        </div>
        <button
          type="button"
          className="server-manager-btn server-manager-btn--primary machines-add"
          disabled={saving}
          onClick={() => void beginPairing()}
        >
          <Plus size={15} /> Add machine
        </button>
      </div>

      {pairing ? (
        <section className="machine-pairing" aria-labelledby="machine-pairing-title">
          <div>
            <span className="eyebrow">Connector code</span>
            <h3 id="machine-pairing-title">Connect another computer</h3>
            <p>Open Choruz on that computer, choose <strong>Connect to Company</strong>, then enter this one-time code.</p>
          </div>
          <div className="machine-pairing-code" role="status" aria-live="polite">
            <code>{pairing.code.slice(0, 4)} {pairing.code.slice(4)}</code>
            <button
              type="button"
              className="icon-button"
              aria-label="Copy connector code"
              onClick={async () => {
                await navigator.clipboard.writeText(pairing.code);
                setCopied(true);
              }}
            >
              {copied ? <Check size={16} /> : <Copy size={16} />}
            </button>
          </div>
          <small>Single use · expires {new Date(pairing.expires_at).toLocaleTimeString("en-US")}</small>
        </section>
      ) : null}

      <section className="machine-list" aria-busy={loading}>
        <article className="machine-card machine-card--local">
          <div className="machine-icon"><Laptop size={20} /></div>
          <div className="machine-copy">
            <div className="machine-title-row">
              <h3>This computer</h3>
              <span className="machine-pill machine-pill--local">Local</span>
            </div>
            <p><span className="runtime-host-status runtime-host-status--online" /> Online now</p>
            <small>{localAgentCount} {localAgentCount === 1 ? "Agent" : "Agents"}</small>
          </div>
        </article>

        {loading ? <p className="machines-loading"><Spinner label="Loading machines…" /></p> : null}
        {!loading && hosts.map((host) => {
          const count = agentCountByHost.get(host.id) ?? 0;
          return (
            <article className="machine-card" key={host.id}>
              <div className="machine-icon"><Server size={20} /></div>
              <div className="machine-copy">
                <div className="machine-title-row">
                  {renamingId === host.id ? (
                    <input
                      className="machine-rename-input"
                      aria-label={`Rename ${host.name}`}
                      autoFocus
                      value={renameValue}
                      maxLength={80}
                      onChange={(event) => setRenameValue(event.target.value)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter") { event.preventDefault(); void saveName(host); }
                        if (event.key === "Escape") setRenamingId(null);
                      }}
                      onBlur={() => void saveName(host)}
                    />
                  ) : <h3>{host.name}</h3>}
                  <span className={`machine-pill machine-pill--${host.status}`}>{host.status}</span>
                </div>
                <p><span className={`runtime-host-status runtime-host-status--${host.status}`} /> {lastSeenLabel(host)}</p>
                <small>{count} {count === 1 ? "Agent" : "Agents"}</small>
              </div>
              <div className="machine-actions">
                <button
                  type="button"
                  className="icon-button"
                  aria-label={`Rename ${host.name}`}
                  disabled={saving}
                  onClick={() => { setRenamingId(host.id); setRenameValue(host.name); }}
                ><Pencil size={15} /></button>
                <button
                  type="button"
                  className="icon-button icon-button--danger"
                  aria-label={`Disconnect ${host.name}`}
                  disabled={saving}
                  onClick={() => void removeHost(host)}
                ><Trash2 size={15} /></button>
              </div>
            </article>
          );
        })}
      </section>
    </Modal>
  );
}
