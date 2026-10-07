"use client";

import { useRef, useState } from "react";
import { ArrowUp, Code2, Search, WandSparkles } from "lucide-react";
import { useDriverAvailability } from "../../hooks/use-driver-availability";
import { transportFetch } from "../../lib/api/transport";
import { trace } from "../../lib/api/choruz-trace";
import { readDraft, writeDraft, writeSessionDraft } from "../../lib/chat-drafts";
import type { ProvisionResponse } from "../../lib/agents/agent-provisioning";

type Props = {
  principalId: string;
  companyId: string | null;
  projectName: string;
  workspacePath?: string | null;
  onCreated: (result: ProvisionResponse) => Promise<void>;
  onAdvanced: () => void;
};

export function TaskStart({ principalId, companyId, projectName, workspacePath, onCreated, onAdvanced }: Props) {
  const draftScope = `new-task:${companyId}`;
  const [prompt, setPrompt] = useState(() => readDraft(principalId, draftScope));
  function updatePrompt(value: string) {
    setPrompt(value);
    writeDraft(principalId, draftScope, value);
  }
  const [driver, setDriver] = useState("codex_terminal");
  const [model, setModel] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const attempt = useRef<{ key: string; payload: string; result?: ProvisionResponse } | null>(null);
  const inFlight = useRef(false);
  const { availability, loaded, error: availabilityError } = useDriverAvailability();
  const drivers = availability.filter((item) =>
    (item.driverId === "codex_terminal" || item.driverId === "claude_terminal") && item.status === "available",
  );
  const selected = drivers.find((item) => item.driverId === driver) ?? drivers[0];

  async function start() {
    if (inFlight.current || !prompt.trim() || !selected || !companyId) return;
    inFlight.current = true;
    setBusy(true);
    setError(null);
    const request = {
      name: prompt.trim().replace(/\s+/g, " ").slice(0, 64),
      driver_type: selected.driverId,
      inherit_learning: true,
      instructions: "Complete the user's task in the selected workspace. Report results and unresolved blockers clearly.",
      workspace_id: companyId,
      ...(workspacePath ? { workspace_path: workspacePath } : {}),
      ...(model.trim() ? { model: model.trim() } : {}),
    };
    const payload = JSON.stringify(request);
    if (attempt.current?.payload !== payload) attempt.current = { key: crypto.randomUUID(), payload };
    const current = attempt.current;
    const span = trace.start("start_workbench_task", { driver: selected.driverId, company_id: companyId });
    try {
      if (!current.result) {
        const response = await transportFetch("/api/agents/provision", {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({
            ...request,
            name: `${request.name} · ${current.key.slice(0, 8)}`,
            idempotency_key: current.key,
          }),
        });
        if (!response.ok) {
          const detail = await response.json().catch(() => null);
          throw new Error(typeof detail?.error === "string" ? detail.error : "Could not start the task. Please retry.");
        }
        current.result = await response.json() as ProvisionResponse;
      }
      writeSessionDraft(principalId, current.result.binding.id, { text: prompt.trim(), submissionId: `task:${current.result.binding.id}`, autoSubmit: true });
      await onCreated(current.result);
      writeDraft(principalId, draftScope, "");
      span.end({ status: 201 });
    } catch (failure) {
      const message = failure instanceof Error ? failure.message : "Could not start the task.";
      setError(message);
      span.end({ error: message });
    } finally {
      inFlight.current = false;
      setBusy(false);
    }
  }

  return <section className="task-start" aria-label="New task">
    <div className="task-start-content">
      <p className="task-project">{projectName}</p>
      <h2>What would you like to work on?</h2>
      <form className="task-start-composer" onSubmit={(event) => { event.preventDefault(); void start(); }}>
        <textarea autoFocus aria-label="Task instructions" placeholder="Ask anything, or describe a task…" value={prompt} disabled={busy}
          onChange={(event) => updatePrompt(event.target.value)} rows={4}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
              event.preventDefault(); void start();
            }
          }} />
        <div className="task-start-controls">
          <select aria-label="Task harness" disabled={busy || !loaded} value={selected?.driverId ?? ""} onChange={(event) => setDriver(event.target.value)}>
            {!drivers.length && <option value="">{loaded ? "No supported CLI installed" : "Checking available agents…"}</option>}
            {drivers.map((item) => <option key={item.driverId} value={item.driverId}>{item.label}</option>)}
          </select>
          <input aria-label="Task model" placeholder="Default model" value={model} disabled={busy} onChange={(event) => setModel(event.target.value)} />
          <button type="submit" aria-label="Start task" disabled={busy || !prompt.trim() || !selected || !companyId}><ArrowUp size={19} /></button>
        </div>
      </form>
      {(error || availabilityError) && <p role="alert" className="form-error">{error ?? availabilityError}</p>}
      {!companyId && <p>Create a project from settings to start a task.</p>}
      {busy && <p role="status">Preparing your task…</p>}
      <div className="task-suggestions">
        {[
          { Icon: Code2, label: "Build something", text: "Help me build " },
          { Icon: Search, label: "Explore the project", text: "Explore this project and explain its architecture." },
          { Icon: WandSparkles, label: "Improve something", text: "Review this project and suggest a focused improvement." },
        ].map(({ Icon, label, text }) => (
          <button type="button" key={label} disabled={busy} onClick={() => updatePrompt(text)}><Icon size={16} />{label}</button>
        ))}
      </div>
      <button type="button" className="task-advanced" disabled={busy} onClick={onAdvanced}>Configure device, account or agent</button>
    </div>
  </section>;
}
