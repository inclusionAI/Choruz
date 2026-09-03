# Agent Note: Runtime bindings ride the bootstrap and the sync feed

Status: implemented

## Problem

Opening a freshly created Claude Code agent's DM could sit on an empty chat view. The terminal pane mounts only when the client finds a terminal binding for the conversation in `runtimeBindings`, and that list lived outside the data path everything else uses: the dashboard fetched it once, separately from `/v1/bootstrap`, and swallowed a failure into `[]`; afterwards only three flows re-fetched it. The database already emitted `runtime_binding.created|updated|deleted` into `sync_change`, but `handleSyncChanges` treated them as unknown events and answered each with a full `/v1/bootstrap` refresh plus `GET /v1/runtime/bindings`, awaited inside the serialised apply chain. That list endpoint looped over every binding with four to five sequential queries each (`get_principal`, `get_conversation`, one `get_principal` per member for the label), and dropped a binding without a log when any lookup failed. A binding's `state` and `config_json` change on every turn and capture, so a busy agent made every open dashboard re-run the whole snapshot repeatedly while the feed waited. The transcript decision was also client-side (`isTerminalDriver` over a hard-coded list), so the `mathcode_terminal` plugin driver never got a terminal.

## Decision

Bindings use the same path as conversations. `GET /v1/bootstrap` carries `runtime_bindings`, read under the same `sync_cursor`; the dashboard page seeds `ChatApp` from it and no longer fetches bindings on its own; `applyBootstrap` sets the list. `handleSyncChanges` handles `runtime_binding.*` by id: created and updated re-read that one binding through `GET /v1/runtime/bindings/{binding_id}` and upsert it, deleted (or a re-read that fails) removes it; no snapshot refresh. On the gateway `list_binding_views` is one JOIN over `agent_runtime_bindings`, `principal` and `conversation` with the member label computed in SQL, scoped by the caller's workspaces; the list, the by-id read and the bootstrap all use it. Every view carries `interaction_mode`, filled from the stored value or from `is_terminal_driver` (`terminal`) else `message`, and the web decides the transcript from that field alone. An agent DM with no binding keeps the message view: an agent created through `POST /v1/agents` alone talks over messages and has no binding by design, so a missing binding is not an error state; the `terminal_binding_missing` trace still records it.

## Alternatives considered

- **Retry or poll bindings from the client when a DM has none**: rejected. It keeps a second data path with its own failure modes and adds latency on every miss; the feed already reports the change.
- **Put the full binding view into the trigger payload** so the client never re-reads: rejected. The payload would duplicate the label and workspace scoping logic in PL/pgSQL, and a per-recipient row per turn would carry redacted `last_error` text into the feed; one bounded GET per change is cheaper and keeps one view implementation.
- **Narrow the update trigger** so turn-time `state`/`config_json` changes stop firing: rejected for now. Other devices rely on `state` to show a busy agent; with a per-binding re-read the events are cheap.
- **Render an explicit "no runtime binding" state for an agent DM without a binding**: rejected after `theme.spec.ts` showed why: API-created agents have no binding and are message conversations; the state would have hidden their composer.
- **Resolve the terminal binding on the gateway when the WebSocket opens (connect by conversation id)**: not done. The client still needs bindings for account names, machines and the runtime panel, so the list must be correct anyway; with it in the bootstrap and the feed, a by-conversation socket adds a second lookup path without removing one.

## Consequences

- One query replaces N×5; `GET /v1/runtime/bindings` and the bootstrap return the same rows, pinned by `bootstrap_carries_runtime_bindings_and_the_feed_names_new_ones`.
- A binding change costs one small GET per open dashboard instead of a snapshot refresh, and the apply chain no longer waits on the snapshot.
- Plugin drivers get a terminal without a client list; `bindingUsesTerminalTranscript` reads `interaction_mode` only.
- The `terminal_binding_missing` trace now fires only when the gateway itself has no binding for the DM.
