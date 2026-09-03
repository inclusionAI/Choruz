import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  createChannelTaskFromMessage,
  createCompany,
  deleteCompany,
  fetchCompanies,
  fetchChannelTask,
  fetchChannelTasks,
  fetchCompany,
  localLogin,
  localSignup,
  patchChannelTask,
  revokeRuntimeHost,
} from "./choruz-api";

// Helpers ---------------------------------------------------------------------

type MockResponse = {
  ok: boolean;
  status: number;
  statusText?: string;
  json: () => Promise<unknown>;
};

function jsonResponse(body: unknown, status = 200): MockResponse {
  return {
    ok: status >= 200 && status < 300,
    status,
    statusText: status >= 400 ? "Error" : "OK",
    json: async () => body,
  };
}

function captureFetch(impl: (url: string, init?: RequestInit) => MockResponse) {
  const fn = vi.fn((url: string, init?: RequestInit) => Promise.resolve(impl(url, init)));
  vi.stubGlobal("fetch", fn);
  return fn;
}

beforeEach(() => {
  // Force the server-side branch so apiBaseUrl() returns a deterministic prefix.
  vi.stubGlobal("window", undefined);
  process.env.CHORUZ_API_BASE_URL = "http://gateway.test";
});

afterEach(() => {
  vi.unstubAllGlobals();
  delete process.env.CHORUZ_API_BASE_URL;
});

// URL construction + method + body -------------------------------------------

describe("apiJson — URL construction", () => {
  it("prefixes the path with apiBaseUrl()", async () => {
    const fn = captureFetch(() => jsonResponse([]));
    await fetchCompanies("session-1");
    expect(fn).toHaveBeenCalledWith(
      "http://gateway.test/v1/companies",
      expect.any(Object),
    );
  });

  it("interpolates path parameters into the URL", async () => {
    const fn = captureFetch(() => jsonResponse({ id: "c1", name: "X" }));
    await fetchCompany("session", "c1");
    expect(fn.mock.calls[0][0]).toBe("http://gateway.test/v1/companies/c1");
  });
});

describe("apiJson — auth + content-type headers", () => {
  it("sets Authorization Bearer header when sessionToken is provided", async () => {
    const fn = captureFetch(() => jsonResponse([]));
    await fetchCompanies("token-abc");
    const init = fn.mock.calls[0][1] as RequestInit;
    const auth = (init.headers as Headers).get("authorization");
    expect(auth).toBe("Bearer token-abc");
  });

  it("auto-sets content-type=application/json when a body is present", async () => {
    const fn = captureFetch(() => jsonResponse({ id: "c1" }));
    await createCompany("t", { actor_id: "actor-1", name: "Acme" });
    const init = fn.mock.calls[0][1] as RequestInit;
    expect((init.headers as Headers).get("content-type")).toBe("application/json");
  });

  it("does NOT set content-type when there is no body", async () => {
    const fn = captureFetch(() => jsonResponse([]));
    await fetchCompanies("t");
    const init = fn.mock.calls[0][1] as RequestInit;
    // GET, no body
    expect(init.body).toBeUndefined();
    expect((init.headers as Headers).get("content-type")).toBeNull();
  });

  it("uses no-store cache to avoid stale auth responses", async () => {
    const fn = captureFetch(() => jsonResponse([]));
    await fetchCompanies("t");
    const init = fn.mock.calls[0][1] as RequestInit;
    expect(init.cache).toBe("no-store");
  });
});

// Method overrides -----------------------------------------------------------

