# Agent Note: Online history is not an authentication poll

Status: implemented

## Problem

Polling cloud account verification for every browser's group refresh exhausts
the shared IP request budget. A throttled verification then blocks restored
group history, although the local user and their saved draft remain valid.
Mapping the upstream 429 to 500 also hides the service's recovery interval.
The [workspace chat](../architecture/2026-09-06-online-groups-share-the-workspace-chat.md)
and [identity boundary](../architecture/2026-09-05-online-identity-is-not-device-control.md)
decisions remain active; this decision separates their refresh lifecycles.

## Decision

`useOnlineGroups` reads the existing actor-scoped group endpoint directly.
The account dialog verifies cloud identity explicitly; the background mailbox
owns authenticated transport. Local history is readable independently of cloud
availability. Sign-out removes the local binding, so subsequent group requests
return forbidden and clear the sidebar projection.

Account operations share one throttling conversion in `handlers_online.rs`.
Verification preserves HTTP 429 and the upstream retry interval without
discarding identity or treating throttling as successful authentication.

## Alternatives considered

**Increase or disable the authentication rate limit.** It retains unnecessary
requests per tab and weakens a public endpoint's protection.

**Cache successful verification with a new expiration policy.** It introduces
another authority and revocation delay to fix a local-history read. Existing
mailbox authentication already owns cloud access.

## Consequences

Saved groups can remain visible while their cloud session needs reauthentication,
just as offline history remains visible. They do not imply connected presence.
The regression traverses real browser, API, PostgreSQL and Worker group traffic;
only the verification response is replaced with 429 during the reload assertion.
The API regression uses a loopback account-service fixture to verify throttling,
recovery and revocation through the production route.
