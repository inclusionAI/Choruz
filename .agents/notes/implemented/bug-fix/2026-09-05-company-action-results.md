# Agent Note: Company action results

Status: implemented

## Problem

Company operations can succeed for one selection and fail for another. Discarding individual errors makes a completed loop look like a completed batch and leaves no visible recovery path.

## Decision

The sidebar uses one action owner for single and batch archive, unarchive and delete. It counts fulfilled mutations, names rejected selections, and retains only failed selections for a batch retry. A partial result is an error in the trace, not an all-success event. Selection controls are disabled while the batch runs.

## Alternatives considered

**Separate single and batch handlers.** Duplicated error handling makes their reporting diverge. The existing mutation callbacks already propagate failure and update only successful Company state.

**Retry the whole selection.** Repeating completed mutations obscures partial progress. Keeping only failures makes the retry scope explicit without new server state.

## Consequences

The UI reports request outcomes, not an atomic batch transaction. Browser tests inject one failed request, verify persisted partial results and retry through the normal menu. An ambiguous network failure after a server commit still requires reconciliation; this owner does not invent transaction guarantees.
