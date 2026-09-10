# Agent Note: Add execution roles only after observed prompt recurrence

Status: implemented

## Problem

A stored prompt revision is not proof that an Agent used it. Repeated windows or feedback about one objective are not independent failures. Escalating on either signal grows the execution machinery without evidence that a prompt intervention failed.

## Decision

The fixed analysis process records distinct objective references and failure evidence. The first intervention comes from category-only web research followed by source-grounded adaptation; failed search is not an empty result. Pending problems survive window boundaries until later feedback has been read. An episode qualifies for escalation only with a native user-input revision marker before its failure and a different objective from the recorded first occurrence.

Content review records eligibility for structural search without adding collaborators. The [team search](2026-09-10-evolving-execution-teams.md) supersedes the direct single-reviewer proposal path and owns measured membership and ordering changes. Prompt and team remain owned by one revision, captured together for each future turn and restored together. This is a bounded execution policy, not permission for generated code to rewrite the runtime or for the analyzer to evolve its own procedure.

Structured sessions reserve the turn while the collaborator runs. Stop invalidates that reservation and cancels the collaborator, so late results cannot deliver a cancelled task. Headless execution uses the same device-local role runner; connector heartbeats cover the additional call. The collaborator uses the target's account in a fresh scratch conversation, not the analyst's ongoing session. Its input labels checks as proposals, not completed verification. The conversation preview preserves that native text verbatim: text delimiters alone cannot establish trusted platform authorship.

## Alternatives considered

**Add another instruction after every repeated report.** This cannot distinguish retrying the same task from a new failure after intervention. The ledger deduplicates objectives and retains the applied revision evidence.

**Let the analyzer rewrite runtime code or create arbitrary teams.** Unbounded generated control flow cannot share the runtime's cancellation, authorization and rollback guarantees. Typed execution configuration keeps those guarantees in the existing execution owners.

**Automatically repair the historical task.** Later cancellation, changed intent or unrelated workspace changes can make this unsafe. Only future submissions use the new role; history is evidence, not a new execution request.

## Consequences

Preflight adds a model call and waiting time to eligible future turns. Planning is not verification, and preview/content review is not evidence of improved future outcomes. Native PTY input remains outside this mechanism. A provider may refuse analysis; diagnostics must distinguish that from an invalid account and must not claim an intervention was produced.

The assembled gateway test covers a failure before EOF, later activation, a distinct failure after a usage marker and role rollback. The structured-process test covers role delivery, duplicate submission and cancellation; remote E2E covers activation through future device-owned DM input. These do not establish long-term behavioral improvement.
