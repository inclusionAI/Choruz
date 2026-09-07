import { describe, expect, it } from "vitest";
import { mergeSessionPage } from "./agent-session-view";

const item = (id: string, position: number, revision: number, text: string) => ({ id, position, revision, text, kind: "assistant", status: "completed", detail: {} });
const page = (revision: number, items: ReturnType<typeof item>[], instance = "process-one") => ({ instance, revision, cursor: revision, more: false, session_id: "native", status: "ready", items, requests: [], error: null });

describe("structured session replay", () => {
  it("evicts old positions when the server advances its bounded history window", () => {
    const old = page(5, [item("a", 0, 1, "Old"), item("b", 1, 4, "Retained")]);
    const next = mergeSessionPage(old, {...page(6, []), retained_from:1, history_truncated:true});
    expect(next.items.map((row) => row.id)).toEqual(["b"]);
    expect(mergeSessionPage(next, old).items.map((row) => row.id)).toEqual(["b"]);
    expect(next.history_truncated).toBe(true);
  });
  it("replaces streamed rows without losing earlier pages or transcript order", () => {
    const first = page(5, [item("a", 0, 2, "Hello"), item("b", 1, 4, "Tool result")]);
    const next = mergeSessionPage(first, page(9, [item("a", 0, 8, "Hello world")]));
    expect(next.items.map((row) => row.text)).toEqual(["Hello world", "Tool result"]);
    expect(mergeSessionPage(next, first)).toEqual(next);
  });

  it("does not regress a pending approval when an older command reply arrives", () => {
    const latest = { ...page(12, [item("a", 0, 9, "Waiting")]), status: "waiting", requests: [{ id: 3, method: "item/fileChange/requestApproval" }] };
    expect(mergeSessionPage(latest, page(10, []))).toEqual(latest);
  });

  it("starts a separate replay after the owner process restarts", () => {
    const first = page(90, [item("old", 0, 80, "Old process")]);
    const fresh = page(2, [item("new", 0, 1, "Restored native history")], "process-two");
    expect(mergeSessionPage(first, fresh)).toEqual(fresh);
  });
});
