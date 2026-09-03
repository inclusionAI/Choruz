# Threads

Message threads are Slack-style side conversations under any message: a threaded reply is an ordinary `conversation_events` row whose `reply_event_id` points at the thread root and whose `metadata.thread` is JSON `true`, optionally broadcast to the main timeline with `metadata.broadcast: true`. A reader can use this page to find the single discriminator definition per language, the two read endpoints, the receipt table that drives per-thread unread, and how agents receive and echo `thread:` context. Source: [`../../crates/choruz-application/src/db_service/messages.rs`](../../crates/choruz-application/src/db_service/messages.rs) (write and read paths) with the schema in [`../../migrations/V018__message_threads.sql`](../../migrations/V018__message_threads.sql).

## Owns

- Schema: the partial index `idx_conversation_events_thread (conversation_id, reply_event_id, seq) WHERE reply_event_id IS NOT NULL AND COALESCE(metadata->'thread' = 'true'::jsonb, false)` and the table `thread_read_receipt` with indexes `idx_thread_read_receipt_principal` and `idx_thread_read_receipt_root`, all in [`V018__message_threads.sql`](../../migrations/V018__message_threads.sql).
- Sync trigger: `trg_thread_read_sync_change` on `thread_read_receipt` in [`V026__sync_change_log.sql`](../../migrations/V026__sync_change_log.sql) emits `thread.read_state_changed` (entity_type `thread_read_state`) to the receipt's principal.
- Discriminator: `THREAD_FLAG_SQL`, `thread_flag_sql_for(alias)`, `ThreadFlags::from_metadata`, `ThreadFlags::bumps_conversation_unread` and `EventStore::canonicalize_thread_root_in_tx` in [`crates/choruz-store/src/conversation_events.rs`](../../crates/choruz-store/src/conversation_events.rs).
- Service: `DbService::send_message` (thread branch), `list_timeline_messages`, `list_thread_replies`, `mark_thread_viewed`, and the `thread_unread_count` LATERAL in `get_unread_counts_scoped`, all in [`messages.rs`](../../crates/choruz-application/src/db_service/messages.rs).
- Gateway: [`handlers_threads.rs`](../../services/choruz-api-gateway/src/handlers_threads.rs) (`get_thread`, `view_thread`, `require_conversation_read_access`), the `?view=timeline` branch of `list_messages` in [`handlers_messages.rs`](../../services/choruz-api-gateway/src/handlers_messages.rs), and the route registrations in [`lib.rs`](../../services/choruz-api-gateway/src/lib.rs).
- Agent side: the `thread_suffix` in `build_prompt` ([`crates/choruz-router/src/router.rs`](../../crates/choruz-router/src/router.rs)) and `metadata_for_group_send_command` in [`services/choruz-pipeline/src/outbox_handler.rs`](../../services/choruz-pipeline/src/outbox_handler.rs); the rules in the root [`CLAUDE.md`](../../CLAUDE.md) and [`agent-templates/core-protocol.md`](../../agent-templates/core-protocol.md).
- Web: [`lib/messages/threads.ts`](../../apps/web/lib/messages/threads.ts), [`lib/messages/thread-unreads.ts`](../../apps/web/lib/messages/thread-unreads.ts), [`components/chat/thread-panel.tsx`](../../apps/web/components/chat/thread-panel.tsx), the `ThreadRollupInfo` chip and `onOpenThread` action in [`components/chat/message-bubble.tsx`](../../apps/web/components/chat/message-bubble.tsx), `fetchThread` and `markThreadViewed` in [`lib/api/choruz-api.ts`](../../apps/web/lib/api/choruz-api.ts), and the thread state in [`components/chat/chat-app.tsx`](../../apps/web/components/chat/chat-app.tsx) (`openThreadRootId`, `handleOpenThread`, `postThreadReceipt`).

## Data

A threaded reply row: `conversation_events.reply_event_id = <root event_id>` (canonicalized at write time; threads are flat) and `metadata` containing `reply_to_id` (the id the client targeted, preserved verbatim), `thread: true` and `broadcast: true | false`. A row with `reply_to_id` but no boolean `thread` flag is a legacy quote-reply and stays on the timeline.

`ThreadFlags { is_thread_reply, is_broadcast }` is parsed with `as_bool()`, so the string `"true"` is not a thread; `THREAD_FLAG_SQL` uses jsonb equality for the same reason, and the V018 index predicate is textually identical to it.

`thread_read_receipt (conversation_id, thread_root_id, principal_id, last_read_seq, last_read_at)` with the composite primary key; rows appear lazily on first view and cascade from `conversation`, `conversation_events(event_id)` and `principal`.

Read types in [`crates/choruz-application/src/types.rs`](../../crates/choruz-application/src/types.rs): `TimelineMessages { messages, thread_summaries }`, `ThreadSummary { root_event_id, reply_count, last_reply_at, participant_sample }` (up to 5 sender ids, unordered), `ThreadDetail { root, replies }`; `ConversationUnread.thread_unread_count` is `COUNT(DISTINCT reply_event_id)` of threads with a reply newer than the caller's receipt, excluding the caller's own replies.

