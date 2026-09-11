# Agent Note: Independent task quality review

Status: implemented

## Problem

A single batch verdict withdraws good tasks alongside a bad one. Source-only review also lacks evidence about whether the extracted input actually conveys the intended question. Blind trial failure cannot itself distinguish an invalid question from a difficult valid task.

## Decision

The existing background worker asks its selected analyst account to attempt bounded task inputs without references, then starts a separate fixed quality review with original evidence. Each task receives its own grounded decision. Repairs can change only input, check and rationale, preserving source identity and equivalence links. A repair must receive another blind trial and independent review; unresolved tasks are withdrawn from scoring without deleting their source evidence. Only one repair round is attempted per sweep.

The revision owns bounded trial outputs, inputs, decisions and procedure digest. A sensitive-data decision is terminal: no repair or retry, and the task-quality history retains only its reference, round and exclusion flag. Earlier attempts for that task are removed from this history too. The extracted task input and check are redacted; original trace retention is unchanged. Repair payloads are validated before reaching history. Existing workspace scoping, leases, source checkpoints and daily curation remain authoritative. There is no second dataset service or scheduler.

## Alternatives considered

**Admit the reviewer's repair immediately.** This evaluates the original task but publishes a different, untested task.

**Drop every failed trial.** This systematically removes difficult work and changes the benchmark instead of improving the Agent.

**Reject a whole batch after one unsupported task.** This loses independent valid objectives and can prevent all three evaluation splits from filling.

## Consequences

Trial and review consume up to four additional model calls per source batch, independently of optimizer budgets. Verdicts remain fallible; explicit source evidence and fixed rules constrain them but are not formal proofs. Bad task data is quarantined by a null check and can be reconsidered on a later sweep. The [trace admission](2026-09-10-trace-evaluation-cases.md) and [curation](2026-09-10-trace-dataset-curation.md) decisions retain provenance, grouping and data-version ownership.
