import { describe, expect, it } from "vitest";
import { requestOrigin, requestUrl } from "./request-origin";
import type { NextRequest } from "next/server";

// Fake the bits of NextRequest we use: `.headers.get(name)` and `.nextUrl`.
function fakeRequest({
  forwardedProto,
  forwardedHost,
  host,
  protocol,
  nextUrlHost,
}: {
  forwardedProto?: string | null;
  forwardedHost?: string | null;
  host?: string | null;
  protocol?: string;
  nextUrlHost?: string;
}): NextRequest {
  const headers = new Map<string, string>();
  if (forwardedProto) headers.set("x-forwarded-proto", forwardedProto);
  if (forwardedHost) headers.set("x-forwarded-host", forwardedHost);
  if (host) headers.set("host", host);
  return {
    headers: { get: (k: string) => headers.get(k.toLowerCase()) ?? null },
    nextUrl: {
      protocol: protocol ?? "http:",
      host: nextUrlHost ?? "localhost:3000",
    },
  } as unknown as NextRequest;
}

describe("requestOrigin — protocol", () => {
  it("uses x-forwarded-proto when present (strips trailing colon)", () => {
    const req = fakeRequest({ forwardedProto: "https", host: "example.com" });
    expect(requestOrigin(req)).toBe("https://example.com");
  });

  it("strips trailing ':' from x-forwarded-proto if a client sent it", () => {
    const req = fakeRequest({ forwardedProto: "https:", host: "example.com" });
    expect(requestOrigin(req)).toBe("https://example.com");
  });

  it("falls back to nextUrl.protocol when x-forwarded-proto is absent", () => {
    const req = fakeRequest({
      protocol: "http:",
      nextUrlHost: "internal:3000",
      host: "h:3000",
    });
    expect(requestOrigin(req)).toMatch(/^http:\/\//);
  });
});

describe("requestOrigin — host", () => {
  it("prefers x-forwarded-host over host header", () => {
    const req = fakeRequest({
      forwardedProto: "https",
      forwardedHost: "edge.example.com",
      host: "internal:3000",
    });
    expect(requestOrigin(req)).toBe("https://edge.example.com");
  });

  it("uses host header when no x-forwarded-host present", () => {
    const req = fakeRequest({ forwardedProto: "https", host: "h.example.com" });
    expect(requestOrigin(req)).toBe("https://h.example.com");
  });

  it("falls back to nextUrl.host when neither header is present", () => {
    const req = fakeRequest({
      forwardedProto: "https",
      nextUrlHost: "fallback:8080",
    });
    expect(requestOrigin(req)).toBe("https://fallback:8080");
  });
});

describe("requestUrl", () => {
  it("resolves a pathname against the request origin", () => {
    const req = fakeRequest({ forwardedProto: "https", host: "api.example.com" });
    const url = requestUrl(req, "/v1/foo?x=1");
    expect(url.toString()).toBe("https://api.example.com/v1/foo?x=1");
  });

  it("returns a URL instance (not a string)", () => {
    const req = fakeRequest({ forwardedProto: "https", host: "api.example.com" });
    expect(requestUrl(req, "/")).toBeInstanceOf(URL);
  });
});
