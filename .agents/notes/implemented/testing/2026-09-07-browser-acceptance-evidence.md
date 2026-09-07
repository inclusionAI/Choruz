# Agent Note: Browser assertions follow the owning boundary

Status: implemented

## Problem

Empty conversation loops can pass without testing presentation. Exact floating
point equality rejects imperceptible layout rounding. Terminal layout checks
can fail while the asynchronous session is still starting. Retry-only traces
lose the original failure when that retry passes.

## Decision

One seeded message-list scenario verifies display cleanup of ANSI and reply
tags while the read API preserves the original message. Unseeded outbox and
empty-group tag checks are removed. The existing detail resize journey checks
20px growth and persisted width to hundredth-pixel precision. Terminal layout
checks wait for the owned binding's ready response before inspecting its DOM.
History reading checks the visible message anchor rather than a virtual list's
estimated distance from the bottom, which changes as rows are measured.

CI records and retains first-attempt failures. Reports upload when a trace
exists even if a retry succeeds; passing tests do not retain their traces.

## Alternatives considered

**Increase assertion timeouts or retries.** This conflates asynchronous session
startup with terminal layout and preserves the missing synchronization point.

**Keep empty loops as cheap smoke tests.** They do not detect a broken formatter
and can assert an obsolete storage contract.

**Record only retry traces.** A successful retry does not explain the initial
failure; first-run recording has a runtime cost but retains the required evidence.

## Consequences

The test count falls without removing a distinct behavior guarantee. Tests
still reject absent formatting, missing resize persistence and failed session
startup. Native CLI output and stored message contents remain unchanged.
