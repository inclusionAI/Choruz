"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { apiFetch, ApiRequestError } from "../../lib/api/choruz-api";
import { TerminalView } from "./terminal-view";
import { ExperienceSettings } from "./experience-settings";

type Item = { id: string; revision: number; position: number; kind: string; text: string; detail: Record<string, unknown>; status: string };
type Pending = { id?: string | number; request_id?: string; method?: string; params?: Record<string, unknown>; request?: Record<string, unknown> };
type Snapshot = { instance: string; revision: number; cursor: number; more: boolean; retained_from?: number; history_truncated?: boolean; session_id: string | null; status: string; items: Item[]; requests: Pending[]; error: string | null };
type Props = { bindingId: string; sessionToken: string; gatewayBaseUrl?: string };

function display(value: unknown): string {
  if (value == null) return "";
  return typeof value === "string" ? value : JSON.stringify(value, null, 2);
}

export function splitLearnedContext(text: string) {
  // Native text has no trusted attribution for a check-plan suffix. Keep it
  // visible instead of presenting user-crafted text as a platform-owned plan.
  const match = /\n\n\[choruz-experience revision=([a-zA-Z0-9-]+)\]\n[^\n]+\n([^]*?)\n\[\/choruz-experience\]$/.exec(text);
  if (!match) return { message: text, guidance: null };
  try {
    const instruction: unknown = JSON.parse(match[2]);
    if (typeof instruction === "string") return { message: text.slice(0, match.index), guidance: { revision: match[1], instruction } };
  } catch { /* Malformed or user-authored text stays visible verbatim. */ }
  return { message: text, guidance: null };
}

function SessionMessage({ item }: { item: Item }) {
  const { message, guidance } = item.kind === "user" ? splitLearnedContext(item.text) : { message: item.text, guidance: null };
  return <>
    <div className="agent-session-speaker">{item.kind === "user" ? "You" : "Agent"}</div>
    <ReactMarkdown remarkPlugins={[remarkGfm]}>{message}</ReactMarkdown>
    {guidance && <details><summary>Choruz learned context · {guidance.revision.slice(0, 8)}</summary><p>Supplemental context supplied by Choruz, not part of your message.</p><pre>{guidance.instruction}</pre></details>}
  </>;
}

export function mergeSessionPage(prior: Snapshot | null, page: Snapshot): Snapshot {
  if (!prior || prior.instance !== page.instance) return page;
  const items = new Map(prior.items.map((item) => [item.id, item]));
  for (const item of page.items) {
    if ((items.get(item.id)?.revision ?? -1) <= item.revision) items.set(item.id, item);
  }
  const latest = prior.revision > page.revision ? prior : page;
  return { ...latest, items: [...items.values()].filter((item) => item.position >= (latest.retained_from ?? 0)).sort((a, b) => a.position - b.position) };
}

