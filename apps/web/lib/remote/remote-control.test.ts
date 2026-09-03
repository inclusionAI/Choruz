import { describe, expect, it } from "vitest";

import { parsePairingCredential, remoteDashboardPath } from "./remote-control";

const CREDENTIAL = "v1.AAAAAAAAAAAAAAAAAAAAAA.BBBBBBBBBBBBBBBBBBBBBB";

describe("remoteDashboardPath", () => {
  it("keeps the credential in the fragment so HTTP requests cannot disclose it", () => {
    expect(remoteDashboardPath("https://gateway.example/base", CREDENTIAL, "Work laptop"))
      .toBe(`/remote?gateway=https%3A%2F%2Fgateway.example%2Fbase&device_name=Work+laptop#credential=${CREDENTIAL}`);
  });

  it("rejects an incomplete credential or a missing gateway", () => {
    expect(() => remoteDashboardPath("https://gateway.example", "v1.short", "Laptop"))
      .toThrow("Paste the complete pairing credential.");
    expect(() => remoteDashboardPath(" ", CREDENTIAL, "Laptop"))
      .toThrow("Cloud Gateway is not configured.");
  });

  it("parses only the versioned 128-bit identifier and secret", () => {
    expect(parsePairingCredential(CREDENTIAL)).toEqual({
      value: CREDENTIAL,
      id: "AAAAAAAAAAAAAAAAAAAAAA",
      secret: "BBBBBBBBBBBBBBBBBBBBBB",
    });
    expect(() => parsePairingCredential("12345678")).toThrow();
  });
});
