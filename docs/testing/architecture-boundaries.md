# Architecture boundaries

Choruz extends one business owner per contract. Device, transport and provider adapters translate requests; they do not copy account selection, execution policy, persistence or evidence interpretation. Different contracts may have different owners: a database-backed service and a standalone in-memory library are intentional alternatives, not competing persistence authorities inside one request.

## Before changing a capability

Trace the affected production entry to its result and record the following in the PR's Ownership section:

- The existing business owner and authoritative state, with concrete callers.
- Where local and remote adapters join that owner, or why the contract is device-independent.
- Which replaced implementations, exports, configuration and tests are removed. An intentional difference names its distinct contract and consumers, not just a different filename.
- The failure assertion that protects the contract. Durable mutations must fail when storage fails; projections update after commit. Sharing a helper name alone does not prove either property.

If the capability has no owner, put its business policy in one reusable module and have adapters call it. Do not invent a generic framework for unrelated operations. Existing lease, cancellation, authorization and privacy fences remain at their owning boundaries.

## Executable checks

Run from the repository root after staging new source files:

```bash
python3 scripts/check_architecture.py
python3 -m unittest discover scripts/tests -p test_check_architecture.py
```

The checker reads tracked files, including staged additions. CI's Static checks job runs both commands on every non-documentation change; `CI (linux) required` requires that job. The rule definitions in [`scripts/check_architecture.py`](../../scripts/check_architecture.py) own exact paths and patterns. They protect browser API transport, headless turns, native learning evidence, feedback recovery, execution identity, database webhook delivery and driver metadata. They also reject reintroduced retired projection, adapter and sandbox modules. Each rule checks its owner exists, known consumer wiring remains, and specified bypasses are absent. New API client files in the checked directory and its descendants are scanned too. Server-only authentication verification is an explicit exception to browser transport, not an allowed browser bypass.

These are lexical boundary checks, not a proof of semantic uniqueness. Aliases, macros, equivalent code under a new name, new capabilities outside the declared scopes and database transaction ordering require the independent audit and behavioral tests. Comments, literal text, separate web test files and Rust test modules are excluded; executable template interpolation is checked. The checker is not a language parser; changes using new syntax need a fixture if it affects recognition. Do not interpret a green scan as a whole-repository architecture certificate.

## Changing a boundary

A moved owner updates its rule and every caller in the same change. A new mechanically recognizable bypass adds a focused rule and a disposable negative-control fixture that demonstrates rejection and a legitimate-adapter fixture that passes. Keep the scopes narrow enough to avoid banning intentional provider or transport implementations. Do not add a violation baseline, inline suppressions or broad exceptions to get green. A genuine exception requires consumer evidence, rationale and an independently reviewed rule/test change.

The independent [pre-simplify audit](../../.agents/skills/choruz-pre-simplify-audit/SKILL.md) checks semantic ownership as well as the command result. It traces production consumers, checks removed paths and evaluates whether the tests would reject a copied or bypassed implementation. A repeated defect belongs in a shared contract test or a precise boundary rule, not another duplicate test suite for each adapter.
