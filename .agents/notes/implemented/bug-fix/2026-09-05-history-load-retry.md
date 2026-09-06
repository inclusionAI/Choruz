# Agent Note: Failed history loads have an explicit retry

Status: implemented

## Problem

The message list swallows history-load failures. Users cannot tell why older messages stop appearing or how to retry without scrolling again.

## Decision

`MessageList` displays a conversation-scoped error and Retry button when its existing history loader rejects. Retry uses the same loader and prepend-position restoration as scrolling. A visible failure pauses automatic scroll retries, and switching conversations clears it. A late failure from another conversation cannot set the current error.

## Alternatives considered

**Automatically retry on every scroll.** This leaves failure unexplained and can repeatedly request an unavailable service while the user reads loaded messages.

**Add a second pagination owner for Retry.** The parent already owns the cursor and in-flight state; the list only needs to present the failed attempt and invoke the same loader.

## Consequences

Loaded messages remain available after a failed request. The browser regression fails one real conversation's history request, checks its error and retained messages, then retries through the real API and renders the oldest seeded message. Native terminal history is outside this paginated message-list contract.
