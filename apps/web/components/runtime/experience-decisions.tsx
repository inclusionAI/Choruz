"use client";

import { useState } from "react";
import { apiFetch, type RuntimeBinding } from "../../lib/api/choruz-api";
import type { BrowserWorkflow } from "./browser-workflow";

export type DecisionSettings = {
  model: string;
  minimum_confidence: number;
  classify: boolean;
  supervise: boolean;
  assist_turns: boolean;
  builder_binding_id: string | null;
};

export type DecisionEvidence = {
  decisions?: Record<string, { status?: string; outcome?: string; error_category?: string; answer?: { model: string; answers: Record<string, { choice?: string; confidence?: number }> } }> | null;
  program_trial?: { status: string; model?: string; workflow?: BrowserWorkflow; program?: { name: string; applicability: string }; results?: { score?: number | null; split?: string }[] } | null;
};

export function ExperienceDecisions({ endpoint, sessionToken, initial, bindings, onSaved }: {
  endpoint: string; sessionToken: string; initial: DecisionSettings | null;
  bindings: RuntimeBinding[]; onSaved: () => void;
}) {
  const [enabled, setEnabled] = useState(Boolean(initial));
  const [value, setValue] = useState<DecisionSettings>(initial ?? { model: "", minimum_confidence: 0.9, classify: true, supervise: true, assist_turns: false, builder_binding_id: null });
  const [saving, setSaving] = useState(false);
  const [confidence, setConfidence] = useState(String(value.minimum_confidence));
  const confidenceValid = confidence.trim() !== "" && Number.isFinite(Number(confidence)) && Number(confidence) >= 0 && Number(confidence) <= 1;
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  async function save() {
    if (enabled && !confidenceValid) return;
    setSaving(true); setError(null); setSaved(false);
    try {
      await apiFetch(`${endpoint}/decisions`, sessionToken, { method: "PUT", body: JSON.stringify({ settings: enabled ? { ...value, minimum_confidence: Number(confidence) } : null }) });
      setSaved(true); onSaved();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Unable to save decision settings");
    } finally { setSaving(false); }
  }
  return <section aria-label="Fast decisions" className="modal-form">
    <h3>Fast decisions</h3>
    <p className="field-hint">Optional Jev classification, progress advice and reusable decision programs. They do not replace the independent evaluator or authorize tool actions.</p>
    <label><input type="checkbox" style={{ width: "auto" }} checked={enabled} disabled={saving} onChange={(e) => { setEnabled(e.target.checked); setSaved(false); }} /> Allow this Agent’s task evidence to be sent to TypeSafe</label>
    {enabled && <>
      <p className="field-hint">This includes task text, tool results and feedback from the selected Agent. Configure TYPESAFE_API_KEY on that Agent’s device; the key is not entered here. Calls consume provider usage. Background learning must also be enabled.</p>
      <label>Jev model<input value={value.model} maxLength={128} disabled={saving} placeholder="Exact provider model ID" onChange={(e) => { setValue({ ...value, model: e.target.value }); setSaved(false); }} /></label>
      <label>Minimum routing confidence<input type="number" min={0} max={1} step={0.01} value={confidence} disabled={saving} aria-invalid={!confidenceValid} onChange={(e) => { setConfidence(e.target.value); setSaved(false); }} /></label>
      <label><input type="checkbox" style={{ width: "auto" }} checked={value.classify} disabled={saving} onChange={(e) => { setValue({ ...value, classify: e.target.checked }); setSaved(false); }} /> Classify reviewed work</label>
      <label><input type="checkbox" style={{ width: "auto" }} checked={value.supervise} disabled={saving} onChange={(e) => { setValue({ ...value, supervise: e.target.checked }); setSaved(false); }} /> Suggest progress checks</label>
      <label><input type="checkbox" style={{ width: "auto" }} checked={Boolean(value.assist_turns)} disabled={saving} onChange={(e) => { setValue({ ...value, assist_turns: e.target.checked }); setSaved(false); }} /> Assist later Agent turns with the selected program</label>
      <p className="field-hint">When enabled, later task inputs are sent to TypeSafe on this device. The selected program supplies a bounded proposal; your Agent still handles tools, verification and the final reply. Provider failures and abstentions return the task to your Agent.</p>
      <label>Program-building Agent<select value={value.builder_binding_id ?? ""} disabled={saving} onChange={(e) => { setValue({ ...value, builder_binding_id: e.target.value || null }); setSaved(false); }}>
        <option value="">Do not build programs</option>
        {bindings.filter((b) => b.driver_type === "codex_terminal").map((b) => <option value={b.id} key={b.id}>{b.agent_name} · {b.runtime_host_id ? "Remote device" : "This computer"}</option>)}
      </select></label>
      <p className="field-hint">The selected Codex Agent builds bounded programs from reviewed training tasks in a separate session. Independent held-out tasks evaluate them. Programs remain inactive until explicitly selected.</p>
    </>}
    {error && <p role="alert" className="modal-form-error">{error}</p>}
    {saved && <p role="status">Decision settings saved.</p>}
    <button className="btn-secondary" type="button" disabled={saving || (enabled && (!confidenceValid || !value.model.trim() || !(value.classify || value.supervise || value.assist_turns || value.builder_binding_id)))} onClick={() => void save()}>{saving ? "Saving decisions…" : "Save decision settings"}</button>
  </section>;
}

