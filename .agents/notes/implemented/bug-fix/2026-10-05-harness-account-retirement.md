# Agent Note: Complete account removal after owned processes stop

Status: implemented

## Problem

Disabling account metadata leaves native processes executing while disabled bindings deny their controls. An offline device cannot acknowledge cleanup, and process admission can race a removal request.

## Decision

The authenticated gateway owns account retirement. `DbService` records a company-scoped durable request, disables new account work and cancels queued logins and commands atomically. Device cleanup uses `RuntimeHost` and the shared host link. Completion requires device acknowledgement and no remaining leased, started or heartbeating commands. Pending requests survive restart and retry when the device connects.

`choruz-agent-runtime::process_scope` fences admissions and signals existing jobs without owning credentials or a database. Structured sessions, PTYs, background learning, Codex sign-in and headless execution participate. A job is released after its process stops. The existing process container terminates the process tree; cancellation paths reap the child before acknowledging quiescence. The pipeline observes the durable retirement queue, while a connected device receives the account-close operation directly.

The account manager displays pending removal and refreshes its state. Raw terminal input rechecks binding access. Account metadata is hidden after cleanup; profile directories and credentials remain on the device, retaining [single-login ownership](../feature/2026-09-03-single-login-by-default.md).

## Alternatives considered

**Disable metadata and forget the process.** This removes the control surface without stopping charged execution or filesystem changes.

**Kill by executable name or profile directory.** These identities are shared with other accounts and user-owned applications. Only processes admitted under the owned account or binding scope can be stopped.

**Declare offline cleanup successful.** The controller cannot establish the device's process state without acknowledgement. A durable pending request preserves the operation instead.

## Consequences

Stopping cannot undo completed tool effects or recall a request already sent to a provider. Remote cleanup requires an execution-device version that implements account close; an unavailable or unsupported device leaves removal pending. Native credentials are not deleted or logged out.

The browser acceptance traverses the real account route and host link, verifies process and descendant exit on device B, preserves another account's process and checks retained profile bytes. Only model generation is replaced by a deterministic CLI fixture. An offline-device scenario verifies durable pending state without claiming a device effect.
