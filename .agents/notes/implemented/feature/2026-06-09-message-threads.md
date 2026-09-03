# Agent Note: Message threads

Status: implemented

## Problem

A conversation is a single flat timeline. A group with three agents produces 30 to 100 messages per task, and human coordination messages ("@reviewer please check", "deploy approved") drown in agent transcript output. The quote-reply affordance (`metadata.reply_to_id`) renders a quote block but still appends the reply to the bottom of the same timeline, so it does nothing for readability. Slack-style threading is the standard answer, but most thread traffic here is agent-generated, which raises a question Slack never has to answer: when an agent is triggered from inside a thread, where does its reply go and what does the operator still see?

## Decision

A thread is identified by its root event; there is no thread entity table. A threaded reply is an ordinary `conversation_events` row whose `reply_event_id` names the root and whose `metadata.thread` is JSON `true`, with `metadata.broadcast: true | false` deciding whether the reply also appears on the main timeline. A row with `reply_to_id` but no boolean `thread` flag is a legacy quote-reply and stays inline. The schema in [`migrations/V018__message_threads.sql`](../../../../migrations/V018__message_threads.sql) adds only the partial index `idx_conversation_events_thread (conversation_id, reply_event_id, seq)` and the lazily populated receipt table `thread_read_receipt (conversation_id, thread_root_id, principal_id, last_read_seq, last_read_at)`, whose three foreign keys cascade and whose `idx_thread_read_receipt_root` index serves the `conversation_events(event_id)` cascade on conversation delete.

The discriminator is defined once per language: `THREAD_FLAG_SQL` (`COALESCE(metadata->'thread' = 'true'::jsonb, false)`), `thread_flag_sql_for(alias)`, `ThreadFlags::from_metadata` and `ThreadFlags::bumps_conversation_unread` in `crates/choruz-store/src/conversation_events.rs`, and `partitionThreadMessages` in `apps/web/lib/messages/threads.ts`. Both use JSON-boolean semantics, so the string `"true"` is a quote-reply, and the V018 index predicate is textually identical to `THREAD_FLAG_SQL`.

Threads are flat. `EventStore::canonicalize_thread_root_in_tx` follows a reply-to-reply target to its root inside the `DbService::send_message` transaction (`crates/choruz-application/src/db_service/messages.rs`), so every read is a single-level lookup and `reply_event_id` never points at another reply. The target must be a message-like event (`event_type IN ('message','message.created','reply')`) in the same conversation; anything else is `NotFound`, and `thread: true` without `reply_to_id` is `AppError::Validation("threaded reply requires metadata.reply_to_id")`.

There is no dedicated write route. Humans post `POST /v1/messages` with `metadata: { reply_to_id, thread: true, broadcast? }`; every pipeline stage (outbox, router, fanout, idempotency) sees a normal message. Reads are `GET /v1/conversations/{conversation_id}/messages?view=timeline` (quiet replies hidden, one batched `thread_summaries` array per page), `GET /v1/conversations/{conversation_id}/threads/{thread_root_id}?since_seq&limit` and `POST …/threads/{thread_root_id}/view`, handled by `list_timeline_messages`, `list_thread_replies` and `mark_thread_viewed` behind `handlers_threads.rs` (`get_thread`, `view_thread`) and the `view=timeline` branch of `list_messages` in `handlers_messages.rs`. Thread reads share `require_conversation_read_access` with `list_messages`, so thread visibility equals conversation visibility.

## Data

The metadata keys on a reply row and who sets them:

| Key | Set by | Meaning |
|---|---|---|
| `reply_to_id` | thread composer, agent `thread` param, legacy quote-reply | the id the client targeted, preserved verbatim; lifted into `reply_event_id` and canonicalized when `thread` is true |
| `thread` | thread composer (`true`), `metadata_for_group_send_command` (`true`) | JSON boolean discriminator; anything else means quote-reply |
| `broadcast` | panel checkbox (default `false`), agent send (default `true`) | reply also appears on the main timeline and bumps `total_msg_count` |
| `trace_id` | `send_message` from the request's `x-trace-id` | correlator only; no thread semantics |

Read types in `crates/choruz-application/src/types.rs`: `TimelineMessages { messages, thread_summaries }`, `ThreadSummary { root_event_id, reply_count, last_reply_at, participant_sample }` (up to 5 sender ids, unordered), `ThreadDetail { root, replies }`. The web mirrors them as `ThreadRollup` and `ThreadPartition` in `lib/messages/threads.ts`.

## Unread semantics

A quiet threaded reply does not bump `conversation.total_msg_count` and does not auto-mark the sender as viewed; a broadcast reply does both, exactly like a timeline message (`ThreadFlags::bumps_conversation_unread`). Thread-level unread comes from `thread_read_receipt`: the `thread_unread_count` LATERAL in `get_unread_counts_scoped` counts `DISTINCT reply_event_id` with a reply newer than the caller's receipt, excluding the caller's own replies, and rides on `GET /v1/unreads` as `ConversationUnread.thread_unread_count`. Viewing a conversation never clears it; only the `/view` route does, which upserts `last_read_seq = MAX(seq)` of the thread. The trigger `trg_thread_read_sync_change` (`migrations/V026__sync_change_log.sql`) emits `thread.read_state_changed` to the receipt's principal so other tabs converge. `send_message` calls `increment_msg_count` with an empty mention list, and the code comment pins that a future mention bump must sit outside the broadcast gate.

## Agent protocol