Agent envelope: `[choruz-incoming] from:@Alice group:proj-team conv:019d… thread:<root-id> roster:[…] | …`; the `thread:` field appears only when the routed event has `reply_event_id` and `ThreadFlags.is_thread_reply`. Agent send: `{"type":"send","group":"…","content":"…","thread":"<root-id>","broadcast":false?}`; the handler writes `metadata.reply_to_id`, `metadata.thread = true` and `metadata.broadcast` (default `true`).

The metadata keys and who sets them:

| Key | Set by | Meaning |
|---|---|---|
| `reply_to_id` | thread composer, agent `thread` param, legacy quote-reply | target message id; lifted into `reply_event_id` and canonicalized when `thread` is true |
| `thread` | thread composer (`true`), `metadata_for_group_send_command` (`true`) | JSON boolean discriminator; anything else means quote-reply |
| `broadcast` | panel checkbox (default `false`), agent send (default `true`) | reply also appears on the main timeline and bumps `total_msg_count` |
| `trace_id` | `send_message` from the request's `x-trace-id` | correlator only; no thread semantics |

Web: `ThreadRollup { rootId, replies, replyCount, lastReplyAt, participantIds }` and `ThreadPartition { timeline, rollups }` from `partitionThreadMessages`; `UnreadEntry { unread, mentions, threadUnread? }`, `UnreadRow` (`thread_unread_count?`), `LocallyViewedRegistry`, `UnreadCommitGate` and `WsUnreadEffect { bumpConversationUnread, refreshThreadUnread }` from `lib/messages/thread-unreads.ts`; `ThreadPanelProps.onSendReply(content, broadcast)`.

## Entry points

