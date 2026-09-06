import { betterAuth, type BetterAuthOptions } from "better-auth";
import { bearer } from "better-auth/plugins/bearer";

/** Online identity is independent of room capabilities and grants no runtime access. */
export function onlineAuth(database: BetterAuthOptions["database"], secret: string, origin: string) {
  return betterAuth({
    database,
    secret,
    baseURL: origin,
    basePath: "/v1/online/auth",
    trustedOrigins: [origin],
    emailAndPassword: { enabled: true, minPasswordLength: 12, maxPasswordLength: 128 },
    session: { expiresIn: 60 * 60 * 24 * 7, updateAge: 60 * 60 * 24 },
    rateLimit: { enabled: true, storage: "database", window: 60, max: 60 },
    advanced: { ipAddress: { ipAddressHeaders: ["cf-connecting-ip"] } },
    plugins: [bearer()],
  });
}

export interface OnlineAuthEnv {
  ONLINE_DATABASE?: D1Database;
  ONLINE_AUTH_SECRET?: string;
}

/** Only account/session operations are public; Better Auth owns hashing and revocation. */
export async function onlineAuthResponse(request: Request, env: OnlineAuthEnv): Promise<Response> {
  const url = new URL(request.url);
  const action = url.pathname.slice("/v1/online/auth/".length);
  const methods: Record<string, string> = {
    "sign-up/email": "POST", "sign-in/email": "POST", "get-session": "GET", "sign-out": "POST",
  };
  if (methods[action] !== request.method) return new Response("Not found", { status: 404 });
  if (!env.ONLINE_DATABASE || !env.ONLINE_AUTH_SECRET || env.ONLINE_AUTH_SECRET.length < 32) {
    return Response.json({ message: "Online account service is not configured" }, { status: 503 });
  }
  if (Number(request.headers.get("content-length") ?? 0) > 8192) return new Response("Request too large", { status: 413 });
  let body: string | undefined;
  if (request.method === "POST" && request.body) {
    const reader = request.body.getReader();
    const chunks: Uint8Array[] = [];
    let size = 0;
    try {
      while (true) {
        const { value, done } = await reader.read();
        if (done) break;
        size += value.byteLength;
        if (size > 8192) {
          await reader.cancel();
          return new Response("Request too large", { status: 413 });
        }
        chunks.push(value);
      }
    } finally {
      reader.releaseLock();
    }
    const bytes = new Uint8Array(size);
    let offset = 0;
    for (const chunk of chunks) {
      bytes.set(chunk, offset);
      offset += chunk.byteLength;
    }
    body = new TextDecoder().decode(bytes);
  }
  const auth = onlineAuth(env.ONLINE_DATABASE, env.ONLINE_AUTH_SECRET, url.origin);
  const response = await auth.handler(new Request(request.url, { method: request.method, headers: request.headers, body }));
  const headers = new Headers(response.headers);
  headers.set("cache-control", "no-store");
  return new Response(response.body, { status: response.status, headers });
}
