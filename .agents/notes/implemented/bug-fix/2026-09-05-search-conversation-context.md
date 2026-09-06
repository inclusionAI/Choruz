# Agent Note: Search belongs to its conversation and query

Status: implemented

## Problem

The detail panel survives conversation switches. A retained query can display another conversation's results, and a slower previous request can replace the current query's results.

## Decision

`useMessageSearch` clears the query when its conversation or authenticated user changes. One query effect owns its debounce and response updates; cleanup cancels the timer and makes late completions inert. Selecting a result clears the query through that same lifecycle.

## Alternatives considered

**Reset only the panel tab.** This hides the stale results until Search is reopened without invalidating the request that owns them.

**Track another request counter.** The effect cleanup already identifies which query may update state, so another sequence owner is unnecessary.

## Consequences

Switching chats starts a fresh search. The hook removes its separate timer ref and imperative request callback. The browser regression delays a real search response until another conversation's query completes, then verifies that the newer result remains visible. [Historical result navigation](2026-09-05-search-message-navigation.md) owns loading and locating a selected message, independently of query cancellation.
