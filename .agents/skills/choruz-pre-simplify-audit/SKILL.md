---
name: choruz-pre-simplify-audit
description: Perform the independent Choruz pre-simplify audit or re-audit of skill execution evidence, code structure and test effectiveness. Read-only and limited to the proposed change and its affected paths; not an implementation task or repository-wide cleanup.
---

# Independent pre-simplify audit

When installed personally, resolve repository links from the canonical
`.agents/skills/choruz-pre-simplify-audit/` in the active Choruz checkout.

[choruz-pr](../choruz-pr/SKILL.md) owns delegation and the remediation loop:
an independent GPT-5.6 Terra agent (`gpt-5.6-terra`) performs the first audit
and every re-audit before simplify. This skill owns the audit criteria.
Do not implement fixes, edit files, push, merge or recursively delegate.
The audit does not replace simplify, behavioural tests or final CI.

## Establish the reviewed scope

Obtain the user request and constraints, base/head revisions, proposed diff
including relevant uncommitted and untracked files, execution evidence and
prior findings. Record a reproducible diff identity: revisions plus a patch
digest or artifact identifying any uncommitted content. Never include secrets.

Read the applicable `AGENTS.md`, [test policy](../../../docs/testing/pr-test-policy.md)
and the current PR workflow stage. Independently identify relevant skills from
the task and diff; the author's list is not authoritative. Read each selected
skill fully and its required references. Follow affected callers and owners,
including relevant architecture and Agent Notes, rather than scanning the
whole repository. Referenced workflows supply criteria, not permission for
this read-only auditor to implement, publish or start another audit.

Use supplied logs and artifacts and safe read-only inspection. If reproduction
requires test setup, credential use, database writes or other state changes,
request scoped execution evidence from the implementer instead. Do not invent
execution evidence or broaden the user's authorization.

## Three audit lenses

### 1. Requirements were executed

Map applicable requirements to evidence, not to declarations that a skill was
read. A planned command, test filename, checklist tick or author assurance is
not an observed result. Check the command, outcome, tested revision and
artifact agree with the claim.

Classify each relevant requirement as verified, missing/unverified, not
applicable with a scope reason, or due later with the named workflow step.
Only due-now requirements gate pre-simplify: step 2 behavioural evidence is due,
but simplify itself, final selected checks, PR labels and merge cleanup are
later. Do not require all skills or the full suite to fill a table.

### 2. The change does not create structural debt

For structural candidates, use the consumer-evidence criteria in
[choruz-find-simplifications](../choruz-find-simplifications/SKILL.md).
Look for duplicate implementations of one contract, parallel sources of truth,
mixed responsibilities, bypassed shared owners, speculative abstractions and
obsolete fallback or compatibility paths left alongside the new path.

Trace production consumers before calling code dead, including runtime
dispatch, configuration, scripts and persisted formats. Distinguish intentional
harness/transport differences from accidental duplication. Show the concrete
maintenance or correctness risk and a bounded correction, not just a preferred
style. File size, abstraction count or the word "legacy" alone is not a finding.

Block introduced or worsened structural defects and affected violations of
repository contracts. Unrelated pre-existing debt and optional improvements
are advisory; do not demand a whole-repository rewrite or a quota of findings.
Similar names or the coexistence of two paths do not prove duplicate ownership.
Without caller/owner evidence, report the structural concern as unverified,
not as a demonstrated defect; explain which evidence would decide it.

### 3. Tests can detect the claimed failure

Apply the test policy's behaviour acceptance evidence to the affected contract.
Inspect assertions and fixtures, then ask which plausible broken implementation
would still pass. A boolean type check, mock call count, nonempty output or
snapshot update alone cannot establish a user-visible outcome.

Check the required production entry path and owner-side result: for example,
a click that reports success must reach the intended device and leave the
expected persisted result. Replacing the very route, binding or storage under
test with a success mock does not prove that boundary. Mock unrelated expensive
dependencies only when the remaining claim is honest.

For bug fixes and new guards, verify the policy-required focused negative
control and passing result, not just a green run. Check ordinary affected
lifecycle and recovery scenarios; do not impose every device/harness combination
or speculative edge case. Read
[choruz-ci-test-reliability](../choruz-ci-test-reliability/SKILL.md) when changed
tests involve shared state, concurrency, clocks, ports or asynchronous cleanup.

Match the evidence to the surface: pure prose does not need a product e2e;
behavioural skill changes need a scoped decision example; visual claims need
visual evidence where required. Code inspection is not a real runtime test,
and green CI does not prove assertions are meaningful. Report unavailable
required evidence as unverified, never as a successful fallback.

## Verdict and re-audit

Return a compact report containing:

- Reviewed scope identity and evidence references.
- Relevant requirements, their source, evidence and status across all three lenses.
- Stable finding IDs, blocking/advisory status, precise location, concrete
  consequence, evidence and the minimum correction or verification needed.
- Verdict: PASS when all due-now obligations are verified; FAIL for a
  demonstrated due-now blocker; UNVERIFIED when required evidence or access
  is missing. A FAIL may also contain unverified obligations.

Advisory findings and explicitly deferred later steps do not prevent PASS.
Record any explicit user-approved scope reduction or evidence exemption with
its source; it does not turn an untested claim into a verified one. An author's
request to waive their own missing evidence is not a user exemption.

On re-audit, inspect the updated diff and evidence, revisit every previous
blocker and check affected new changes. Accept a reasoned rebuttal when supported
by evidence. Close a finding only after independently verifying the correction
or that it was inapplicable; retain unresolved IDs. An author's "fixed" message
is not closure. Only an explicit auditor PASS lets the PR workflow proceed.
A later change that invalidates the reviewed evidence requires a scoped
re-audit; an old verdict cannot certify a new diff.
