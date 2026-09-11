"use client";

import { useEffect, useState } from "react";
import { apiFetch, type RuntimeBinding } from "../../lib/api/choruz-api";
import { Modal } from "../ui/modal";
import { ExperienceDataset, ExperiencePerformance, type DatasetReport, type TaskPerformance } from "./experience-dataset";
import { emptyOptimization, OptimizationFields, OptimizationHistory, type OptimizationSettings } from "./experience-optimization";

type Revision = { id: string; analysis: string; instruction: string; disposition: string; created_at: string; validation: { dataset?: DatasetReport | null; evaluation_cases?: { episode_ref: string; input: string; check: unknown | null; reason: string; classification?: { task_type: string; capability: string; structure: string; outcome: string } }[]; review?: string; observed_revision_id?: string; observed_revision_outcome?: string; team?: { config: { order: "serial" | "parallel"; members: { name: string; prompt: string }[] }; review: string } | null } };
type Learning = { policy: { enabled: boolean; analyst_binding_id: string; active_revision_id: string | null; last_error: string | null; checked_at: string | null; optimization_settings: OptimizationSettings | null; optimization_error: string | null } | null; revisions: Revision[]; task_performance?: TaskPerformance | null };

export function ExperienceSettings({ bindingId, sessionToken, onClose }: { bindingId: string; sessionToken: string; onClose: () => void }) {
  const [data, setData] = useState<Learning | null>(null);
  const [bindings, setBindings] = useState<RuntimeBinding[]>([]);
  const [analyst, setAnalyst] = useState("");
  const [enabled, setEnabled] = useState(false);
  const [measured, setMeasured] = useState(false);
  const [optimization, setOptimization] = useState<OptimizationSettings>(emptyOptimization);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const endpoint = `/v1/runtime/bindings/${encodeURIComponent(bindingId)}/experience`;
  useEffect(() => {
    const abort = new AbortController();
    void Promise.all([
      apiFetch<Learning>(endpoint, sessionToken, { signal: abort.signal }),
      apiFetch<RuntimeBinding[]>("/v1/runtime/bindings", sessionToken, { signal: abort.signal }),
    ]).then(([learning, available]) => {
      if (abort.signal.aborted) return;
      setData(learning);
      setAnalyst(learning.policy?.analyst_binding_id ?? "");
      setEnabled(learning.policy?.enabled ?? false);
      setMeasured(Boolean(learning.policy?.optimization_settings));
      setOptimization(learning.policy?.optimization_settings ?? emptyOptimization());
      const target = available.find((binding) => binding.id === bindingId);
      setBindings(available.filter((binding) => binding.id !== bindingId && binding.workspace_id === target?.workspace_id && binding.conversation_type === "direct" && binding.state !== "disabled" && ["claude_terminal", "codex_terminal"].includes(binding.driver_type)));
    }).catch((cause) => { if (!abort.signal.aborted) setError(cause instanceof Error ? cause.message : "Unable to load learning settings"); });
    return () => abort.abort();
  }, [bindingId, endpoint, sessionToken]);

  useEffect(() => {
    const abort = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      try {
        const learning = await apiFetch<Learning>(endpoint, sessionToken, { signal: abort.signal });
        if (!abort.signal.aborted) setData(learning);
      } catch { /* Explicit refresh reports failures; polling preserves the last snapshot. */ }
      finally { if (!abort.signal.aborted) timer = setTimeout(() => void poll(), 3000); }
    }
    timer = setTimeout(() => void poll(), 3000);
    return () => { abort.abort(); clearTimeout(timer); };
  }, [endpoint, sessionToken]);

  async function refreshHistory() {
    setSaving(true);
    setError(null);
    try {
      setData(await apiFetch<Learning>(endpoint, sessionToken));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Unable to refresh learning history");
    } finally {
      setSaving(false);
    }
  }

  async function selectRevision(revisionId: string | null) {
    setSaving(true);
    setError(null);
    try {
      setData(await apiFetch<Learning>(endpoint, sessionToken, {
        method: "PATCH",
        body: JSON.stringify({ revision_id: revisionId }),
      }));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Unable to select revision");
    } finally {
      setSaving(false);
    }
  }

  async function save() {
    setSaving(true);
    setError(null);
    try {
      setData(await apiFetch<Learning>(endpoint, sessionToken, {
        method: "PUT",
        body: JSON.stringify({ enabled, analyst_binding_id: analyst, optimization_settings: measured ? optimization : null }),
      }));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Unable to save learning settings");
    } finally {
      setSaving(false);
    }
  }

  return <Modal title="Experience learning" activitySurface="experience-learning" onClose={onClose} closeDisabled={saving} description="Learn from this Agent’s work and your feedback without interrupting the conversation.">
    {!data && !error && <p role="status">Loading learning settings…</p>}
    {error && <p className="modal-form-error" role="alert">{error}</p>}
    {data && <div className="modal-form">
      <label style={{ flexDirection: "row", alignItems: "center" }}><input style={{ width: "auto" }} type="checkbox" checked={enabled} onChange={(event) => setEnabled(event.target.checked)} disabled={saving} /> Enable background learning</label>
      <label>Analysis Agent<select value={analyst} onChange={(event) => setAnalyst(event.target.value)} disabled={saving}>
        <option value="">Choose an Agent</option>
        {bindings.map((binding) => <option key={binding.id} value={binding.id}>{binding.agent_name} · {binding.runtime_host_id ? "Remote device" : "This computer"}{binding.harness_account_name ? ` · ${binding.harness_account_name}` : ""}</option>)}
      </select></label>
      {!bindings.length && <p>Create another Claude Code or Codex Agent in this workspace to analyze the work.</p>}
      <p className="field-hint">Analysis uses the selected Agent’s account in a separate background session and consumes its usage. This Agent’s native transcripts, your shared conversation messages and the workspace’s CLAUDE.md and AGENTS.md are sent to that account’s model provider. Transcripts may include task data and quoted conversation content. Enable only for work you are authorized to share. Learned guidance stays scoped to this Agent; running turns keep their current instructions.</p>
      <p className="field-hint">For observed mistakes, a separate web search looks for prompt techniques using short problem categories, without the source transcripts. Search failures are reported rather than treated as missing guidance.</p>
      <p className="field-hint">Team changes require measured search, team-evolution consent and evidence of recurrence after guidance was used. Collaborators add calls on this Agent’s account before execution, increasing usage and waiting time. Clear or restore a revision to change its team. Collaborator proposals are not proof that checks passed.</p>
      <label><input style={{ width: "auto" }} type="checkbox" checked={measured} disabled={saving} onChange={(e) => setMeasured(e.target.checked)} /> Evaluate and optimize against a fixed suite</label>
      {measured && <OptimizationFields value={optimization} onChange={setOptimization} disabled={saving} />}
      {data.policy?.last_error && <p role="status">{data.policy.last_error}</p>}
      {data.policy?.optimization_error && <p role="status">{data.policy.optimization_error}</p>}
      {data.policy?.checked_at && <p>Last checked: {new Date(data.policy.checked_at).toLocaleString()}</p>}
      <button className="btn-primary" type="button" onClick={() => void save()} disabled={saving || !analyst}>{saving ? "Saving…" : "Save settings"}</button>
      <h3>Learning history</h3>
      <button className="btn-secondary" type="button" disabled={saving} onClick={() => void refreshHistory()}>Refresh history</button>
      {data.policy?.active_revision_id && <button className="btn-secondary" type="button" disabled={saving} onClick={() => void selectRevision(null)}>Clear active revision</button>}
      {!data.revisions.length && <p>{data.policy?.enabled ? "No analysis yet. Background checks continue after you close this panel." : "Learning is off. Choose an analysis Agent and save with learning enabled to begin."}</p>}
      {data.task_performance && <ExperiencePerformance report={data.task_performance} />}
      {data.revisions.map((revision) => <details key={revision.id} data-revision-id={revision.id}>
        <summary>{new Date(revision.created_at).toLocaleString()} · {revision.disposition.replaceAll("_", " ")}</summary>
        <p>{revision.analysis}</p>
        {revision.validation.dataset && <ExperienceDataset report={revision.validation.dataset} />}
        {Boolean(revision.validation.evaluation_cases?.length) && <section aria-label="Extracted evaluation tasks"><h4>Extracted evaluation tasks</h4>{revision.validation.evaluation_cases!.map((task) => <p key={task.episode_ref}><strong>{task.check === null ? "Not evaluable" : "Reviewed task"}</strong>: {task.input || task.episode_ref} — {task.reason}</p>)}</section>}
        {revision.instruction && <pre style={{ whiteSpace: "pre-wrap" }}>{revision.instruction}</pre>}
        {revision.validation.team && <section aria-label="Execution team"><p>{revision.validation.team.config.members.length + 1} total agents · {revision.validation.team.config.order} collaborators · {revision.validation.team.review.replaceAll("_", " ")}</p>{revision.validation.team.config.members.map((member) => <p key={member.name}><strong>{member.name}</strong>: {member.prompt}</p>)}</section>}
        <p>Evidence about the previously active revision: {revision.validation.observed_revision_outcome?.replaceAll("_", " ") ?? "not observed"}</p>
        {revision.validation.review === "passed" && revision.disposition === "superseded" && <button className="btn-secondary" type="button" disabled={saving} onClick={() => void selectRevision(revision.id)}>Restore this revision</button>}
      </details>)}
      <OptimizationHistory endpoint={endpoint} sessionToken={sessionToken} />
    </div>}
  </Modal>;
}