export function DecisionHistory({ evidence, active = false, onSelect, disabled = false }: { evidence: DecisionEvidence; active?: boolean; onSelect?: () => void; disabled?: boolean }) {
  return <>
    {evidence.decisions && <section aria-label="Decision observations"><h4>Decision observations · advisory</h4>{Object.entries(evidence.decisions).map(([kind, result]) => <div key={kind}>
      <p><strong>{kind}</strong> · {result.status ?? result.outcome}{result.error_category ? ` · ${result.error_category}` : ""}</p>
      {result.answer && <><p>{result.answer.model}</p><ul>{Object.entries(result.answer.answers).map(([head, answer]) => <li key={head}>{head}: {answer.choice ?? "No choice"}{answer.confidence == null ? "" : ` · ${(answer.confidence * 100).toFixed(0)}% confidence`}</li>)}</ul></>}
    </div>)}</section>}
    {evidence.program_trial && <section aria-label="Program evaluation"><h4>Program evaluation</h4>
      <p>{evidence.program_trial.status.replaceAll("_", " ")} · {active ? "selected" : "not active"}</p>
      {evidence.program_trial.program && <p><strong>{evidence.program_trial.program.name}</strong>: {evidence.program_trial.program.applicability}</p>}
      {evidence.program_trial.results && <p>{evidence.program_trial.results.filter((r) => r.score === 1).length} / {evidence.program_trial.results.length} evaluated cases passed</p>}
      {evidence.program_trial.status === "validated" && onSelect && <button className="btn-secondary" type="button" disabled={disabled || active} onClick={onSelect}>Select this program</button>}
    </section>}
  </>;
}

export function DecisionExecution({ endpoint, sessionToken, onClear }: { endpoint: string; sessionToken: string; onClear: () => void }) {
  const [input, setInput] = useState("");
  const [output, setOutput] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  async function run() {
    setRunning(true); setError(null); setOutput(null);
    try {
      const reply = await apiFetch<{ result: { output: string | null } }>(`${endpoint}/decisions/execute`, sessionToken, { method: "POST", body: JSON.stringify({ input }) });
      setOutput(reply.result.output ?? "Program abstained. Return this task to your Agent.");
    } catch (cause) { setError(cause instanceof Error ? cause.message : "Program execution failed"); }
    finally { setRunning(false); }
  }
  return <section aria-label="Selected decision program" className="modal-form">
    <h4>Use selected program</h4>
    <label>Program input<textarea value={input} maxLength={16000} disabled={running} onChange={(e) => setInput(e.target.value)} /></label>
    <p className="field-hint">Runs the validated program on this Agent’s device. It only returns an answer; it cannot operate tools or change files.</p>
    <button className="btn-primary" type="button" disabled={running || !input.trim()} onClick={() => void run()}>{running ? "Running program…" : "Run program"}</button>
    <button className="btn-secondary" type="button" disabled={running} onClick={onClear}>Clear selected program</button>
    {error && <p role="alert" className="modal-form-error">{error}</p>}
    {output !== null && <pre role="status" style={{ whiteSpace: "pre-wrap" }}>{output}</pre>}
  </section>;
}
