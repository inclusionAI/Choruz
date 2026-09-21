# Agent Note: Share community evidence independently of the platform

Status: implemented

## Problem

The behavior schema and evidence counts live in domain while the revision-pinned dataset client lives in the API gateway. A standalone consumer would need platform packages or copy those contracts, risking different identity, privacy and counting rules.

## Decision

`choruz-community` owns the existing behavior contract and dataset exchange client. Application, host runtime and gateway import the library directly. Database records, owner consent, privacy review, publication leases and activation remain with their existing platform owners. There is no domain re-export or alternate client.

This preserves the [behavior community decision](../feature/2026-09-12-behavior-community.md) and the [modular monolith](2026-08-18-modular-monolith.md). A client reports a contribution PR, not acceptance, and cannot establish that prose is safe to publish.

## Alternatives considered

**Expose gateway internals as the library API.** It would couple a dataset consumer to the platform's HTTP server and persistence.

**Copy the schema into a separate SDK.** Two owners could disagree on evidence identity or count retries as independent occurrences.

**Move the complete worker into the library.** Its leases, consent and activation are platform policy, not a dataset transport responsibility.

## Consequences

The same record, deduplication and HTTP implementations serve standalone callers and the product. Existing schema and HTTP fixture tests move with their owners. Packaging and a deterministic external example verify use without platform storage; they do not certify public contributions or model quality.

Background acceptance tests advance only future idle schedules. Updating an already-due row on every poll can lock it away from the worker's `SKIP LOCKED` claim; the policy test holds the polling transaction open to verify that overlap without timing assumptions. Production scheduling, lease fencing and acceptance timeouts remain unchanged.
