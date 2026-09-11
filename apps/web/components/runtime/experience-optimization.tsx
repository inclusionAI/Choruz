"use client";

import { useEffect, useId, useState } from "react";
import { apiFetch } from "../../lib/api/choruz-api";

type Case = { id: string; split: "train" | "validation" | "test"; input: string; environment?: { image: string; files: Record<string, string>; verification: string[]; max_steps: number }; check: { type: "exact" | "json"; expected: unknown } | { type: "judge"; expected: string; rubric: string } };
export type OptimizationSettings = {
  trace_cases?: boolean;
  suite: { name: string; cases: Case[] };
  config: { max_metric_calls: number; max_proposals: number; minibatch_size: number; seed: number; merge: boolean; cache_evaluations: boolean; evolve_team: boolean; max_agents: number };
  auto_apply: boolean;
};

export function emptyOptimization(): OptimizationSettings {
  return { suite: { name: "", cases: (["train", "validation", "test"] as const).map((split) => ({ id: split, split, input: "", check: { type: "exact", expected: "" } })) }, config: { max_metric_calls: 32, max_proposals: 3, minibatch_size: 1, seed: 7, merge: true, cache_evaluations: false, evolve_team: false, max_agents: 4 }, auto_apply: false };
}

export function OptimizationFields({ value, onChange, disabled }: { value: OptimizationSettings; onChange: (value: OptimizationSettings) => void; disabled: boolean }) {
  const id = useId();
  const executionCalls = value.config.max_metric_calls * (value.config.max_agents - 1 + Math.max(1, ...value.suite.cases.map((row) => row.environment?.max_steps ?? 1)));
  function changeCase(index: number, update: Partial<Case>) {
    onChange({ ...value, suite: { ...value.suite, cases: value.suite.cases.map((row, i) => i === index ? { ...row, ...update } : row) } });
  }
  return <fieldset disabled={disabled} className="modal-form">
    <legend>Fixed evaluation suite</legend>
    <label><input type="checkbox" style={{ width: "auto" }} checked={value.trace_cases ?? false} onChange={(e) => onChange({ ...value, trace_cases: e.target.checked })} /> Automatically build tasks from work episodes</label>
    {value.trace_cases && <p className="field-hint">The background analyst turns meaningful objectives into tasks and independently reviews their answer evidence. Unsupported or later-disputed answers are excluded. Runs wait for independent objectives covering all three splits, then freeze a snapshot. This is historical regression evidence, not unseen-task performance. No manual answers are required.</p>}
    <p className="field-hint">Use distinct, self-contained tasks with reference answers. Exact checks compare text; an independent AI judge applies your acceptance criteria using a fixed platform skill. Inconclusive judgments stop the run without assigning a failure score. Training examples guide proposals; validation selects a winner; held-out tests can reject application but cannot change the winner. Tasks cannot use tools or modify your workspace. These scores measure only this suite, not general research ability.</p>
    {!value.trace_cases && <>
    <label>Suite name<input value={value.suite.name} maxLength={120} onChange={(e) => onChange({ ...value, suite: { ...value.suite, name: e.target.value } })} /></label>
    {value.suite.cases.map((row, index) => <fieldset key={row.id} className="modal-form">
      <legend>Case {index + 1}</legend>
      <label>Split<select value={row.split} onChange={(e) => changeCase(index, { split: e.target.value as Case["split"] })}><option value="train">Training</option><option value="validation">Validation</option><option value="test">Held-out test</option></select></label>
      <label htmlFor={`${id}-${index}-task`}>Task input</label><textarea id={`${id}-${index}-task`} value={row.input} maxLength={16000} rows={2} onChange={(e) => changeCase(index, { input: e.target.value })} />
      <label htmlFor={`${id}-${index}-assessment`}>Assessment</label><select id={`${id}-${index}-assessment`} value={row.check.type} onChange={(e) => changeCase(index, { check: e.target.value === "judge" ? { type: "judge", expected: typeof row.check.expected === "string" ? row.check.expected : JSON.stringify(row.check.expected), rubric: "" } : { type: "exact", expected: typeof row.check.expected === "string" ? row.check.expected : JSON.stringify(row.check.expected) } })}><option value="exact">Exact text</option>{row.check.type === "json" && <option value="json">Saved JSON</option>}<option value="judge">AI judge</option></select>
      <label htmlFor={`${id}-${index}-answer`}>Expected answer</label><textarea id={`${id}-${index}-answer`} value={typeof row.check.expected === "string" ? row.check.expected : JSON.stringify(row.check.expected)} maxLength={row.check.type === "judge" ? 8000 : 16000} rows={2} onChange={(e) => changeCase(index, { check: row.check.type === "judge" ? { ...row.check, expected: e.target.value } : { type: "exact", expected: e.target.value } })} />
      {row.check.type === "judge" && <><label htmlFor={`${id}-${index}-criteria`}>Acceptance criteria</label><textarea id={`${id}-${index}-criteria`} value={row.check.rubric} maxLength={4000} rows={3} onChange={(e) => { if (row.check.type === "judge") changeCase(index, { check: { ...row.check, rubric: e.target.value } }); }} /></>}
      {row.check.type === "json" && <p className="field-hint">Saved JSON comparison. Editing the answer switches to exact text comparison.</p>}
      <button className="btn-secondary" type="button" disabled={value.suite.cases.length <= 3} onClick={() => onChange({ ...value, suite: { ...value.suite, cases: value.suite.cases.filter((_, i) => i !== index) } })}>Remove case {index + 1}</button>
    </fieldset>)}
    <button className="btn-secondary" type="button" disabled={value.suite.cases.length >= 64} onClick={() => onChange({ ...value, suite: { ...value.suite, cases: [...value.suite.cases, { id: crypto.randomUUID(), split: "train", input: "", check: { type: "exact", expected: "" } }] } })}>Add case</button>
    </>}
    <details><summary>Search budget and ordering</summary><div className="modal-form">
      <label>Maximum task evaluations<input type="number" min={4} max={512} value={value.config.max_metric_calls} onChange={(e) => onChange({ ...value, config: { ...value.config, max_metric_calls: Number(e.target.value) } })} /></label>
      <label>Maximum proposals<input type="number" min={1} max={32} value={value.config.max_proposals} onChange={(e) => onChange({ ...value, config: { ...value.config, max_proposals: Number(e.target.value) } })} /></label>
      <label>Training batch size<input type="number" min={1} max={64} value={value.config.minibatch_size} onChange={(e) => onChange({ ...value, config: { ...value.config, minibatch_size: Number(e.target.value) } })} /></label>
      <label>Ordering seed<input type="number" min={0} max={Number.MAX_SAFE_INTEGER} value={value.config.seed} onChange={(e) => onChange({ ...value, config: { ...value.config, seed: Number(e.target.value) } })} /></label>
      <label><input type="checkbox" style={{ width: "auto" }} checked={value.config.merge} onChange={(e) => onChange({ ...value, config: { ...value.config, merge: e.target.checked } })} /> Combine complementary candidates</label>
      <label><input type="checkbox" style={{ width: "auto" }} checked={value.config.cache_evaluations} onChange={(e) => onChange({ ...value, config: { ...value.config, cache_evaluations: e.target.checked } })} /> Reuse measured candidate/case results</label>
    </div></details>
    <label><input type="checkbox" style={{ width: "auto" }} checked={value.config.evolve_team} onChange={(e) => onChange({ ...value, config: { ...value.config, evolve_team: e.target.checked } })} /> Evolve the internal execution team</label>
    <p className="field-hint">After a documented recurring problem, search can change collaborator count, individual prompts and serial or parallel ordering. The analyst and fixed evaluation stay unchanged. Collaborators use the Agent’s existing device and account, without task tools or extra permissions.</p>
    <label>Maximum total agents per task<input type="number" min={1} max={4} value={value.config.max_agents} onChange={(e) => onChange({ ...value, config: { ...value.config, max_agents: Number(e.target.value) } })} /></label>
    <p className="field-hint">Each new reviewed revision can start a run automatically, even with this panel closed. Per run: at most {executionCalls} execution calls including collaborators, {value.config.max_metric_calls} independent judge calls, {value.config.max_proposals} proposal calls and one final review. This is a call limit, not a token or billing limit. Disabling learning cancels pending work; an in-flight provider call may still consume usage.</p>
    <label><input type="checkbox" style={{ width: "auto" }} checked={value.auto_apply} onChange={(e) => onChange({ ...value, auto_apply: e.target.checked })} /> Automatically apply a measured and reviewed improvement</label>
    <p className="field-hint">Without this consent, runs only collect evidence. With consent, guidance changes only for future turns. Clear or restore a revision in Learning history to roll back.</p>
  </fieldset>;
}

