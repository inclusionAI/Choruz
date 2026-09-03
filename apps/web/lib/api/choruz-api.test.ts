import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  ApiRequestError,
  apiBaseUrl,
  decodeSessionClaims,
  fetchConsoleSnapshot,
  sessionCookieName,
} from "./choruz-api";

// sessionCookieName -----------------------------------------------------------

describe("sessionCookieName", () => {
  it("returns the canonical session cookie name", () => {
    expect(sessionCookieName()).toBe("choruz_session");
  });
});

// apiBaseUrl ------------------------------------------------------------------

describe("apiBaseUrl", () => {
  let prevBaseEnv: string | undefined;
  let prevUrlEnv: string | undefined;
  let prevPortEnv: string | undefined;

  beforeEach(() => {
    prevBaseEnv = process.env.CHORUZ_API_BASE_URL;
    prevUrlEnv = process.env.CHORUZ_API_URL;
    prevPortEnv = process.env.CHORUZ_API_PORT;
  });

  afterEach(() => {
    if (prevBaseEnv === undefined) delete process.env.CHORUZ_API_BASE_URL;
    else process.env.CHORUZ_API_BASE_URL = prevBaseEnv;
    if (prevUrlEnv === undefined) delete process.env.CHORUZ_API_URL;
    else process.env.CHORUZ_API_URL = prevUrlEnv;
    if (prevPortEnv === undefined) delete process.env.CHORUZ_API_PORT;
    else process.env.CHORUZ_API_PORT = prevPortEnv;
    vi.unstubAllGlobals();
  });

  it("returns '/api' when running in a browser (window defined)", () => {
    vi.stubGlobal("window", {});
    expect(apiBaseUrl()).toBe("/api");
  });

  it("returns the gateway URL on the server side from env var", () => {
    vi.stubGlobal("window", undefined);
    process.env.CHORUZ_API_BASE_URL = "http://gateway.test:9000";
    expect(apiBaseUrl()).toBe("http://gateway.test:9000");
  });

  it("falls back to CHORUZ_API_URL when the canonical base env var is unset", () => {
    vi.stubGlobal("window", undefined);
    delete process.env.CHORUZ_API_BASE_URL;
    process.env.CHORUZ_API_URL = "http://gateway-legacy.test:9001";
    expect(apiBaseUrl()).toBe("http://gateway-legacy.test:9001");
  });

  it("uses CHORUZ_API_PORT when only a port override is set", () => {
    vi.stubGlobal("window", undefined);
    delete process.env.CHORUZ_API_BASE_URL;
    delete process.env.CHORUZ_API_URL;
    process.env.CHORUZ_API_PORT = "3011";
    expect(apiBaseUrl()).toBe("http://127.0.0.1:3011");
  });

  it("falls back to localhost:3000 when no env var is set", () => {
    vi.stubGlobal("window", undefined);
    delete process.env.CHORUZ_API_BASE_URL;
    delete process.env.CHORUZ_API_URL;
    delete process.env.CHORUZ_API_PORT;
    expect(apiBaseUrl()).toBe("http://127.0.0.1:3000");
  });

  it("trims whitespace-only env values back to the localhost default", () => {
    vi.stubGlobal("window", undefined);
    process.env.CHORUZ_API_BASE_URL = "   ";
    delete process.env.CHORUZ_API_URL;
    delete process.env.CHORUZ_API_PORT;
    expect(apiBaseUrl()).toBe("http://127.0.0.1:3000");
  });
});

// decodeSessionClaims ---------------------------------------------------------

function makeToken(payload: object): string {
  // Approximate the gateway's token format: base64url(JSON) + "." + signature.
  // The function only inspects the payload portion (it's signature-naive),
  // matching the documented "do not trust without round-tripping" contract.
  const json = JSON.stringify(payload);
  const b64 = Buffer.from(json, "utf-8").toString("base64");
  const b64url = b64.replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
  return `${b64url}.fake-signature`;
}

describe("decodeSessionClaims", () => {
  it("parses a base64url-encoded JSON payload", () => {
    const claims = {
      principal_id: "p1",
      workspace_id: "ws",
      display_name: "Alice",
      principal_type: "human" as const,
      exp: 1_900_000_000,
    };
    const token = makeToken(claims);
    const out = decodeSessionClaims(token);
    expect(out).toMatchObject(claims);
  });

  it("returns null for malformed (non-JSON) payload", () => {
    expect(decodeSessionClaims("not-base64.signature")).toBeNull();
  });

  it("returns null for empty input", () => {
    expect(decodeSessionClaims("")).toBeNull();
  });

  it("handles base64url charset (- and _ instead of + and /)", () => {
    // Pick payload whose JSON contains chars that map to '+' / '/' in base64.
    const claims = { principal_id: "??>", workspace_id: "??", display_name: "x", principal_type: "agent", exp: 0 };
    const token = makeToken(claims);
    expect(token).toMatch(/[-_]/); // sanity: we have base64url chars
    expect(decodeSessionClaims(token)).toMatchObject(claims);
  });

  it("tolerates missing padding in the encoded payload", () => {
    // Force a payload whose base64 length is not a multiple of 4.
    const claims = { principal_id: "p", workspace_id: "w", display_name: "abc", principal_type: "human", exp: 0 };
    const token = makeToken(claims).replace(/=+(?=\.)/, "");
    expect(decodeSessionClaims(token)).toMatchObject(claims);
  });
});

describe("gateway error responses", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("preserves the HTTP status for authentication recovery", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(
      JSON.stringify({ error: { detail: "invalid or expired session" } }),
      { status: 401, headers: { "content-type": "application/json" } },
    )));

    await expect(fetchConsoleSnapshot("expired-token")).rejects.toEqual(
      expect.objectContaining<ApiRequestError>({
        name: "ApiRequestError",
        status: 401,
        message: "invalid or expired session",
      }),
    );
  });
});
