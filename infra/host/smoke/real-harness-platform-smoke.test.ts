import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import test from "node:test";
import { promisify } from "node:util";
import { HttpError, ResultBook, classifyHttpError, groupScenario, hasTrackedActiveBinding, hasTrackedResource, importContinuityScenario, isExplicitHarnessAuthError, openCodeSeedInvocation, parseClaudeSeed, parseCodexSeed, parseGrokSeed, parseOpenCodeSeed, parsePiSeed } from "./real-harness-platform-smoke.ts";

const execFileAsync = promisify(execFile);

test("the complete five-Harness scenario matrix fails closed", () => {
  const book = new ResultBook(["claude", "codex", "pi", "grok", "opencode"]);
  const harnessEntries = book.entries().filter((entry) => entry.harness !== "platform");
  assert.equal(harnessEntries.length, 25);
  assert.ok(harnessEntries.every((entry) => entry.verdict === "FAIL" && entry.reason === "scenario-not-executed"));
  assert.equal(book.exitCode(), 1);
});

test("blocked and skipped results never exit successfully", () => {
  const book = new ResultBook(["pi"]); book.fillHarness("pi", "BLOCKED", "node-version-unavailable");
  book.set("platform", "authentication", "PASS", "operator-auth-confirmed"); book.set("platform", "setup", "PASS", "setup-complete"); book.set("platform", "cleanup", "PASS", "cleanup-complete");
  assert.equal(book.exitCode(), 3);
});

test("spawn errors clear command timers and stdin errors in an isolated process", async () => {
  const moduleUrl = new URL("./real-harness-platform-smoke.ts", import.meta.url).href;
  const script = `
    import { runCommand } from ${JSON.stringify(moduleUrl)};
    try {
      await runCommand("choruz-definitely-missing-command", [], {
        cwd: ${JSON.stringify(process.cwd())},
        input: "input that must not trigger an unhandled stdin error",
        timeoutMs: 10_000,
      });
      process.exitCode = 2;
    } catch (error) {
      if (error?.code !== "ENOENT") throw error;
    }
  `;
  await execFileAsync(
    process.execPath,
    ["--disable-warning=ExperimentalWarning", "--experimental-strip-types", "--input-type=module", "--eval", script],
    { timeout: 2_000 },
  );
});

test("only explicit HTTP auth failures are blocked", () => {
  assert.deepEqual(classifyHttpError(new HttpError(401, "")), { verdict: "BLOCKED", reason: "operator-auth-unavailable" });
  assert.deepEqual(classifyHttpError(new HttpError(403, "")), { verdict: "BLOCKED", reason: "operator-auth-unavailable" });
  assert.deepEqual(classifyHttpError(new HttpError(500, "")), { verdict: "FAIL", reason: "platform-request-failed" });
  assert.deepEqual(classifyHttpError(new SyntaxError("bad json")), { verdict: "FAIL", reason: "platform-request-failed" });
});

test("Harness auth classification is a narrow allowlist", () => {
  assert.equal(isExplicitHarnessAuthError("Not logged in. Run /login"), true);
  assert.equal(isExplicitHarnessAuthError("process crashed while parsing flags"), false);
  assert.equal(isExplicitHarnessAuthError("request timed out"), false);
});

test("all adapters extract exact native ids only from complete event streams", () => {
  assert.equal(parseClaudeSeed('{"is_error":false,"session_id":"claude-session"}'), "claude-session");
  assert.equal(parseCodexSeed('{"type":"thread.started","thread_id":"codex-thread"}\n{"type":"item.completed","item":{"type":"agent_message"}}\n'), "codex-thread");
  assert.equal(parsePiSeed('{"type":"session","id":"pi-session"}\n{"type":"message_end"}\n'), "pi-session");
  assert.equal(parseGrokSeed('{"type":"text","data":"ok"}\n{"type":"end","sessionId":"grok-session"}\n'), "grok-session");
  assert.equal(parseOpenCodeSeed('{"type":"text","sessionID":"oc-session","part":{"text":"ok"}}\n'), "oc-session");
  assert.throws(() => parseCodexSeed('{"type":"thread.started","thread_id":"incomplete"}\n'));
  assert.throws(() => parsePiSeed('{"type":"session","id":"incomplete"}\n'));
});

test("cleanup verification detects ledger ids, company workspaces, and unique prefixes", () => {
  const ids = new Set(["company-1", "agent-1", "group-1"]);
  assert.equal(hasTrackedResource([{ id: "agent-1" }], ids, "company-1", ["smoke-run-"]), true);
  assert.equal(hasTrackedResource([{ workspace_id: "company-1" }], ids, "company-1", ["smoke-run-"]), true);
  assert.equal(hasTrackedResource([{ agent_principal_id: "agent-1" }], ids, "company-1", ["smoke-run-"]), true);
  assert.equal(hasTrackedResource([{ name: "smoke-run-abc" }], ids, "company-1", ["smoke-run-"]), true);
  assert.equal(hasTrackedResource([{ id: "unrelated", workspace_id: "other", name: "normal" }], ids, "company-1", ["smoke-run-"]), false);
});

test("OpenCode native seeds pin both logical and physical workspace", () => {
  const invocation = openCodeSeedInvocation("/safe/workspace", "opencode/model", "remember marker");
  assert.deepEqual(invocation.args, [
    "run", "--format", "json", "--pure", "--dir", ".", "--model", "opencode/model", "remember marker",
  ]);
  assert.equal(invocation.env.PWD, "/safe/workspace");
});

test("cleanup accepts disabled binding tombstones but rejects executable bindings", () => {
  const tracked = new Set(["agent-1", "binding-1"]);
  assert.equal(hasTrackedActiveBinding([
    { id: "binding-1", agent_principal_id: "agent-1", state: "disabled" },
  ], tracked), false);
  assert.equal(hasTrackedActiveBinding([
    { id: "binding-1", agent_principal_id: "agent-1", state: "idle" },
  ], tracked), true);
});

test("live collaboration prompts describe natural work while preserving strict evidence", () => {
  const group = groupScenario("abc123", "claude");
  assert.match(group.prompt, /group-chat protocol in AGENTS\.md/);
  assert.match(group.prompt, /two separate project updates/);
  assert.ok(group.prompt.includes(group.one) && group.prompt.includes(group.two));
  assert.doesNotMatch(group.prompt, /CHORUZ_SEND|helper|continuity test/i);

  const continuity = importContinuityScenario("abc123def456", "opencode");
  assert.ok(continuity.seedPrompt.includes(continuity.fact));
  assert.ok(continuity.followupPrompt.includes(continuity.requestId));
  assert.ok(!continuity.followupPrompt.includes(continuity.fact));
  assert.doesNotMatch(`${continuity.seedPrompt} ${continuity.followupPrompt}`, /harmless continuity|reply exactly|marker/i);
});
