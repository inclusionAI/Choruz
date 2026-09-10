"use client";

import { useEffect, useRef, useState } from "react";
import { manageComputerUse, type ComputerUseState } from "../../lib/api/choruz-api";

const labels = { browser: "Browser automation", desktop: "Desktop automation" };
const statuses = { ready: "Ready", installing: "Installing…", error: "Installation failed", needs_attention: "Setup needed" };

export function ComputerUseSettings({ sessionToken, companyId, runtimeHostId }: { sessionToken: string; companyId: string; runtimeHostId: string }) {
  const [state, setState] = useState<ComputerUseState | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [refresh, setRefresh] = useState(0);
  const controller = useRef<AbortController | null>(null);
  useEffect(() => {
    const abort = new AbortController();
    controller.current = abort;
    let timer: ReturnType<typeof setTimeout> | undefined;
    setError(null);
    const check = async () => {
      try {
        const result = await manageComputerUse(sessionToken, companyId, runtimeHostId, undefined, abort.signal);
        if (abort.signal.aborted) return;
        setState(result);
        if (result.tools.some((tool) => tool.status === "installing")) timer = setTimeout(check, 2000);
      } catch (caught) {
        if (!abort.signal.aborted) setError(caught instanceof Error ? caught.message : "Unable to check device tools");
      }
    };
    void check();
    return () => { abort.abort(); clearTimeout(timer); };
  }, [sessionToken, companyId, runtimeHostId, refresh]);

  const change = async (tool: "browser" | "desktop", enabled: boolean) => {
    const current = controller.current;
    const previous = state;
    setState((value) => value && ({ tools: value.tools.map((item) => item.tool === tool ? { ...item, enabled } : item) }));
    setBusy(true);
    setError(null);
    try {
      const result = await manageComputerUse(sessionToken, companyId, runtimeHostId, { tool, enabled }, current?.signal);
      if (!current?.signal.aborted) { setState(result); setRefresh((value) => value + 1); }
    } catch (caught) {
      if (!current?.signal.aborted) { setState(previous); setError(caught instanceof Error ? caught.message : "Unable to configure device tools"); }
    } finally { if (!current?.signal.aborted) setBusy(false); }
  };

  return <section aria-label="Computer use" className="modal-form">
    <h3>Computer use</h3>
    <p className="field-hint">Enable to install the upstream tools and connect their skills on this device. Installation continues if you close this panel. Restart existing Agents to discover new skills.</p>
    <p className="field-hint">Disabling removes Choruz-managed skill links from isolated accounts on their next launch. It does not uninstall tools, remove your own skills, stop running Agents, or revoke system permissions.</p>
    {!state && !error && <p role="status">Checking device tools…</p>}
    {error && <p role="alert" className="create-agent-warning">{error}</p>}
    {state?.tools.map((tool) => <div key={tool.tool}>
      <label className="harness-accounts-toggle" style={{ display: "flex", flexDirection: "row" }}><input style={{ width: "auto" }} type="checkbox" checked={tool.enabled} disabled={busy || tool.status === "installing"} onChange={(event) => void change(tool.tool, event.target.checked)} />{labels[tool.tool]}</label>
      <p role="status">{tool.enabled ? statuses[tool.status] : "Disabled"}</p>
      <ul>{tool.checks.map((check, index) => <li key={`${check.name}:${index}`}>{check.ok ? "✓" : "!"} {check.name}{!check.ok && check.hint ? ` — ${check.hint}` : ""}</li>)}</ul>
      {tool.status !== "ready" && tool.status !== "installing" && <button type="button" disabled={busy} onClick={() => void change(tool.tool, true)}>Install / repair {labels[tool.tool].toLowerCase()}</button>}
    </div>)}
    <p className="field-hint">Browser automation needs the BrowserSkill extension in a browser on the selected device. Desktop automation needs that device’s graphical desktop and OS permissions.</p>
    <a href="https://chromewebstore.google.com/detail/hhcmgoofomhgciiibhipgmgkgnoenaoi" target="_blank" rel="noreferrer">Open BrowserSkill extension page</a>
    <button type="button" disabled={busy} onClick={() => setRefresh((value) => value + 1)}>Check health again</button>
  </section>;
}
