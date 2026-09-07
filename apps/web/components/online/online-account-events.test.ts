import { afterEach, expect, it, vi } from "vitest";
import { notifyOnlineAccountChanged, subscribeOnlineAccountChanges } from "./online-account-events";

afterEach(() => vi.unstubAllGlobals());

it("invalidates only the owning account across tabs and removes listeners on disposal", () => {
  const target = new EventTarget();
  const setItem = vi.fn();
  vi.stubGlobal("window", target);
  vi.stubGlobal("localStorage", { setItem });
  const refresh = vi.fn();
  const dispose = subscribeOnlineAccountChanges("owner", refresh);
  try {
    notifyOnlineAccountChanged("other");
    expect(refresh).not.toHaveBeenCalled();
    notifyOnlineAccountChanged("owner");
    expect(refresh).toHaveBeenCalledTimes(1);
    const [key, value] = setItem.mock.calls[1];
    expect(value).toMatch(/^[a-f0-9-]{36}$/);
    target.dispatchEvent(Object.assign(new Event("storage"), { key }));
    expect(refresh).toHaveBeenCalledTimes(2);
    target.dispatchEvent(Object.assign(new Event("storage"), { key: "unrelated" }));
    expect(refresh).toHaveBeenCalledTimes(2);
    setItem.mockImplementation(() => { throw new Error("Storage disabled"); });
    notifyOnlineAccountChanged("owner");
    expect(refresh).toHaveBeenCalledTimes(3);
  } finally {
    dispose();
  }
  notifyOnlineAccountChanged("owner");
  target.dispatchEvent(Object.assign(new Event("storage"), { key: "choruz-online-account-changed:owner" }));
  expect(refresh).toHaveBeenCalledTimes(3);
});
