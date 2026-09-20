# Agent Note: Host failure signals have an operator recovery path

Status: implemented

## Problem

Headless host startup and child-exit logs identify failures without telling an operator which deployment layout, retained data or service boundary to inspect. Retrying from an incomplete bundle cannot fix a missing migration directory.

## Decision

The existing [runbook](../../../../docs/operations/runbook.md#incident-headless-host-startup-or-child-failure) owns the mapping from server and supervisor log patterns to safe recovery steps. It distinguishes the embedded database host from managed services, documents executable-relative migration discovery and identifies child stderr as the root-cause source. Alerts belong to the operator's existing log collector or service manager, including process-down detection when metrics cannot be served.

## Alternatives considered

**Add a second deployment guide.** The existing runbook and [verified delivery decision](2026-09-06-verified-continuous-delivery.md) already own diagnosis and activation respectively. Cross-links preserve that boundary.

**Automatically reset state or kill conflicting listeners.** Startup errors do not establish that retained data is disposable or that another process belongs to Choruz.

## Consequences

This is documentation of existing behavior, not a new alert transport or automatic repair mechanism. It preserves applied migrations, data and unrelated processes. Recovery metric semantics remain with the pipeline's implementation and subsystem reference.