export function AgentSessionView({ bindingId, sessionToken, gatewayBaseUrl }: Props) {
  const [learningOpen, setLearningOpen] = useState(false);
  const [mode, setMode] = useState<"conversation" | "terminal">("conversation");
  const [state, setState] = useState<Snapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const submission = useRef<{ text: string; id: string } | null>(null);
  const scroll = useRef<HTMLDivElement>(null);
  const follow = useRef(true);
  const endpoint = `/v1/runtime/bindings/${encodeURIComponent(bindingId)}/session`;

  useEffect(() => {
    if (mode !== "conversation") return;
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    let cursor = 0;
    let instance: string | null = null;
    let launchRetries = 0;
    async function poll(open: boolean) {
      try {
        const snapshot = await apiFetch<Snapshot>(open ? endpoint : `${endpoint}?after=${cursor}`, sessionToken, {
          method: open ? "POST" : "GET", signal: controller.signal,
        });
        if (controller.signal.aborted) return;
        if (instance !== null && instance !== snapshot.instance) {
          instance = snapshot.instance;
          cursor = 0;
          timer = setTimeout(() => void poll(false), 0);
          return;
        }
        instance = snapshot.instance;
        cursor = snapshot.cursor;
        setState((prior) => mergeSessionPage(prior, snapshot));
        setError(null);
        timer = setTimeout(() => void poll(false), snapshot.more ? 0 : 500);
      } catch (failure) {
        if (controller.signal.aborted) return;
        if (open && failure instanceof ApiRequestError && failure.status === 409 && launchRetries++ < 6) {
          timer = setTimeout(() => void poll(true), 500);
          return;
        }
        setError(failure instanceof Error ? failure.message : "Unable to connect to this Agent");
        if (!open) timer = setTimeout(() => void poll(false), 3000);
      }
    }
    void poll(true);
    return () => { controller.abort(); clearTimeout(timer); };
  }, [endpoint, sessionToken, mode, attempt]);

  useEffect(() => {
    if (follow.current && scroll.current) scroll.current.scrollTop = scroll.current.scrollHeight;
  }, [state]);

  const command = useCallback(async (payload: object) => {
    if (!state) return false;
    const instance = state.instance;
    setBusy(true);
    try {
      const snapshot = await apiFetch<Snapshot>(`${endpoint}/commands`, sessionToken, {
        method: "POST", body: JSON.stringify({ ...payload, instance }),
      });
      setState((prior) => prior?.instance === instance ? mergeSessionPage(prior, snapshot) : prior);
      setError(null);
      return true;
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : "The Agent action failed");
      return false;
    } finally { setBusy(false); }
  }, [endpoint, sessionToken, state]);

  async function send() {
    if (!draft.trim() || busy) return;
    if (submission.current?.text !== draft) submission.current = { text: draft, id: crypto.randomUUID() };
    if (await command({ action: "send", text: draft, submission_id: submission.current.id })) {
      setDraft("");
      submission.current = null;
      follow.current = true;
    }
  }

  async function switchToTerminal() {
    if (state && state.status !== "closed" && !await command({ action: "close" })) return;
    setMode("terminal");
  }

  const running = state?.status === "running" || state?.status === "waiting";
  return <section className="agent-session" aria-label="Agent session">
    <div className="agent-session-toolbar">
      <div role="group" aria-label="Session display">
        <button type="button" aria-pressed={mode === "conversation"} onClick={() => { setMode("conversation"); setAttempt((value) => value + 1); }}>Conversation</button>
        <button type="button" aria-pressed={mode === "terminal"} disabled={busy || running} onClick={() => void switchToTerminal()}>Terminal</button>
      </div>
      <span role="status">{mode === "terminal" ? "Raw terminal" : error ? "Disconnected" : state?.status ?? "Connecting…"}</span>
      <button type="button" onClick={() => setLearningOpen(true)}>Experience learning</button>
      {mode === "conversation" && running && <button type="button" disabled={busy} onClick={() => void command({ action: "interrupt" })}>Stop</button>}
    </div>
    {learningOpen && <ExperienceSettings bindingId={bindingId} sessionToken={sessionToken} onClose={() => setLearningOpen(false)} />}
    {mode === "terminal" ? <TerminalView bindingId={bindingId} sessionToken={sessionToken} gatewayBaseUrl={gatewayBaseUrl} /> : <>
      <div className="agent-session-timeline" ref={scroll} onScroll={() => {
        const element = scroll.current;
        if (element) follow.current = element.scrollHeight - element.scrollTop - element.clientHeight < 80;
      }}>
        {!state?.items.length && !error && <p className="agent-session-empty">Start a conversation. Replies and tool activity appear here.</p>}
        {state?.history_truncated && <p role="note">Showing a bounded conversation preview. Open Terminal for complete native history and tool output.</p>}
        {state?.items.map((item) => <article key={item.id} className={`agent-session-item is-${item.kind}`} data-session-item-id={item.id}>
          {item.kind === "tool" || item.kind === "reasoning" ? <details>
            <summary>{item.kind === "reasoning" ? "Reasoning" : item.text || "Tool activity"}<span>{item.status}</span></summary>
            <pre>{display(item.detail)}</pre>
          </details> : <SessionMessage item={item} />}
        </article>)}
        {state?.requests.map((request) => <SessionQuestion key={String(request.id ?? request.request_id)} request={request} busy={busy} onRespond={(allow, answers) => command({ action: "respond", request_id: request.id ?? request.request_id, allow, answers })} />)}
      </div>
      {(error || state?.error) && <div className="agent-session-error" role="alert">
        <p>{error ?? state?.error}</p>
        <button type="button" disabled={busy} onClick={() => { setAttempt((value) => value + 1); }}>Reconnect</button>
      </div>}
      <form className="agent-session-composer" onSubmit={(event) => { event.preventDefault(); void send(); }}>
        <textarea aria-label="Message Agent" placeholder="Message this Agent…" value={draft} onChange={(event) => setDraft(event.target.value)} rows={3} onKeyDown={(event) => {
          if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
            event.preventDefault();
            if (state?.status === "ready") void send();
          }
        }} />
        <button type="submit" disabled={busy || state?.status !== "ready" || !draft.trim()}>Send</button>
      </form>
    </>}
  </section>;
}

type Question = { id?: string; question?: string; header?: string; isSecret?: boolean; multiSelect?: boolean; options?: { label: string; description?: string }[] };
function SessionQuestion({ request, busy, onRespond }: {
  request: Pending; busy: boolean; onRespond: (allow: boolean, answers: Record<string, string[]>) => Promise<boolean>;
}) {
  const [answers, setAnswers] = useState<Record<string, string[]>>({});
  const body = request.params ?? request.request ?? {};
  const input = (body.input ?? {}) as Record<string, unknown>;
  const questions = (body.questions ?? input.questions ?? []) as Question[];
  const title = questions.length ? "Agent needs your input" : `Allow ${String(body.tool_name ?? body.command ?? "this action")}?`;
  return <section className="agent-session-request" aria-label={title}>
    <h3>{title}</h3>
    {questions.length ? questions.map((question, index) => {
      const id = question.id ?? question.question ?? String(index);
      return <fieldset key={id}><legend>{question.question}</legend>
        {question.options?.map((option) => <button type="button" key={option.label} title={option.description} aria-pressed={answers[id]?.includes(option.label) ?? false} onClick={() => setAnswers((prior) => {
          const selected = prior[id] ?? [];
          if (!question.multiSelect) return { ...prior, [id]: [option.label] };
          const next = selected.includes(option.label)
            ? selected.filter((value) => value !== option.label)
            : [...selected, option.label];
          return { ...prior, [id]: next };
        })}>{option.label}</button>)}
        <input type={question.isSecret ? "password" : "text"} autoComplete="off" aria-label={question.question ?? "Answer"} value={answers[id]?.join(", ") ?? ""} onChange={(event) => setAnswers((prior) => ({ ...prior, [id]: [event.target.value] }))} />
      </fieldset>;
    }) : <pre>{display(body.permissions ? { permissions: body.permissions, reason: body.reason, scope: "This turn only" } : body.command ?? body.input ?? body.reason ?? body)}</pre>}
    <button type="button" disabled={busy || questions.some((question, index) => !answers[question.id ?? question.question ?? String(index)]?.some((value) => value.trim()))} onClick={() => void onRespond(true, answers)}>{questions.length ? "Submit answers" : "Allow once"}</button>
    <button type="button" disabled={busy} onClick={() => void onRespond(false, {})}>Decline</button>
  </section>;
}
