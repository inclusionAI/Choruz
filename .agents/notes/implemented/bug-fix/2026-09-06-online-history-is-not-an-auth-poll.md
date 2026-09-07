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
return forbidden and clear the sidebar projection. HTTP 401/403 stops the group
poll until explicit refresh or an account change. Successful sign-in and
sign-out invalidate account-owned views in this tab and other same-origin tabs
for the same local principal; the notification contains no credentials.

Unchanged group projections retain their React state reference. Hidden or
offline browsers pause polling. Transient failures retain saved groups and use
capped exponential backoff, respecting a longer `Retry-After` interval. Returning
to the foreground or reconnecting preserves that interval. Effect cleanup aborts
requests and removes timers and listeners before a replacement poll takes over.

Account operations share one throttling conversion in `handlers_online.rs`.
Verification preserves HTTP 429 and the upstream retry interval without
discarding identity or treating throttling as successful authentication.

## Alternatives considered

**Increase or disable the authentication rate limit.** It retains unnecessary
requests per tab and weakens a public endpoint's protection.

**Cache successful verification with a new expiration policy.** It introduces
another authority and revocation delay to fix a local-history read. Existing
mailbox authentication already owns cloud access.

**Only reuse an empty array after forbidden responses.** It avoids one source
of render churn but keeps sending requests for an unavailable capability. A
terminal authorization response needs a lifecycle transition, not another timer.

## Consequences

Saved groups can remain visible while their cloud session needs reauthentication,
just as offline history remains visible. They do not imply connected presence.
The regression traverses real browser, API, PostgreSQL and Worker group traffic;
only the verification response is replaced with 429 during the reload assertion.
The API regression uses a loopback account-service fixture to verify throttling,
recovery and revocation through the production route.
Local-only installations make one group availability request on dashboard load;
the existing forbidden response is not polled repeatedly. Browser clock tests
cover this absence guarantee and account-change recovery, while focused unit
tests pin projection equality and retry timing.