type Run = { id: string; name: string; status: string; error_code: string | null; application_status: string; applied_revision_id: string | null; search_summary?: { metric_calls: number; proposal_calls: number; model_calls_reserved: number; candidate_count: number; scores: [number, number, number, number] | null } };

export function OptimizationHistory({ endpoint, sessionToken }: { endpoint: string; sessionToken: string }) {
  const [runs, setRuns] = useState<Run[]>([]);
  const [detail, setDetail] = useState<unknown>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  useEffect(() => {
    const abort = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      try {
        const response = await apiFetch<{ evaluations: Run[] }>(`${endpoint}/evaluations`, sessionToken, { signal: abort.signal });
        if (!abort.signal.aborted) { setRuns(response.evaluations); setError(null); }
      } catch (cause) {
        if (!abort.signal.aborted) setError(cause instanceof Error ? cause.message : "Unable to load evaluations");
      } finally {
        if (!abort.signal.aborted) timer = setTimeout(() => void poll(), 3000);
      }
    }
    void poll();
    return () => { abort.abort(); clearTimeout(timer); };
  }, [endpoint, sessionToken]);
  async function inspect(id: string) {
    setLoading(true);
    try { setDetail(await apiFetch(`${endpoint}/evaluations?id=${encodeURIComponent(id)}`, sessionToken)); }
    catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to load evaluation details"); }
    finally { setLoading(false); }
  }
  return <section aria-label="Evaluation history">
    <h3>Evaluation history</h3>
    {error && <p role="alert">{error}</p>}
    {!runs.length && <p>No evaluations yet. A saved suite evaluates the next reviewed revision automatically.</p>}
    {runs.map((run) => <article key={run.id}>
      <h4>{run.name} · {run.status}</h4>
      <p>Application: {run.application_status.replaceAll("_", " ")}{run.error_code ? ` · ${run.error_code.replaceAll("_", " ")}` : ""}</p>
      {run.search_summary && <><p>{run.search_summary.candidate_count} candidates · {run.search_summary.metric_calls} task evaluations · {run.search_summary.proposal_calls} proposals · {run.search_summary.model_calls_reserved} model calls reserved</p>
        {run.search_summary.scores && <table style={{ width: "100%", borderSpacing: "12px 8px", textAlign: "left" }}><caption>Fixed-suite pass rates, not general capability</caption><thead><tr><th>Split</th><th>Baseline</th><th>Selected winner</th></tr></thead><tbody>{["Validation", "Held-out test"].map((split, i) => <tr key={split}><th>{split}</th><td>{(run.search_summary!.scores![i * 2] * 100).toFixed(0)}%</td><td>{(run.search_summary!.scores![i * 2 + 1] * 100).toFixed(0)}%</td></tr>)}</tbody></table>}
      </>}
      <button className="btn-secondary" type="button" disabled={loading} onClick={() => void inspect(run.id)}>Inspect run {run.id.slice(0, 8)}</button>
    </article>)}
    {detail !== null && <details open><summary>Frozen suite, candidates and observations</summary><pre style={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}>{JSON.stringify(detail, null, 2)}</pre></details>}
  </section>;
}
