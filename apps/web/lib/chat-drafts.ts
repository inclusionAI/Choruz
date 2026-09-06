function draftStorageKey(principalId: string, scopeId: string): string {
  return `choruz:draft:${principalId}:${scopeId}`;
}

/** Reads this tab's draft; missing or unavailable session storage yields empty text. */
export function readDraft(principalId: string, scopeId: string): string {
  try {
    return sessionStorage.getItem(draftStorageKey(principalId, scopeId)) ?? "";
  } catch {
    return "";
  }
}

/** Stores this tab's draft, or removes it for empty text; storage failures do not block typing. */
export function writeDraft(principalId: string, scopeId: string, content: string): void {
  try {
    if (content) sessionStorage.setItem(draftStorageKey(principalId, scopeId), content);
    else sessionStorage.removeItem(draftStorageKey(principalId, scopeId));
  } catch {
    // Storage can be unavailable in privacy-restricted browser contexts.
  }
}