`build_prompt` in `crates/choruz-router/src/router.rs` appends ` thread:<root>` to the `[choruz-incoming]` envelope only when the routed event has `reply_event_id` and `ThreadFlags.is_thread_reply`, in both group and direct chats; legacy quote-replies never carry it. The agent echoes the id with `{"type":"send","group":"…","content":"…","thread":"<root-id>","broadcast":false?}`, and `metadata_for_group_send_command` in `services/choruz-pipeline/src/outbox_handler.rs` writes `metadata.reply_to_id`, `metadata.thread = true` and `metadata.broadcast` with a default of `true`. A non-string or empty `thread` and a non-boolean `broadcast` are rejected rather than coerced. The rules live in the root `CLAUDE.md` and `agent-templates/core-protocol.md`, and `services/choruz-pipeline/src/instructions.rs` carries them as bootstrap version 4 so existing workspaces pick them up through the [versioned bootstrap refresh](2026-08-18-versioned-bootstrap-refresh.md).

## Web surface

`ThreadRollupInfo` in `components/chat/message-bubble.tsx` renders the "N replies" chip from `thread_summaries`; `components/chat/thread-panel.tsx` shows the root pinned above its replies with an "Also send to #channel" checkbox that defaults to unchecked, so human replies are quiet by default while agent replies are broadcast by default. `lib/messages/thread-unreads.ts` (`LocallyViewedRegistry`, `UnreadCommitGate`, `wsUnreadEffect`) decides that a broadcast reply bumps the conversation badge, that any threaded reply schedules a `/v1/unreads` refresh, and that a `/v1/unreads` response racing a local view cannot resurrect a cleared badge. `chat-app.tsx` posts the receipt when the panel opens and again, debounced, as the reply count grows.

Subsystem reference: [docs/subsystems/threads.md](../../../../docs/subsystems/threads.md). History: [Message threads RFC (archived)](../../archived/feature/2026-06-09-message-threads.md).

## Alternatives considered

- **Dedicated `thread` and `broadcast` columns on `conversation_events`**: rejected because a migration on the hottest table buys nothing the JSONB flags do not; `reply_to_id` and channel-task payloads already live in `metadata`, and the fanout envelope carries `metadata` for free. The cost of a JSONB predicate is contained by the partial index, which only admits rows that are already flagged replies.
- **A new `thread_root_id` column instead of reusing `reply_event_id`**: rejected because it double-stores the same relationship and forces a backfill decision for legacy quote-replies; reuse plus a discriminator costs one JSONB check on a filtered subset.
- **Denormalized reply counters on the root row**: rejected in favour of a per-page `GROUP BY` over the partial index; writes stay a single insert with no counter drift, and `thread_summaries` is an opaque array so a summary table can back it later without an API change.
- **Broadcast default `false` for agent thread replies**: rejected because it hides agent activity inside collapsed threads, judged too surprising for an operations tool; agents opt out per message with `"broadcast": false` for noisy intermediate updates.
- **Nested threads**: rejected in favour of flat Slack semantics; a reply to a reply canonicalizes to the root, which keeps every read a single-level lookup with no recursive CTE.
- **A dedicated sync event type for thread replies, copying the `channel_task.*` identifier-envelope pattern**: rejected because those are non-message domain events fetched separately, whereas a thread reply is a message; clients route on `metadata` and nothing in the fanout crate changes.
- **Backfilling historical quote-replies into threads**: rejected; existing `reply_event_id` rows keep rendering as quote blocks, and only rows written with `thread: true` get thread semantics.
- **Thread-level membership or follow/unfollow state**: rejected for the first release because follow state doubles the receipt table's semantics; every conversation member sees thread unread, and follow state is an additive column on `thread_read_receipt` if it ever proves noisy.
- **A `0NNN`-series migration**: rejected because the index targets `conversation_events`, which `V001` creates, and the `0NNN` series applies before every V-file.

## Consequences

- Every existing pipeline stage handles threaded replies unchanged; the whole feature is a write-time canonicalization plus read-path filtering over data the table already stores.
- The discriminator is duplicated across SQL, Rust and TypeScript by design, and drift between them is a correctness bug; `thread_flags_require_json_booleans`, `v018_partial_index_predicate_matches_thread_flag_sql` and `thread_flag_sql_for_qualifies_the_shared_predicate` in `conversation_events.rs` and "requires JSON boolean true plus a non-empty reply_to_id" in `lib/messages/threads.test.ts` pin it.
- Agent threads still add main-timeline volume until agents use `"broadcast": false`, which is the accepted price of operator visibility; `metadata_for_group_send_injects_thread_fields` pins the default.
- The envelope carries only the root id, so an agent that needs earlier thread context must fetch it; the executor injects no thread transcript lines.
- Canonicalization, target validation, counter gating, receipts and cascade are pinned by `thread_reply_canonicalizes_to_root`, `thread_reply_rejects_bad_targets`, `thread_reply_counter_semantics`, `quiet_thread_reply_preserves_sender_unread_state`, `thread_detail_and_receipts` and `delete_conversation_cascades_thread_receipts` in `services/choruz-api-gateway/src/tests/`, `send_to_group_threads_canonicalize_and_gate_unread` in `outbox_handler.rs`, and `build_prompt_adds_thread_field_for_threaded_replies` in `router.rs`.
- The end-to-end contract is pinned by `apps/web/tests/threads.spec.ts` ("thread rollup, side panel, and quiet-reply timeline filtering", "thread unread badge: lights on agent reply, survives conversation view, clears on thread view", "broadcast reply shows in both the timeline and the thread").
