"use client";

import { useEffect, useState } from "react";
import { apiFetch } from "../../lib/api/choruz-api";

type Settings = { search: boolean; automatic_trial: boolean; contribute: boolean };
type Record = {
  id: string;
  problem: { title: string; input_background: string; expected_behavior: string; bad_behavior: string; applicability: string };
  model: { observed: string | null; configured: string | null; harness: string; harness_version: string | null };
  kind: string;
  evidence_summary: string;
  solution: { id: string; based_on: string[]; instruction: string; applicability: string; team: { order: string; members: { name: string; prompt: string }[] } | null } | null;
};
type Library = {
  settings: Settings;
  local: { id: string; record: Record | null; state: string; url: string | null; error: string | null }[];
  community: Record[];
  counts: { encountered: number; applied: number; effective: number; ineffective: number; recurrence: number };
  publisher_configured: boolean;
  sync: { checked_at: string | null; error: string | null } | null;
};

export function ExperienceCommunity({ endpoint, sessionToken }: { endpoint: string; sessionToken: string }) {
  const [data, setData] = useState<Library | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  useEffect(() => {
    const abort = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    let first = true;
    async function refresh() {
      try {
        const library = await apiFetch<Library>(`${endpoint}/community`, sessionToken, { signal: abort.signal });
        if (abort.signal.aborted) return;
        setData(library);
        if (first) { setSettings(library.settings); first = false; }
        setError(null);
      } catch (cause) {
        if (!abort.signal.aborted) setError(cause instanceof Error ? cause.message : "Unable to load the behavior library");
      } finally {
        if (!abort.signal.aborted) timer = setTimeout(() => void refresh(), 5000);
      }
    }
    void refresh();
    return () => { abort.abort(); clearTimeout(timer); };
  }, [endpoint, sessionToken]);

  async function save() {
    setSaving(true);
    setSaveError(null);
    try {
      await apiFetch(`${endpoint}/community`, sessionToken, { method: "PUT", body: JSON.stringify(settings) });
      setData(await apiFetch<Library>(`${endpoint}/community`, sessionToken));
    } catch (cause) {
      setSaveError(cause instanceof Error ? cause.message : "Unable to save community settings");
    } finally { setSaving(false); }
  }

  async function retry(id: string) {
    setSaving(true);
    setSaveError(null);
    try {
      await apiFetch(`${endpoint}/community/${encodeURIComponent(id)}/retry`, sessionToken, { method: "POST" });
      setData(await apiFetch<Library>(`${endpoint}/community`, sessionToken));
    } catch (cause) {
      setSaveError(cause instanceof Error ? cause.message : "Unable to retry preparation");
    } finally { setSaving(false); }
  }

  return <section aria-label="Behavior community" style={{ display: "grid", gap: "var(--space-3)" }}>
    <h3>Behavior community</h3>
    <p>Keep experience locally, or learn from shared problem cards and versioned solutions. Original traces are never published.</p>
    {error && <p role="alert" className="modal-form-error">{error}</p>}
    {saveError && <p role="alert" className="modal-form-error">{saveError}</p>}
    {!data && !error && <p role="status">Loading behavior evidence…</p>}
    {data && settings && <>
      <fieldset disabled={saving} style={{ display: "grid", gap: "var(--space-3)", padding: "var(--space-3)", border: "1px solid var(--border)", borderRadius: "var(--radius-md)" }}>
        <legend>Community permissions</legend>
        <label style={{ flexDirection: "row", alignItems: "center" }}><input style={{ width: "auto" }} type="checkbox" checked={settings.search} onChange={(event) => setSettings({ ...settings, search: event.target.checked, automatic_trial: event.target.checked && settings.automatic_trial })} />Search accepted community experience</label>
        <label style={{ flexDirection: "row", alignItems: "center" }}><input style={{ width: "auto" }} type="checkbox" disabled={!settings.search} checked={settings.automatic_trial} onChange={(event) => setSettings({ ...settings, automatic_trial: event.target.checked })} />Automatically trial applicable solutions through learning review</label>
        <label style={{ flexDirection: "row", alignItems: "center" }}><input style={{ width: "auto" }} type="checkbox" checked={settings.contribute} onChange={(event) => setSettings({ ...settings, contribute: event.target.checked })} />Contribute independently reviewed, redacted experience publicly</label>
      </fieldset>
      <p>Public contributions use this server’s Hugging Face publisher account and remain pending until the community accepts them. Redaction can make mistakes; enable only for work you are authorized to share.</p>
      {!data.publisher_configured && <p>Public contribution needs a server publisher token. Local learning and public search do not.</p>}
      <button type="button" className="btn-secondary" disabled={saving} onClick={() => void save()}>{saving ? "Saving…" : "Save community permissions"}</button>
      <p>Displayed evidence: {data.counts.encountered} encounters · {data.counts.applied} applications · {data.counts.effective} effective · {data.counts.ineffective} ineffective · {data.counts.recurrence} recurrences. These are deduplicated reports, not model failure rates.</p>
      <h4>Local experience</h4>
      {!data.local.length && <p>No confirmed problems have been organized yet.</p>}
      {data.local.map((entry) => <details key={entry.id}>
        <summary>{entry.record?.problem.title ?? "Preparing experience"} · {entry.state}</summary>
        {entry.error && <p role="status">{entry.error}</p>}
        {entry.state === "blocked" && <button type="button" className="btn-secondary" disabled={saving} onClick={() => void retry(entry.id)}>Retry preparation and review</button>}
        {entry.record && <BehaviorDetails record={entry.record} />}
        {entry.url?.startsWith("https://huggingface.co/datasets/gjcjcg/ai-bad-behavior-library/discussions/") && <a href={entry.url} target="_blank" rel="noreferrer">View community review</a>}
      </details>)}
      {data.settings.search && <>
        <h4>Accepted community experience</h4>
        {data.sync?.error && <p role="status">{data.sync.error}</p>}
        {!data.community.length && <p>No accepted community records are cached yet.</p>}
        {data.community.map((record) => <details key={record.id}><summary>{record.problem.title}</summary><BehaviorDetails record={record} /></details>)}
      </>}
    </>}
  </section>;
}

function BehaviorDetails({ record }: { record: Record }) {
  return <>
    <p>Execution model: {record.model.observed ?? "Not present in the source"} · {record.model.harness}{record.model.harness_version && ` ${record.model.harness_version}`}</p>
    {record.model.configured && <p>Configured model: {record.model.configured} (not execution evidence)</p>}
    <p><strong>Context:</strong> {record.problem.input_background}</p>
    <p><strong>Expected:</strong> {record.problem.expected_behavior}</p>
    <p><strong>Observed problem:</strong> {record.problem.bad_behavior}</p>
    <p><strong>Applies to:</strong> {record.problem.applicability}</p>
    <p>{record.kind} · {record.evidence_summary}</p>
    {record.solution && <>
      <p>Solution {record.solution.id} · {record.solution.applicability}</p>
      {record.solution.based_on.length > 0 && <p>Adapted from: {record.solution.based_on.join(", ")}</p>}
      <pre style={{ whiteSpace: "pre-wrap" }}>{record.solution.instruction}</pre>
      {record.solution.team && <details><summary>Execution team · {record.solution.team.order}</summary>
        {record.solution.team.members.map((member) => <section key={member.name}><h5>{member.name}</h5><pre style={{ whiteSpace: "pre-wrap" }}>{member.prompt}</pre></section>)}
      </details>}
    </>}
  </>;
}
