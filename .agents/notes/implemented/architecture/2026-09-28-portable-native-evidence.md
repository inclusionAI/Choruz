# Agent Note: Share native evidence reading without a host dispatcher

Status: implemented

## Problem

Independent learning consumers need bounded native records and verified source
references, but coupling the reader to a terminal specification brings unrelated
PTY and device-dispatch dependencies or encourages a second parser.

## Decision

The optional `choruz-learning::native_source` module owns windows, projections,
reference recovery and observed model attribution. Its input selects one native
account directory, workspace, session and supported harness. It does not select
ambient accounts, start a process or persist cursors. Shared Codex session-file
discovery lives in `choruz-agent-runtime::session_files` and also serves managed
terminal attribution; neither caller keeps a copied parser.

Host runtime resolves the binding's account and recorded session. It retains the
unfinished-turn gate before requesting an analysis window. Historical reference
requests retain explicit selected-session validation. The
[background learning decision](../feature/2026-09-08-background-experience-learning.md)
continues to own consent, durable cursor commits, recurrence and activation.

## Alternatives considered

**Keep the reader behind HostRequest.** A filesystem-only consumer would inherit
the terminal and device dispatcher solely to read evidence.

**Create a standalone collector.** This would duplicate scheduling and checkpoint
ownership. The reader is a synchronous operation composed by existing workers.

## Consequences

Both harnesses use the same reader in standalone and platform workflows.
Callers must authorize their source and ensure turn completion. Returned private
evidence is not a publication projection; reasoning filtering is not general
PII redaction. The append-only source assumption and existing response bounds
remain unchanged.

Moved reader tests retain cursor, record-boundary and attribution coverage.
Account-isolated fixtures exercise both public harness paths; a host request
test verifies that persisted running turns remain blocked and an idle turn can
use its recorded native identity without starting a CLI.