- Write: `POST /v1/messages` with `metadata: { reply_to_id, thread: true, broadcast? }` (humans; the thread panel's "Also send to #channel" checkbox sets `broadcast`), or the agent `send` command with `"thread"`. No dedicated write route exists; `send_message` validates `thread: true` without `reply_to_id` as `AppError::Validation("threaded reply requires metadata.reply_to_id")`.
- Read: `GET /v1/conversations/{conversation_id}/messages?view=timeline` (quiet replies hidden, `thread_summaries` per visible root); `GET /v1/conversations/{conversation_id}/threads/{thread_root_id}?since_seq&limit` (limit clamped to 1..200, a reply id canonicalizes to its root); `GET /v1/unreads` for `thread_unread_count`.
- Receipt: `POST /v1/conversations/{conversation_id}/threads/{thread_root_id}/view` → `mark_thread_viewed` upserts `last_read_seq = MAX(seq)` of the thread; answers `204`.
- Live updates: replies arrive through the normal sync feed as message rows carrying full `metadata`; `wsUnreadEffect` decides whether to bump the conversation badge (broadcast only) and always schedules a `/v1/unreads` refresh for a threaded reply; `thread.read_state_changed` changes also trigger that refresh.
- Agent turn: the router adds `thread:<root>` to the envelope; the agent replies with `"thread":"<root>"` and the reply lands in the thread, broadcast by default so operators keep timeline visibility.
- Receipt cadence: `chat-app.tsx` posts the receipt when the panel opens and again, debounced, as `openThreadReplyCount` grows, flushing the pending `{ convId, rootId }` when the panel closes or the thread changes.
- Web panel: clicking the rollup chip or "open thread" calls `handleOpenThread`, which resolves the root locally (`resolveThreadRoot`), calls `fetchThread`, merges with `mergeThreadReplies`, renders `ThreadPanel`, and posts the receipt while the panel is open (`postThreadReceipt`).

## Invariants

- `reply_event_id` always names the thread root: `canonicalize_thread_root_in_tx` follows a reply to its root and `list_thread_replies` / `mark_thread_viewed` canonicalize on read; pinned by `thread_reply_canonicalizes_to_root` and `thread_detail_and_receipts` in [`services/choruz-api-gateway/src/tests/threads.rs`](../../services/choruz-api-gateway/src/tests/threads.rs), `send_to_group_threads_canonicalize_and_gate_unread` in `outbox_handler.rs`, and "attaches reply-to-reply messages to the canonical root" in [`lib/messages/threads.test.ts`](../../apps/web/lib/messages/threads.test.ts).
- A thread can only root on a message-like event (`event_type IN ('message','message.created','reply')`) in the same conversation; unknown or cross-conversation targets are `NotFound`; pinned by `thread_reply_rejects_bad_targets`.
- The discriminator is defined once per language and the index predicate matches it; pinned by `thread_flags_require_json_booleans`, `v018_partial_index_predicate_matches_thread_flag_sql` and `thread_flag_sql_for_qualifies_the_shared_predicate` in `conversation_events.rs`, and "requires JSON boolean true plus a non-empty reply_to_id" in `threads.test.ts`.
- A quiet reply does not bump `total_msg_count` or auto-mark the sender as viewed; a broadcast reply does both (`bumps_conversation_unread`); pinned by `thread_reply_counter_semantics`, `quiet_thread_reply_preserves_sender_unread_state`, the outbox test above, and `wsUnreadEffect` cases in [`lib/messages/thread-unreads.test.ts`](../../apps/web/lib/messages/thread-unreads.test.ts).
- Viewing a conversation never clears its thread counter; only `POST …/threads/{root}/view` does (`clearConversationUnread` preserves `threadUnread`); pinned by `thread_detail_and_receipts` and the e2e "thread unread badge: lights on agent reply, survives conversation view, clears on thread view".
- Agent thread replies broadcast by default, and a non-boolean `broadcast` or empty `thread` is rejected rather than coerced; pinned by `metadata_for_group_send_injects_thread_fields`.
- The envelope carries `thread:` only for flagged replies, never for legacy quote-replies, in both group and direct chats; pinned by `build_prompt_adds_thread_field_for_threaded_replies` in `router.rs`.
- Deleting a conversation removes its receipts through the `thread_root_id` cascade (served by `idx_thread_read_receipt_root`); pinned by `delete_conversation_cascades_thread_receipts`.
- Thread reads share `require_conversation_read_access` with `list_messages`, so thread visibility equals conversation visibility.

## Failure modes

- `POST /v1/messages` with `thread: true` and a missing target answers `404 thread target message not found`; with no `reply_to_id` it answers a validation error. The agent path returns the same errors from `send_to_group`.
- `GET …/threads/{root}` on an id that is not a message in that conversation answers `404 thread root not found`; the panel shows `threadError` and keeps the local rollup.
- `POST …/threads/{root}/view` is rate limited through `check_rate_limit` and returns `404` for a bad root; the client posts the receipt fire-and-forget and relies on the next `/v1/unreads` poll to converge.
- A `/v1/unreads` response racing a local view is gated by `UnreadCommitGate.tryCommit` and the `LocallyViewedRegistry` consume rule, so an older response cannot resurrect a cleared badge or permanently suppress new unreads.
- A `fetchThread` that settles after the user switched threads is discarded by `openThreadRequestSeqRef` in `chat-app.tsx`.
- Every thread write logs `send_message_succeeded` with `is_thread_reply`, `is_broadcast` and `thread_root`, and canonicalization logs `thread reply canonicalized to root`, which is how an operator confirms where a reply landed.

## Tests

- Rust: `thread_reply_canonicalizes_to_root`, `thread_reply_counter_semantics`, `thread_reply_rejects_bad_targets`, `thread_detail_and_receipts`, `quiet_thread_reply_preserves_sender_unread_state`, `delete_conversation_cascades_thread_receipts` in [`services/choruz-api-gateway/src/tests/threads.rs`](../../services/choruz-api-gateway/src/tests/threads.rs); `thread_flags_require_json_booleans`, `v018_partial_index_predicate_matches_thread_flag_sql`, `thread_flag_sql_for_qualifies_the_shared_predicate` in [`crates/choruz-store/src/conversation_events.rs`](../../crates/choruz-store/src/conversation_events.rs); `build_prompt_adds_thread_field_for_threaded_replies` in [`crates/choruz-router/src/router.rs`](../../crates/choruz-router/src/router.rs); `metadata_for_group_send_injects_thread_fields` and `send_to_group_threads_canonicalize_and_gate_unread` in [`services/choruz-pipeline/src/outbox_handler.rs`](../../services/choruz-pipeline/src/outbox_handler.rs).
- Web unit: [`lib/messages/threads.test.ts`](../../apps/web/lib/messages/threads.test.ts), [`lib/messages/thread-unreads.test.ts`](../../apps/web/lib/messages/thread-unreads.test.ts), thread cases in [`components/chat/message-list.test.ts`](../../apps/web/components/chat/message-list.test.ts).
- E2E: [`tests/e2e/threads.spec.ts`](../../apps/web/tests/e2e/threads.spec.ts) ("thread rollup, side panel, and quiet-reply timeline filtering", "thread unread badge: lights on agent reply, survives conversation view, clears on thread view", "broadcast reply shows in both the timeline and the thread").

## Related

- [store.md](store.md) — `conversation_events`, `server_seq` and `DbService::send_message`.
- [sync-feed.md](sync-feed.md) — how replies and `thread.read_state_changed` reach the browser, and `/v1/unreads`.
- [agent-protocol.md](agent-protocol.md) — the envelope and send-command grammar the `thread` / `broadcast` keys extend.
- [message-pipeline.md](message-pipeline.md) — the router and outbox handler that carry thread context.
- [web-client.md](web-client.md) — the chat shell that hosts the thread panel.
