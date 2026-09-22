# Agent Note: Evaluate bounded decision programs before selection

Status: implemented

## Problem

Recurring finite-choice tasks can consume a full coding-agent turn, but converting a successful trace into automation does not establish that the automation generalizes. Private learning consent also does not authorize sending those traces to another inference provider.

## Decision

Keep structured inference in a standalone library and require separate transmission consent. The target device owns the provider credential. An isolated Codex conversation builds a declarative program from training examples; held-out objectives use the existing task partitioner, output checks and independent judge. Programs cannot contain shell commands or JavaScript. Selection is explicit and requires compatible evaluated model identity and confidence settings.

The Rust question contract preserves structured instructions and rubrics from the official TypeSafe API, including optional Noul outcome descriptions. The builder embeds scoped guidance adapted from the official skill: question IDs carry no inference meaning, parallel heads cannot consume each other's answers, and only the designated result Choice controls finite output. Dependent retrieval is outside this one-round format. Source revisions and attribution live in the decision package README and repository NOTICE.

An explicitly authorized browser run can ask its binding's native reasoning Harness to resolve two decision abstentions within the current approved target set. The gateway supplies the account and model from the authorized binding; browser input cannot supply a different process specification. Device execution uses a fresh, tool-disabled CLI conversation under the same run deadline. The assistant cannot change steps or retry an uncertain mutation. Action receipts distinguish proposed decisions, actual attempts and acknowledgements; final visible-text checks remain independent.

## Alternatives considered

**Execute arbitrary generated code with the provider key.** This adds credentials and general tool authority to an untrusted artifact. Finite-output programs need neither.

**Treat confidence as acceptance.** A coherent answer distribution can still be wrong. Confidence controls abstention; external evaluation controls selection.

**Let the reasoning assistant execute its own browser tools.** That would bypass current-observation checks, cancellation ownership and the reviewed manifest. The assistant returns only a target proposal to the existing executor.

**Add a Python SDK service or a second comparison evaluator.** The Rust HTTP owner and existing provider-injected evaluation already own transport and acceptance. Porting needed question semantics avoids another runtime; an upstream comparison adapter is not a reason to duplicate the platform's fixed judge or introduce silent provider fallback.

**Replay recorded browser targets.** Element identities belong to one observation. The decision library selects only from supplied targets; the [device computer-use decision](2026-09-07-device-computer-use-skills.md) still owns tool setup, not permission to execute generated workflows.

## Consequences

Program construction shares the existing learning claim and revision transaction. Corpus reservations prevent duplicate construction after ambiguous failure. Corrections invalidate dependent evaluation evidence, not just the selection pointer. Finite-output execution is separate from browser action execution and from automatic replacement of live Agent turns; a validated trial is not proof of broad production improvement.
