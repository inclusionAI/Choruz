# Agent Note: Preserve provisioning gateway failures

Status: implemented

## Problem

Wrapping a gateway failure as a provisioning step error loses the HTTP status.
The route can misreport a rate limit as a server fault and omit retry timing.
Message-heavy browser tests sharing one operator also consume one rate bucket.

## Decision

The step wrapper retains the original cause. The route forwards only the typed
gateway status, message and Retry-After header; internal causes and completed
step data are not serialized. Untyped local failures remain HTTP 500.
Messaging tests register their own users, and upload tests own their company.

## Alternatives considered

**Increase the rate limit or serialize tests.** Neither removes shared ownership
or corrects the application's misleading HTTP response.

**Parse the error message for a status.** Human-readable text is not a protocol;
the existing typed gateway error owns the status.

## Consequences

Callers can distinguish rate limits from local faults. This forwards the
gateway's retry timing; it does not change its calculation or automatically
retry partially completed provisioning. The assembled route test replaces only
upstream HTTP and unrelated auth/idempotency storage, not the step or decoder.
