# Agent Note: Raise the message that owns an open actions menu

Status: implemented

## Problem

A message arriving below an open actions menu covered its Reply button. The message entrance animation retains a transform and therefore a stacking context; increasing the menu child's z-index cannot escape that context. Parallel outbox tests exposed the ordinary interaction by sending into whichever shared group happened to be first.

## Decision

`quote-reply.css` raises the owning `.msg-group` to the existing menu layer while its actions button is expanded. The menu remains in its existing React owner, retaining click-away and keyboard handling. Mutating outbox tests create their own uniquely named groups rather than borrowing a shared conversation.

## Alternatives considered

**Raise only the menu.** Its parent stacking context still sits below later message rows.

**Portal every message menu.** This needs new positioning and lifecycle handling for a defect the existing positioned message owner can resolve.

**Only isolate the tests.** That removes the accidental concurrent sender but leaves the same failure when a real participant sends a message.

## Consequences

The open menu's message is above neighboring messages until it closes. No global overlay state, new z-index scale or conversation behavior is introduced.

## Testing

`messaging.spec.ts` opens actions, delivers another message through the real API, and clicks Reply without forced interaction. The assertion fails on the base stylesheet because the later message intercepts pointer events. `outbox.spec.ts` keeps delivery, ordering and idempotency assertions on test-owned groups.
