# Agent Note: CLI capabilities retain their existing owners

Status: implemented

## Problem

Independent libraries need usable entry points for users who keep their native Harness workflow. Reimplementing platform learning or writing its tables from the CLI would create different policy, authorization and lifecycle rules.

## Decision

The existing `choruz` binary calls local libraries for explicit native trace sources, output checks, community validation and device tools. It calls authenticated host routes for durable learning configuration and other platform operations. It does not create a second worker or data store. Native trace sources require an explicit account home and workspace; judge checks remain unscored until a judge executes them.

Device setup retains the ownership in [portable device tools](2026-09-28-portable-device-tools.md); the command waits for its asynchronous installer before exiting. Host operations retain the authorization boundary in [activity data tools](../feature/2026-09-05-activity-data-tools.md). Both notes remain active for their independent security and lifecycle guarantees.

## Alternatives considered

**A separate standalone learning daemon.** Rejected because it would duplicate scheduling, consent and persistence already owned by the host.

**Direct database commands.** Rejected because they bypass the shared API permissions and audit trail.

## Consequences

Local library commands need no Company or running server. Durable learning still requires an existing host and binding. The [native preparation adapter](2026-09-28-native-learning-preparation.md) returns per-turn guidance for explicit native consumers, not an automatic hook into every CLI session. Tool installation can succeed while external permissions still need attention. JSON inputs are bounded, and API errors are surfaced without automatic mutation retries.
