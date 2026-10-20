# Agent Note: Route template jobs and reconcile browser caches through their shared owners

Status: implemented

## Problem

Group-template provisioning calls browser `fetch` directly, outside the transport selected by Remote Control. A browser controlling another computer can therefore send provisioning requests to its own origin. IndexedDB hydration independently chooses a message array by length, dropping newer bootstrap messages when the cache is longer. Bootstrap application timestamps a snapshot after its response arrives, although the flag reconciler requires the request's start time to protect acknowledged local changes.

## Decision

`apps/web/lib/groups/create-group-template-flow.ts` delegates HTTP delivery to `transportFetch` while retaining its job-specific response handling. The target's Next.js routes remain the provisioning owner; a missing target route does not redirect the operation to the controller.

`chat-app.tsx` hydrates cached messages through `mergeFetchedMessages`. Bootstrap refresh and continuation capture their start time before HTTP delivery and pass that time to `useConversationFlags`. The existing flag override engine owns stale-snapshot reconciliation.

## Alternatives considered

- **Separate remote template provisioning:** rejected because the relay carries Next.js requests with the target session; copying the job lifecycle introduces a second owner.
- **Prefer the longest message array:** rejected because array length does not establish freshness or preserve messages delivered during cache loading.
- **Delay all mutations until snapshots finish:** rejected because request-start ordering preserves responsive local actions without adding another mutation queue.

## Consequences

Remote template provisioning depends on the target's web provisioning capability. Cache hydration retains both history and newer messages. Flag reconciliation preserves changes acknowledged after a snapshot request starts. Deterministic browser barriers cover delayed cache opening and bootstrap responses; the remote scenario uses the encrypted transport and reads the resulting group from the target's bootstrap.
