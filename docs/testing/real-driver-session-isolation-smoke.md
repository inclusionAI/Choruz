# Real-Driver Session Isolation Smoke

This manual smoke supports `AGENT-007` / `B-005`. Deterministic regressions
prove platform contracts; this smoke exercises a real local CLI session store
and workspace cwd behavior.

It proves only Codex local session persistence and cwd isolation. It cannot
satisfy the Task 8.8 real-driver platform PASS requirement on its own: separate
disposable Choruz evidence must prove binding identity and provenance, routing,
persistence, PTY behavior, and fanout for the required supported actor, driver,
mode, and feature rows.

Do not run it against a developer's existing long-lived agent workspace. Use
fresh throwaway agents and clean the workspaces after recording the result.

## Opt-In Codex Runner

For repeatable Codex local-session coverage, run:

```sh
CHORUZ_REAL_DRIVER_SMOKE=1 infra/host/smoke/agent-session-isolation-real-driver.sh
```

The script creates disposable workspaces, invokes real `codex exec`, resumes
the two resulting sessions, and asks symmetric prefixed-token leakage
questions without revealing the other agent's sentinel suffix. It retains and
prints only a verdict-only result with opaque aliases and a truncated SHA-256
digest of the sanitized summary. It does not retain raw session IDs, absolute
paths, synthetic sentinel output, raw CLI logs, or the artifact root. Synthetic
sentinel output must never be retained in release evidence.

The smoke artifact directory contains only the sanitized result; raw runner
outputs and sentinel files are deleted. A real opted-in Codex invocation may
still retain native local CLI session records containing the synthetic prompts.
The runner does not modify global Codex configuration or state, and it never
copies or commits those native records.

Run this static, non-model check to inspect the public result shape:

```sh
CHORUZ_REAL_DRIVER_SMOKE_TEMPLATE_CHECK=1 \
  infra/host/smoke/agent-session-isolation-real-driver.sh
```

The runner makes no login, configuration, or global-state changes. It may
create normal native local Codex session records as part of an explicitly
opted-in live run; it does not inspect, copy, modify, or delete existing
session state.

## Preconditions

- A disposable run directory and workspaces are available locally.
- Codex is authenticated and available: `codex --version`.
- The operator has opted into a real local model turn with
  `CHORUZ_REAL_DRIVER_SMOKE=1`.
- Do not inspect or copy existing `~/.codex`, `~/.claude`, or Gemini session
  files into the repo.
- Use unique synthetic sentinel strings that are safe to submit to the
  disposable run. Do not retain those values in result files, logs, screenshots,
  PR descriptions, or bug reports.

## What The Runner Checks

1. It creates two local disposable workspaces and establishes a separate Codex
   session for each.
2. It resumes each session from only its own workspace.
3. It asks each session whether it can see a complete prefixed token from the
   other session or workspace, including a second resume-path check.
4. It checks the raw transient output locally, converts the outcome to
   `not-observed`, `detected`, `confirmed`, `failed`, or `not-run`, writes the
   sanitized verdict, and removes raw transient outputs and sentinel files.

## Pass Criteria For This Narrow Smoke

- Agent B never outputs agent A's direct-chat or workspace sentinel.
- Agent A never outputs agent B's direct-chat or workspace sentinel.
- Each resume-path check returns its expected isolation confirmation.
- The retained result contains only the safe result template fields below.

This is not evidence that Choruz created the correct runtime binding, persisted
the right `external_session_id` provenance, routed a direct or group message,
streamed the correct PTY output, or delivered participant fanout. Task 8.8 must
keep any corresponding matrix row BLOCKED until separate disposable Choruz
evidence proves those claims for the specific supported row.

## Fail Criteria

Record a sanitized failure for `B-005` if an agent can quote the other agent's
direct-chat sentinel, read the other agent's workspace sentinel, resumes
another binding's session, or starts in another binding's workspace. Record
only the driver, safe aliases, matrix row, and high-level failure class. Do not
paste sentinel values, full local CLI session files, raw logs, absolute paths,
or credentials into the bug report.

## Safe Result Template

```text
Run alias: <opaque-run-alias>
Driver: Codex CLI
Scope: local session persistence and cwd isolation
Session aliases: agent-a=<opaque-session-a>; agent-b=<opaque-session-b>
Driver invocation: <completed|failed|session-unavailable|establishment-unconfirmed|transient-output-missing>
Direct-history cross-actor result: <not-observed|detected|not-run>
Workspace cross-actor result: <not-observed|detected|not-run>
Resume-path result: <confirmed|failed|not-run>
Verdict: <PASS|FAIL>
Verdict summary SHA-256 (truncated): <16 lowercase hex characters>
Residual risk: no Choruz binding, provenance, routing, persistence, PTY, or fanout coverage
```

## Attempt Ledger

- 2026-05-17: Integrated Choruz host smoke did not produce release evidence in
  this local Codex environment. Starting an isolated Postgres-backed host stack
  was blocked by sandbox shared-memory restrictions and escalation timeouts.
- 2026-05-17: Claude Code local-driver probe did not produce release evidence.
  The CLI started with a throwaway session id and temp workspace but returned
  `FailedToOpenSocket` before any model turn or workspace/session check ran.
- 2026-05-17: Codex CLI local-driver probe did not run. The sandboxed command
  could not write the real Codex state database and failed before the model turn;
  the required explicit approval to create normal local Codex session records
  timed out before command execution.
- 2026-05-17: Focused local-store regression passed with
  `cargo test -p choruz-agent-runtime session_lookup`. This covers Codex disk
  non-guessing and Claude workspace-scoped `.claude/projects/.../*.jsonl`
  lookup using throwaway session-store files, but it is not a live
  model-invoking driver smoke.
- 2026-05-17: Added opt-in Codex smoke runner:
  `CHORUZ_REAL_DRIVER_SMOKE=1 infra/host/smoke/agent-session-isolation-real-driver.sh`.
  Run it from a normal terminal with a working Codex CLI to reproduce live
  model-invoking evidence.
- 2026-05-17: The historical Codex smoke was live model-invoking evidence only
  for Codex local session persistence and cwd isolation. It did not exercise
  Choruz API provisioning or PTY WebSocket plumbing, and it cannot be used as
  a Task 8.8 platform PASS without the separate evidence specified above.
