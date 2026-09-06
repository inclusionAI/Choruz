import { afterEach, describe, expect, it, vi } from "vitest";
import {
  activeTransport,
  gatewaySocketUrl,
  localTransport,
  setActiveTransport,
  setRequestObserver,
  transportFetch,
  type ChoruzTransport,
} from "./transport";

afterEach(() => {
  setActiveTransport(null);
  setRequestObserver(null);
  vi.unstubAllGlobals();
});

describe("gatewaySocketUrl", () => {
  it("uses the page's host and the configured API port", () => {
    vi.stubGlobal("window", { location: { protocol: "http:", hostname: "192.168.1.9" } });
    expect(gatewaySocketUrl("/v1/ws/sync?cursor=1")).toBe("ws://192.168.1.9:3000/v1/ws/sync?cursor=1");
  });

  it("upgrades to wss on https pages", () => {
    vi.stubGlobal("window", { location: { protocol: "https:", hostname: "choruz.local" } });
    expect(gatewaySocketUrl("/v1/ws/terminals/b1")).toMatch(/^wss:\/\/choruz\.local:3000\/v1\/ws\/terminals\/b1$/);
  });

  it("prefers an explicit gateway origin", () => {
    expect(gatewaySocketUrl("/v1/ws/sync", "https://gateway.example")).toBe("wss://gateway.example/v1/ws/sync");
    expect(gatewaySocketUrl("/v1/ws/sync", "http://10.0.0.2:3000")).toBe("ws://10.0.0.2:3000/v1/ws/sync");
  });
});

describe("active transport", () => {
  it("correlates responses and cancellation without consuming the response body", async () => {
    vi.stubGlobal("window", {});
    const finish = vi.fn();
    setRequestObserver(() => ({ traceId: "owned-trace", finish }));
    const response = new Response("original body", { status: 503 });
    const fetch = vi.fn().mockResolvedValueOnce(response).mockRejectedValueOnce(new DOMException("aborted", "AbortError"));
    setActiveTransport({ fetch, socket: vi.fn() });
    const result = await transportFetch("/api/v1/me");
    expect(await result.text()).toBe("original body");
    expect(new Headers(fetch.mock.calls[0][1].headers).get("x-trace-id")).toBe("owned-trace");
    expect(finish).toHaveBeenLastCalledWith({ status: 503, outcome: "failed" });
    await expect(transportFetch("/api/v1/me")).rejects.toThrow("aborted");
    expect(finish).toHaveBeenLastCalledWith({ outcome: "cancelled" });
  });
  it("is the local one until another is installed, and again after reset", () => {
    expect(activeTransport()).toBe(localTransport);
    const fake: ChoruzTransport = { fetch: vi.fn(), socket: vi.fn() };
    setActiveTransport(fake);
    expect(activeTransport()).toBe(fake);
    setActiveTransport(null);
    expect(activeTransport()).toBe(localTransport);
  });

  it("routes transportFetch through the installed transport in the browser", async () => {
    vi.stubGlobal("window", { location: { protocol: "http:", hostname: "localhost" } });
    const response = new Response("ok");
    const fake: ChoruzTransport = { fetch: vi.fn().mockResolvedValue(response), socket: vi.fn() };
    setActiveTransport(fake);
    await expect(transportFetch("/api/v1/me", { method: "GET" })).resolves.toBe(response);
    expect(fake.fetch).toHaveBeenCalledWith("/api/v1/me", { method: "GET" });
  });

  it("lets the local transport pick up a stubbed global fetch at call time", async () => {
    vi.stubGlobal("window", { location: { protocol: "http:", hostname: "localhost" } });
    const stub = vi.fn().mockResolvedValue(new Response("stubbed"));
    vi.stubGlobal("fetch", stub);
    const response = await transportFetch("/api/v1/me");
    expect(stub).toHaveBeenCalledWith("/api/v1/me", undefined);
    await expect(response.text()).resolves.toBe("stubbed");
  });
});
