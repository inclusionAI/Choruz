import { NextRequest } from "next/server";
import { describe, expect, it } from "vitest";

import { GET } from "./route";

describe("GET /auth/session-invalid", () => {
  it("clears the invalid session cookie before returning to local entry", async () => {
    const response = await GET(new NextRequest("http://127.0.0.1:3100/auth/session-invalid"));

    expect(response.status).toBe(303);
    expect(new URL(response.headers.get("location") ?? "about:blank").pathname).toBe("/");
    expect(response.headers.get("set-cookie")).toContain("choruz_session=");
    expect(response.headers.get("set-cookie")).toContain("Expires=Thu, 01 Jan 1970");
  });
});
