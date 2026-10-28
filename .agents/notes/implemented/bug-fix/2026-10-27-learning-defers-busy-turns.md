# Agent Note: Defer learning while the source turn is still running

Status: implemented

## Problem

A learning check that reached the source Agent while its turn was still running returned `Conflict("Learning waits for the current turn to finish")`. The worker treated that as a failed check: it showed the message as the policy's `last_error` and scheduled the next check two minutes later. Enabling learning or sending feedback right after a turn therefore delayed analysis by two minutes for an ordinary, short-lived condition. The browser acceptance test for remote learning waits 45 seconds for activation and failed intermittently under parallel load for this reason.

The device link also lost the retry signal: `LinkError` encoded `RateLimited` as `rate_limited` but decoded every unknown kind, including that one, as `Internal`.

## Decision

`experience_source::for_read` returns `None` while the source session is running, connecting or waiting, and the `ExperienceTrace` host request answers with a normal `{"busy":true,"retry_after_ms":BUSY_TURN_RETRY_MS}` window (five seconds) instead of an error. The experience worker's review returns `Reviewed::Deferred(delay)` for that window, before any learning state is written. `release_experience` takes a `CheckOutcome`: `Completed`, `Failed(message)` with the existing two-minute retry and visible error, or `Deferred(delay)`, which records no error and schedules the next check after `delay`. Diagnostics record the check as `deferred`.

Only that busy window defers. Every error, including a host or provider `RateLimited`, remains a failed check with its owner-visible message, so a real limit is never mistaken for a turn in progress.

`LinkError` carries `retry_after_ms`, so `RateLimited` round-trips between a remote device and the gateway instead of degrading to `Internal`.

## Alternatives considered

**Shorten the retry for every error.** Real failures, such as an unavailable account or device, would be retried every few seconds and still be shown as errors; only the busy turn is transient.

**Wait inside the check until the turn finishes.** The check holds a lease and an admission slot; a long turn would block other profiles for its whole duration.

**Report the busy turn as `RateLimited`.** The worker could not tell a busy turn from a real host or provider limit, and would have hidden the latter as a routine deferral.

**Match the conflict message in the worker.** Text matching couples the worker to wording in another crate and does not survive translation into a typed wire error.

## Consequences

A busy source turn no longer appears as a learning error, and the check repeats every five seconds until the turn ends. Tests cover the source reader returning the busy window for running, connecting and waiting sessions, the worker mapping a busy window to a deferral while a `RateLimited` error stays a visible failure, `RateLimited` surviving the device link, and the policy scheduling: a failure keeps a two-minute retry with its message, while a deferral retries within seconds without one.
