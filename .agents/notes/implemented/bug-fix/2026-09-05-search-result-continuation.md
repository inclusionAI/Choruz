# Agent Note: Continue search through the existing query owner

Status: implemented

## Problem

A message search returns only its first bounded result set. Without a continuation
action, an older match is unreachable even though the query matches it.

## Decision

The existing search endpoint accepts an exclusive `(created_at, message_id)`
cursor. `DbService::search_messages` owns one query for scoped and global search,
including active conversation membership and workspace/company reachability.
The unique message ID breaks timestamp ties; newer inserts cannot shift an
older-page boundary. The response remains an array of search results.

`useMessageSearch` requests one lookahead row and displays Load more while another
page exists. Failed requests preserve loaded matches and retry the same cursor.
The existing effect cleanup invalidates prior page completions when the query,
conversation or authenticated identity changes. Selecting a result still uses
the existing contiguous history and thread navigation owners.

## Alternatives considered

**Raise the fixed result limit.** This moves the silent cutoff without making
older matches reachable and increases every initial query's payload.

**Use message-history sequence cursors.** Sequence numbers are local to each
conversation, so they cannot order global search results.

**Offset pagination or a separate search pager service.** Offsets shift under
new inserts; another service would duplicate the endpoint's authorization and
the hook's query lifetime.

## Consequences

Each request remains bounded; loading many result pages grows the panel's result
list. Search uses the existing substring query, not a new full-text index or
snapshot store. The independent [query cancellation](2026-09-05-search-conversation-context.md)
and [selected-message navigation](2026-09-05-search-message-navigation.md)
decisions remain active.
