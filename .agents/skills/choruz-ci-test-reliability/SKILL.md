---
name: choruz-ci-test-reliability
description: Design, review, and diagnose Choruz tests and fixtures that can fail nondeterministically under CI concurrency: parallel Playwright workers sharing one PostgreSQL and one API, vitest workers, cargo tests on a shared test database, clocks, ports, and asynchronous teardown. Use when adding or changing tests with those risks, investigating a flaky CI run, or reviewing test isolation.
---

# Reliable Choruz CI tests

Build tests that remain correct under the repository's real CI topology, not only when run alone on a quiet workstation. This skill owns isolation and reliability decisions; [docs/testing/pr-test-policy.md](../../../docs/testing/pr-test-policy.md) owns which tests a change must add, and [choruz-pre-push-checks](../choruz-pre-push-checks/SKILL.md) owns which commands to run before a push.

## Model the execution topology

Assume these layers overlap unless the active configuration proves otherwise:

1. **Playwright:** `apps/web/playwright.config.ts` runs with `fullyParallel` and two workers on CI (`CHORUZ_E2E_WORKERS`), sharded across jobs (`--shard=n/m`). Every worker talks to the same API, the same PostgreSQL and the same pipeline started by `infra/host/web_e2e.sh`; only the browser context is private. `retries: 1` on CI means a test that passes on retry is reported as flaky, not as passing.
2. **vitest:** `apps/web` unit tests run in worker processes; module state, `process.env`, timers and `localStorage` mocks are per file, not per test.
3. **cargo:** integration tests under `crates/` and `services/` share the database created by `infra/host/setup_test_database.sh`; tests in one binary run on parallel threads.
4. **Jobs:** the e2e shards, the Rust jobs and the smoke jobs run at the same time on separate runners; they do not share a host, but they do share GitHub's artifact and cache services.

Process isolation does not isolate database rows, ports, predictable filesystem paths or inherited child processes. For every acquired resource, identify its owner, its allocation mechanism, its readiness signal, its cleanup, and its quiescent completion signal.

## Own your data

The single most common Choruz flake is a test that reads global state another worker is writing.

- Create what you assert on. Seed a group with `createGroup(page, token, principal.id, uniqueName("…"))`, an agent with `provisionAgent`, a message with `sendMessage`; then assert on those ids and names, never on "at least one exists" or "the first item".
- Never compare absolute counts taken at two moments (`countAfter === countBefore`): another worker can add a row in between. Assert the filtered result is empty and the restored list contains the seeded item, or use `toBeGreaterThanOrEqual` with the seeded item present.
- Never assume an empty workspace, an empty conversation list, or that a default agent exists.
- Use `data-conversation-id` and other stable attributes instead of visible text that other workers can also produce.
- In cargo tests, give rows unique names or ids per test and scope queries to them; never truncate a shared table.

## Allocate resources atomically

- Ports come from the environment (`CHORUZ_API_PORT`, `CHORUZ_WEB_PORT`, `CHORUZ_PIPELINE_METRICS_PORT`, `CHORUZ_PG_PORT`) that `web_e2e.sh` already isolates per run; a test never binds a fixed port of its own. A Rust test that needs a listener binds port 0 and reads the assigned address.
- Temporary files use a private per-test directory (`tempfile::tempdir()`, `mkdtemp`), never a predictable path.
- Route mocks (`page.route`) match the exact request the test owns, not a prefix another flow also hits.

## Synchronize on state

A fixed `waitForTimeout` is not evidence that setup completed or that cleanup settled.

- Wait for the observable condition: `expect(locator).toHaveCount(n)`, `expect.poll(...)`, a readiness endpoint, an owned promise.
- Message delivery in the web app arrives through the sync feed; wait for the bubble with the seeded content, not for a duration.
- Use a timeout only to bound a wait, never as the condition that makes the assertion correct.
- When time itself is the subject, inject or fake the clock and always restore real timers.

## Dispose to quiescence

Register cleanup right after acquisition so an assertion failure also releases the resource. Cleanup stops new requests, detaches listeners, restores globals, terminates owned work, and awaits child exit or server close. Calling `abort()` or `kill()` without awaiting completion is incomplete teardown.

## Prove the intended regression

- Observe the regression fail before the fix when practical: introduce the bug, watch red, revert.
- For a fixed flake, run the spec repeatedly with the CI settings: `bash infra/host/web_e2e.sh tests/e2e/<spec> --repeat-each=3`.
- For a race, use barriers to prove overlap; repeated execution alone is not a race test.
- Verify external state (a row, an event, a file, a process exit) instead of trusting the component's self-report.

## Reject flake-masking fixes

Do not present these as root-cause fixes:

- raising a timeout without naming the awaited state;
- adding retries or `test.fixme`/`test.skip`;
- making a whole file serial to hide one shared resource;
- swallowing an error or unhandled rejection;
- weakening an assertion or normalizing away unstable behaviour;
- adding a sleep before cleanup or an assertion.

Never skip, disable or quarantine a test to get CI green; fix the isolation.

## Diagnose an existing flake

1. Download the shard's Playwright artifact (`playwright-<shard>.zip`, or `playwright-full-<shard>.zip`) and read the trace and screenshot of the failed attempt; the retry's pass is not evidence.
2. Read the test for shared state: unseeded data, absolute counts, first-item locators, fixed waits, mocks broader than the test.
3. Reproduce locally with the CI settings (`CI=1`, two workers, `--repeat-each=3`) and the neighbouring specs of the same shard, since the interfering worker is usually one of them.
4. Fix the isolation, then run the repeat locally before pushing. A diagnosis-only request is read-only: report the cause and evidence unless the user also asks for a fix.

## Validate and report

Run the smallest focused regression for the affected behaviour, plus topology-specific evidence only when the change owns that risk: restoration evidence for global mutation, quiescent-teardown evidence for lifecycle work, a negative control for a new guard. Report exact commands and observed results; do not describe retries, skipped tests, or pending CI as passing.
