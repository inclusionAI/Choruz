import { afterEach, expect, it, vi } from "vitest";
import { readFileDraft, readSessionDraft, writeFileDraft, writeSessionDraft } from "./chat-drafts";

afterEach(() => vi.unstubAllGlobals());

function storage() {
  const values = new Map<string, string>();
  vi.stubGlobal("sessionStorage", {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
    removeItem: (key: string) => values.delete(key),
  });
  return values;
}

it("retains file content and its original precondition separately for each owner and project", () => {
  storage();
  const draft = { content: "Unsaved changes", original: "Observed disk content" };
  writeFileDraft("editor-owner", "project-one", "/work/file", draft);
  expect(readFileDraft("editor-owner", "project-one", "/work/file")).toEqual(draft);
  expect(readFileDraft("other-owner", "project-one", "/work/file")).toBeNull();
  expect(readFileDraft("editor-owner", "project-two", "/work/file")).toBeNull();
  writeFileDraft("editor-owner", "project-one", "/work/file", null);
  expect(readFileDraft("editor-owner", "project-one", "/work/file")).toBeNull();
});

it("restores pending task identity across a module reload until acceptance clears it", async () => {
  const values = storage();
  const draft = { text: "Inspect the workspace", submissionId: "task-id", autoSubmit: true };
  writeSessionDraft("task-owner", "binding-one", draft);
  vi.resetModules();
  const reloaded = await import("./chat-drafts");
  expect(reloaded.readSessionDraft("task-owner", "binding-one")).toEqual(draft);
  expect(reloaded.readSessionDraft("task-owner", "binding-two")).toBeNull();
  reloaded.writeSessionDraft("task-owner", "binding-one", null);
  expect(values.size).toBe(0);
  expect(reloaded.readSessionDraft("task-owner", "binding-one")).toBeNull();
});

it("keeps edits across navigation when browser storage rejects writes", () => {
  vi.stubGlobal("sessionStorage", { getItem: () => null, setItem: () => { throw new Error("quota"); }, removeItem: () => { throw new Error("quota"); } });
  writeFileDraft("quota-owner", "project", "/file", { content: "Keep this", original: "Original" });
  expect(readFileDraft("quota-owner", "project", "/file")?.content).toBe("Keep this");
  writeFileDraft("quota-owner", "project", "/file", null);
  expect(readFileDraft("quota-owner", "project", "/file")).toBeNull();
});
