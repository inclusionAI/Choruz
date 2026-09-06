# Agent Note: Locate search results through the existing history owner

Status: implemented

## Problem

Selecting a message search result only selected its conversation. Historical
targets outside the loaded timeline could not be scrolled to or highlighted.

## Decision

The result callback carries both conversation and message IDs. ChatApp owns
the pending selection; MessageList requests one existing history page at a
time until the target is loaded, then reuses its virtual-scroll/highlight path.
Rendering and in-flight state separate requests rather than a synchronous loop.
Cancel or leaving the conversation/view stops further navigation-driven pages;
an already-started page may complete into its conversation cache.

History errors retain the existing Retry owner and pause automatic requests.
An exhausted history reports the unavailable target. A queued bottom scroll
rechecks following intent, so mounting Chat from Tasks cannot steal navigation.
After virtualization renders the target, its measured DOM position corrects
the estimated scroll before highlighting. Successful navigation completes after
that alignment; Cancel invalidates any queued alignment, including a loaded
replacement waiting for an earlier history page.

Quiet thread replies load contiguous history through their root before the
existing thread loader opens the panel and highlights the reply. They are not
inserted as isolated main-timeline messages.

## Alternatives considered

**Fetch and merge only the target.** Rejected because the oldest loaded cursor
would jump across an unloaded interval, hiding the missing history.

**A second search pagination store.** Rejected because normal scroll and Retry
already own the cursor, cache merge, in-flight guard and failure state.

## Consequences

Very old targets can require several bounded page requests. Cancel stops
continuation rather than introducing a new transport cancellation protocol.
The existing [query lifecycle](2026-09-05-search-conversation-context.md) and
[history retry](2026-09-05-history-load-retry.md) decisions remain independent.