describe("apiJson — methods", () => {
  it("uses POST for localLogin and forwards JSON body", async () => {
    const fn = captureFetch(() => jsonResponse({ token: "tok", claims: {} }));
    await localLogin("alice", "pw");
    const init = fn.mock.calls[0][1] as RequestInit;
    expect(init.method).toBe("POST");
    expect(JSON.parse(init.body as string)).toEqual({ username: "alice", password: "pw" });
  });

  it("uses POST for localSignup", async () => {
    const fn = captureFetch(() => jsonResponse({ token: "tok", claims: {} }));
    await localSignup("alice", "pw");
    const init = fn.mock.calls[0][1] as RequestInit;
    expect(init.method).toBe("POST");
  });

  it("uses DELETE for deleteCompany", async () => {
    const fn = captureFetch(() => jsonResponse({}));
    await deleteCompany("t", "c1");
    const init = fn.mock.calls[0][1] as RequestInit;
    expect(init.method).toBe("DELETE");
  });

  it("accepts an empty 204 response when revoking a runtime host", async () => {
    const fn = captureFetch(() => ({
      ok: true,
      status: 204,
      statusText: "No Content",
      json: async () => { throw new Error("empty response"); },
    }));

    await expect(revokeRuntimeHost("t", "host/1")).resolves.toBeUndefined();
    expect(fn.mock.calls[0][0]).toBe("http://gateway.test/v1/runtime-hosts/host%2F1");
    expect((fn.mock.calls[0][1] as RequestInit).method).toBe("DELETE");
  });

  it("uses channel task read and human mutation routes without workflow-task helpers", async () => {
    const fn = captureFetch((url, init) => {
      if (String(url).includes("/tasks/from-message")) {
        expect(init?.method).toBe("POST");
        expect(JSON.parse(init?.body as string)).toEqual({
          message_id: "m1",
          title: "Track this",
          assignee_principal_id: "agent-1",
          context_label: null,
          idempotency_key: "idem-1",
        });
      }
      if (String(url).endsWith("/v1/tasks/task-1")) {
        if (init?.method === "PATCH") {
          expect(JSON.parse(init.body as string)).toEqual({
            status: "blocked",
            blocked_reason: "Waiting",
          });
        }
        return jsonResponse({ task_id: "task-1", events: [], task: { task_id: "task-1" } });
      }
      return jsonResponse([]);
    });

    await fetchChannelTasks("t", "conv/1");
    await fetchChannelTask("t", "task-1");
    await createChannelTaskFromMessage("t", "conv/1", {
      message_id: "m1",
      title: "Track this",
      assignee_principal_id: "agent-1",
      context_label: null,
      idempotency_key: "idem-1",
    });
    await patchChannelTask("t", "task-1", {
      status: "blocked",
      blocked_reason: "Waiting",
    });

    expect(fn.mock.calls.map((call) => call[0])).toEqual([
      "http://gateway.test/v1/conversations/conv%2F1/tasks",
      "http://gateway.test/v1/tasks/task-1",
      "http://gateway.test/v1/conversations/conv%2F1/tasks/from-message",
      "http://gateway.test/v1/tasks/task-1",
    ]);
    expect(fn.mock.calls.some((call) => String(call[0]).includes("workflow-tasks"))).toBe(false);
  });
});

// Error handling -------------------------------------------------------------

describe("apiJson — error responses", () => {
  it("throws with payload.error.detail when error is an object", async () => {
    captureFetch(() => jsonResponse({ error: { detail: "company not found" } }, 404));
    await expect(fetchCompanies("t")).rejects.toThrow("company not found");
  });

  it("throws with payload.error when error is a string", async () => {
    captureFetch(() => jsonResponse({ error: "rate limit exceeded" }, 429));
    await expect(fetchCompanies("t")).rejects.toThrow("rate limit exceeded");
  });

  it("falls back to status + statusText when error body cannot be parsed", async () => {
    captureFetch(() => ({
      ok: false,
      status: 500,
      statusText: "Server Error",
      json: async () => {
        throw new Error("not json");
      },
    }));
    await expect(fetchCompanies("t")).rejects.toThrow(/500.*Server Error/);
  });

  it("returns parsed JSON when response is ok", async () => {
    captureFetch(() => jsonResponse([{ id: "c1", name: "Acme" }]));
    const result = await fetchCompanies("t");
    expect(Array.isArray(result)).toBe(true);
    expect(result[0]).toMatchObject({ id: "c1", name: "Acme" });
  });
});
