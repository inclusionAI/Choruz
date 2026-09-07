const eventName = "choruz-online-account-changed";
const storageKey = (principalId: string) => `${eventName}:${principalId}`;

/** Invalidates account-owned views in this tab and other tabs; carries no credentials. */
export function notifyOnlineAccountChanged(principalId: string) {
  window.dispatchEvent(new CustomEvent(eventName, { detail: principalId }));
  try {
    localStorage.setItem(storageKey(principalId), crypto.randomUUID());
  } catch {
    // Same-tab invalidation still works when browser storage is disabled.
  }
}

export function subscribeOnlineAccountChanges(principalId: string, refresh: () => void) {
  const local = (event: Event) => {
    if ((event as CustomEvent).detail === principalId) refresh();
  };
  const remote = (event: StorageEvent) => {
    if (event.key === storageKey(principalId)) refresh();
  };
  window.addEventListener(eventName, local);
  window.addEventListener("storage", remote);
  return () => {
    window.removeEventListener(eventName, local);
    window.removeEventListener("storage", remote);
  };
}
