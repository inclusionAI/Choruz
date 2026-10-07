# Agent Note: Native turn preparation shares learning ownership

Status: implemented

## Problem

Users of native Claude Code and Codex need to consume reviewed learning without replacing their foreground workflow. Reading revision history or saving a permanent prompt file can retain disabled guidance and confuse preparation with observed execution.

## Decision

The authenticated preparation route reads the existing owned learning policy and active revision. It delegates reviewed collaborators to the binding's existing host operation and formats guidance through the shared native marker owner. The CLI is a JSON adapter to that route. The consumer submits the returned prompt through its chosen native CLI; no project file is rewritten and no foreground task is submitted by preparation.

The route rechecks authorization, binding version, policy generation and active selection after collaborators complete. A changed selection rejects the response. The audit event records preparation, not execution or success. Native trace evidence remains necessary for recurrence or effectiveness.

The [background learning decision](../feature/2026-09-08-background-experience-learning.md) retains analysis, consent and evidence ownership. The [composable CLI decision](2026-09-28-composable-cli.md) retains one authenticated control plane.

## Alternatives considered

**Write persistent instructions into the user's project.** This affects other agents and survives revocation unless another file writer is maintained.

**Select from exported history in the CLI.** That creates a second activation policy and misses concurrent rollback.

**Treat prepared input as an applied intervention.** The external consumer might never run it; only later scoped native evidence can establish use.

## Consequences

Preparation supports explicit consumers rather than transparent interception. They own foreground account, workspace and session selection. Collaborators can consume quota, and an interrupted response is not automatically retried. A response cannot prevent a later consumer from retaining an old prompt. It is a per-turn snapshot, not a revocable credential or proof of improvement.
