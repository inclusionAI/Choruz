import { mkdtempSync, mkdirSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { afterEach, describe, expect, it } from "vitest";

import { postgresDatabaseUrl } from "./group-provisioning-db";

const ORIGINAL_ENV = {
  CHORUZ_DATABASE_URL: process.env.CHORUZ_DATABASE_URL,
  CHORUZ_PG_HOST: process.env.CHORUZ_PG_HOST,
  CHORUZ_PG_PORT: process.env.CHORUZ_PG_PORT,
  CHORUZ_PG_DB: process.env.CHORUZ_PG_DB,
  CHORUZ_PG_USER: process.env.CHORUZ_PG_USER,
  ECHAT_PG_DB: process.env.ECHAT_PG_DB,
  ECHAT_PG_USER: process.env.ECHAT_PG_USER,
  USER: process.env.USER,
};
const ORIGINAL_CWD = process.cwd();

describe("postgresDatabaseUrl", () => {
  afterEach(() => {
    process.chdir(ORIGINAL_CWD);
    restoreEnv();
  });

  it("prefers the explicit database URL", () => {
    clearDbEnv();
    process.env.CHORUZ_DATABASE_URL = "postgres://explicit@db.example/choruz";

    expect(postgresDatabaseUrl()).toBe("postgres://explicit@db.example/choruz");
  });

  it("falls back to infra/host/.env when direct web dev does not export DB env", () => {
    clearDbEnv();
    process.env.USER = "dev-user";
    const root = mkdtempSync(path.join(tmpdir(), "choruz-host-env-"));
    mkdirSync(path.join(root, "apps", "web"), { recursive: true });
    mkdirSync(path.join(root, "infra", "host"), { recursive: true });
    writeFileSync(path.join(root, "infra", "host", ".env"), [
      "CHORUZ_PG_HOST=127.0.0.1",
      "CHORUZ_PG_PORT=55442",
      "CHORUZ_PG_DB=choruz",
      "CHORUZ_PG_USER=",
    ].join("\n"));
    process.chdir(path.join(root, "apps", "web"));

    expect(postgresDatabaseUrl()).toBe("postgres://dev-user@127.0.0.1:55442/choruz");
  });

  it("ignores legacy database component variables", () => {
    clearDbEnv();
    delete process.env.USER;
    process.env.ECHAT_PG_DB = "legacy";
    process.env.ECHAT_PG_USER = "legacy-user";
    const root = mkdtempSync(path.join(tmpdir(), "choruz-empty-host-env-"));
    mkdirSync(path.join(root, "apps", "web"), { recursive: true });
    process.chdir(path.join(root, "apps", "web"));

    expect(postgresDatabaseUrl()).toBe("postgres://choruz@127.0.0.1:5432/choruz");
  });
});

function clearDbEnv() {
  delete process.env.CHORUZ_DATABASE_URL;
  delete process.env.CHORUZ_PG_HOST;
  delete process.env.CHORUZ_PG_PORT;
  delete process.env.CHORUZ_PG_DB;
  delete process.env.CHORUZ_PG_USER;
}

function restoreEnv() {
  for (const [key, value] of Object.entries(ORIGINAL_ENV)) {
    if (value === undefined) {
      delete process.env[key];
    } else {
      process.env[key] = value;
    }
  }
}
