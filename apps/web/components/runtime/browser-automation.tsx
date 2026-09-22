"use client";

import { useEffect, useState } from "react";
import { apiFetch } from "../../lib/api/choruz-api";

type Settings = { browser: string; allowed_urls: string[]; scope: string };
type Run = { id: string; status: string; result?: { report?: { checks_matched: boolean; completed_steps: number } } };
type State = { automation: { settings: Settings | null; ready: boolean } | null; runs: Run[] };

export function BrowserAutomation({ bindingId, sessionToken }: { bindingId: string; sessionToken: string }) {
  const endpoint = `/v1/runtime/bindings/${encodeURIComponent(bindingId)}/browser-automation`;
  const [data, setData] = useState<State | null>(null);
  const [enabled, setEnabled] = useState(false);
  const [browser, setBrowser] = useState("");
  const [pages, setPages] = useState("");
  const [scope, setScope] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [refresh, setRefresh] = useState(0);
  useEffect(() => {
    const abort = new AbortController();
    apiFetch<State>(endpoint, sessionToken, { signal: abort.signal }).then((next) => {
      if (abort.signal.aborted) return;
      setData(next);
      const settings = next.automation?.settings;
      setEnabled(Boolean(settings)); setBrowser(settings?.browser ?? "");
      setPages(settings?.allowed_urls.join("\n") ?? ""); setScope(settings?.scope ?? "");
    }).catch((cause) => { if (!abort.signal.aborted) setError(cause instanceof Error ? cause.message : "Unable to load browser automation"); });
    return () => abort.abort();
  }, [endpoint, sessionToken, refresh]);

  async function save(active: boolean) {
    setBusy(true); setError(null);
    try {
      const reply = await apiFetch<{ device_acknowledged: boolean }>(endpoint, sessionToken, {
        method: "PUT", body: JSON.stringify({ settings: active ? { browser, scope, allowed_urls: pages.split("\n").map((line) => line.trim()).filter(Boolean) } : null }),
      });
      if (!reply.device_acknowledged) setError("Automation stopped admitting work, but the device has not acknowledged cancellation. Already-dispatched actions cannot be recalled.");
      setRefresh((value) => value + 1);
    } catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to save browser automation"); }
    finally { setBusy(false); }
  }
  return <section className="modal-form" aria-label="Automatic browser work" data-activity-value="private">
    <h3>Automatic browser work</h3>
    <label><input type="checkbox" style={{ width: "auto" }} checked={enabled} disabled={!data || busy} onChange={(event) => setEnabled(event.target.checked)} /> Enable automatic learning and reuse</label>
    <p className="field-hint">Set permission once. Your Agent discovers learned workflows, checks them on current tasks and reuses successful versions without asking you to approve each run. Codex or Claude handles uncertain decisions. Failed or uncertain workflows pause for analysis, not blind retries.</p>
    {enabled && <>
      <label>Browser on this device<input value={browser} maxLength={128} disabled={busy} onChange={(event) => setBrowser(event.target.value)} /></label>
      <label>Permitted page URLs, one per line<textarea value={pages} disabled={busy} onChange={(event) => setPages(event.target.value)} /></label>
      <label>Permitted tasks<textarea value={scope} maxLength={2000} disabled={busy} onChange={(event) => setScope(event.target.value)} /></label>
      <p className="field-hint">Enabling allows clicks, text entry and selection within this scope on this Agent’s device and account. Task inputs, page URLs and labels may be sent to TypeSafe and the native reasoning Agent. Background learning and a program-building Agent must be configured. Closing this panel does not stop background work.</p>
    </>}
    <button className="btn-secondary" type="button" disabled={!data || busy || (enabled && (!browser.trim() || !pages.trim() || !scope.trim()))} onClick={() => void save(enabled)}>Save browser automation</button>
    {data?.automation?.settings && <button className="btn-secondary" type="button" disabled={busy} onClick={() => void save(false)}>Stop browser automation</button>}
    {data && <p role="status">{data.automation?.ready ? "Automation enabled" : "Automation off or learning unavailable"}</p>}
    <details><summary>Browser activity</summary>
      <button type="button" className="btn-secondary" disabled={busy} onClick={() => setRefresh((value) => value + 1)}>Refresh browser activity</button>
      {!data?.runs.length && <p>No browser runs yet.</p>}
      {data?.runs.map((run) => <p key={run.id}>{run.id} · {run.status.replaceAll("_", " ")}{run.result?.report ? ` · ${run.result.report.completed_steps} steps · ${run.result.report.checks_matched ? "page checks matched" : "needs analysis"}` : ""}</p>)}
    </details>
    {error && <p role="alert">{error}</p>}
  </section>;
}
