import { readFileSync } from "node:fs";
import { DatabaseSync } from "node:sqlite";
import { URL as NodeURL } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { onlineAuth, onlineAuthResponse } from "./online-auth";

const origin = "https://online.test";
const secret = "integration-test-secret-not-used-outside-this-test";
const databases: DatabaseSync[] = [];
type AuthResult = { token: string; user: { id: string } };
async function authResult(response: Response): Promise<AuthResult> {
  return await response.json() as AuthResult;
}
afterEach(() => { for (const db of databases.splice(0)) db.close(); });

function fixture() {
  const db = new DatabaseSync(":memory:");
  databases.push(db);
  db.exec(readFileSync(new NodeURL("../migrations/0001_online_accounts.sql", import.meta.url), "utf8"));
  const auth = onlineAuth(db, secret, origin);
  const request = (action: string, body?: object, token?: string) => auth.handler(new Request(
    `${origin}/v1/online/auth/${action}`,
    { method: body ? "POST" : "GET", headers: { "content-type": "application/json", ...(token ? { authorization: `Bearer ${token}` } : {}) }, body: body ? JSON.stringify(body) : undefined },
  ));
  return { db, request };
}

describe("Online credentials with the production auth implementation and SQL schema", () => {
  it("hashes passwords, rejects wrong credentials and revokes only the signed-out session", async () => {
    const { db, request } = fixture();
    const credentials = { email: "person@example.test", password: "a-long-test-password", name: "Person" };
    const signup = await request("sign-up/email", credentials);
    expect(signup.status).toBe(200);
    const first = await authResult(signup);
    const row = db.prepare('SELECT password FROM account').get();
    expect(row?.password).toBeTruthy();
    expect(row?.password).not.toBe(credentials.password);
    expect((await request("sign-in/email", { ...credentials, password: "wrong-password" })).status).toBe(401);
    const login = await request("sign-in/email", credentials);
    expect(login.status).toBe(200);
    const second = await authResult(login);
    expect(second.token).not.toBe(first.token);
    expect((await authResult(await request("get-session", undefined, first.token))).user.id).toBe(first.user.id);
    expect((await request("sign-out", {}, first.token)).status).toBe(200);
    expect(await (await request("get-session", undefined, first.token)).json()).toBeNull();
    expect((await authResult(await request("get-session", undefined, second.token))).user.id).toBe(first.user.id);
  });

  it("does not turn another user's token or an arbitrary bearer into this identity", async () => {
    const { request } = fixture();
    const a = await authResult(await request("sign-up/email", { email: "a@example.test", name: "A", password: "long-password-for-a" }));
    const b = await authResult(await request("sign-up/email", { email: "b@example.test", name: "B", password: "long-password-for-b" }));
    expect(a.user.id).not.toBe(b.user.id);
    expect((await authResult(await request("get-session", undefined, b.token))).user.id).toBe(b.user.id);
    expect(await (await request("get-session", undefined, "invalid-token")).json()).toBeNull();
  });

  it("does not expose extra auth routes and fails closed when unconfigured", async () => {
    expect((await onlineAuthResponse(new Request(`${origin}/v1/online/auth/delete-user`, { method: "POST" }), {})).status).toBe(404);
    expect((await onlineAuthResponse(new Request(`${origin}/v1/online/auth/get-session`), {})).status).toBe(503);
  });

  it("rejects expired sessions using the stored expiry", async () => {
    const { db, request } = fixture();
    const created = await authResult(await request("sign-up/email", { email: "expired@example.test", name: "Expired", password: "long-expiry-test-password" }));
    db.prepare('UPDATE session SET expiresAt=0 WHERE token=?').run(created.token);
    expect(await (await request("get-session", undefined, created.token)).json()).toBeNull();
  });

  it("bounds streamed bodies without trusting content-length", async () => {
    let cancelled = false;
    const body = new ReadableStream<Uint8Array>({
      pull(controller) { controller.enqueue(new Uint8Array(4096)); },
      cancel() { cancelled = true; },
    });
    const request = new Request(`${origin}/v1/online/auth/sign-in/email`, { method: "POST", body, duplex: "half" } as RequestInit);
    const response = await onlineAuthResponse(request, { ONLINE_DATABASE: {} as D1Database, ONLINE_AUTH_SECRET: secret });
    expect(response.status).toBe(413);
    expect(cancelled).toBe(true);
  });
});
