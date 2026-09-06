# Agent Note: Attachment send recovery

Status: implemented

## Problem

A composer submission sends files sequentially before its text. Restoring the
entire file queue after partial failure duplicates accepted file messages on
retry. Attachment-only replies also need the selected quote and must not leave
that quote attached to a later unrelated message.

## Decision

The existing composer callback reports each accepted attachment message. The
input keeps a submission-local completed count and restores only the remaining
suffix on failure, alongside its text. Upload completion alone is not acceptance:
the attachment's message must succeed before reporting progress.

Attachment metadata carries the captured reply target. A successful file-only
batch clears that target only if the current selection still matches; failure
keeps it for retry. Text sending retains its existing reply handling.

## Alternatives considered

**Retry the entire batch.** Rejected because completed messages are independent
durable results, not a transaction that can be rolled back by the composer.

**Introduce a persistent upload queue.** Rejected for this recovery boundary:
the existing in-memory files and sequential send callback already own the work.

## Consequences

`apps/web/tests/e2e/attachment.spec.ts` checks partial file failure, subsequent
text failure and retries against persisted message counts, plus file-only quote
metadata and the next unquoted text. Failure responses replace only the owned
request that must fail; successful uploads and messages reach the real service.
This does not add recovery for ambiguous network acknowledgement or browser exit.
