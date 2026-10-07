# Agent Note: Persistent task learning

Status: implemented

## Problem

New tasks created fresh bindings, losing prior learning. Curation only saw the current native session, and a default-model task could not configure measured learning without replacement.

## Decision

Reference the existing owned policy from matching tasks; do not copy mutable learning state. Keep captured sources in a separate learning-only ledger instead of using historical anchors to resume a CLI. Select explicit models on existing idle Agents without changing their session identity.

The [runtime contract](../../../../docs/subsystems/agent-runtime.md#behavior-community) owns scope, consent and lifecycle behavior. The [data model](../../../../docs/data-model.md#experience_policy-and-experience_revision) owns persistence.

## Alternatives considered

Copying prompts, teams and programs on provisioning creates diverging histories and makes rollback ambiguous. Reusing one conversation for every task loses task separation. Reusing old source anchors for implicit resume weakens session isolation.

## Consequences

Matched tasks share one learning owner while keeping their own runtime and conversation. Source retention is evidence retention, not permission retention. Browser grants remain binding-owned. Public contribution needs renewed consent when the collection scope expands; an existing root-only permission is not inherited silently.
