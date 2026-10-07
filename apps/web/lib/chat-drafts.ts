function draftStorageKey(principalId: string, scopeId: string): string {
  return `choruz:draft:${principalId}:${scopeId}`;
}

const unavailableStorage = new Map<string, string>();

/** Reads this tab's draft; missing or unavailable session storage yields empty text. */
export function readDraft(principalId: string, scopeId: string): string {
  const key = draftStorageKey(principalId, scopeId);
  if (unavailableStorage.has(key)) return unavailableStorage.get(key)!;
  try {
    return sessionStorage.getItem(key) ?? "";
  } catch {
    return "";
  }
}

/** Stores this tab's draft, or removes it for empty text; storage failures do not block typing. */
export function writeDraft(principalId: string, scopeId: string, content: string): void {
  const key = draftStorageKey(principalId, scopeId);
  try {
    if (content) sessionStorage.setItem(key, content);
    else sessionStorage.removeItem(key);
    unavailableStorage.delete(key);
  } catch {
    unavailableStorage.set(key, content);
  }
}

export type FileDraft = { content: string; original: string };

/** Preserves the save precondition as well as the buffer across editor mounts. */
export function readFileDraft(principalId: string, workspaceId: string | null | undefined, path: string): FileDraft | null {
  try {
    const value = JSON.parse(readDraft(principalId, `file:${workspaceId ?? "local"}:${path}`));
    return typeof value?.content === "string" && typeof value?.original === "string" ? value : null;
  } catch { return null; }
}

export function writeFileDraft(principalId: string, workspaceId: string | null | undefined, path: string, draft: FileDraft | null): void {
  writeDraft(principalId, `file:${workspaceId ?? "local"}:${path}`, draft ? JSON.stringify(draft) : "");
}

export type SessionDraft = { text: string; submissionId: string | null; autoSubmit: boolean };

/** The same pending submission identity survives reload until command acceptance. */
export function readSessionDraft(principalId: string, bindingId: string): SessionDraft | null {
  try {
    const value = JSON.parse(readDraft(principalId, `session:${bindingId}`));
    return typeof value?.text === "string" && typeof value?.autoSubmit === "boolean"
      && (value.submissionId === null || typeof value.submissionId === "string") ? value : null;
  } catch { return null; }
}

export function writeSessionDraft(principalId: string, bindingId: string, draft: SessionDraft | null): void {
  writeDraft(principalId, `session:${bindingId}`, draft ? JSON.stringify(draft) : "");
}
