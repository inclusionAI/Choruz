import "fake-indexeddb/auto";
import { afterEach, expect, it, vi } from "vitest";
import { initTrace, trace } from "./choruz-trace";
import { localTransport, setActiveTransport } from "./transport";

afterEach(() => { setActiveTransport(null); vi.unstubAllGlobals(); vi.restoreAllMocks(); });

it("retains the destination of an authenticated scope across a transport switch", async () => {
  vi.stubGlobal("window", {});
  const wrongHost = vi.fn();
  const delivery = vi.fn().mockResolvedValue(new Response(null, { status: 204 }));
  setActiveTransport({ ...localTransport, fetch: delivery });
  const stop = initTrace("owned-session", "", crypto.randomUUID());
  try {
    trace.event("owned_save");
    setActiveTransport({ ...localTransport, fetch: wrongHost });
    await vi.waitFor(() => expect(delivery).toHaveBeenCalledOnce(), { timeout: 4000 });
    expect(wrongHost).not.toHaveBeenCalled();
  } finally { stop(); }
});
