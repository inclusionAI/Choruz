# Store

The store is the PostgreSQL-backed service layer behind the API gateway: `DbService` in `crates/choruz-application` owns every read and write of workspaces, principals, conversations, messages, receipts, and audit rows, `EventStore` in `crates/choruz-store` owns the connection pool and the `conversation_events` / `event_outbox` primitives shared with the pipeline, `crates/choruz-domain` defines the row types, and `crates/choruz-infrastructure` provides tracing setup. Source: [`crates/choruz-application/src/db_service/mod.rs`](../../crates/choruz-application/src/db_service/mod.rs).

## Owns

- [`crates/choruz-application/src/db_service/`](../../crates/choruz-application/src/db_service/mod.rs): `DbService { store: EventStore, rate_limiter: Arc<RateLimiter> }` with one module per aggregate: [`principals.rs`](../../crates/choruz-application/src/db_service/principals.rs), [`companies.rs`](../../crates/choruz-application/src/db_service/companies.rs), [`conversations.rs`](../../crates/choruz-application/src/db_service/conversations.rs), [`messages.rs`](../../crates/choruz-application/src/db_service/messages.rs), [`events.rs`](../../crates/choruz-application/src/db_service/events.rs), [`audit.rs`](../../crates/choruz-application/src/db_service/audit.rs), [`group_workflow_tasks.rs`](../../crates/choruz-application/src/db_service/group_workflow_tasks.rs), [`sync.rs`](../../crates/choruz-application/src/db_service/sync.rs), and the row mappers in [`helpers.rs`](../../crates/choruz-application/src/db_service/helpers.rs) (`row_to_principal`, `row_to_conversation`, `row_to_member`, `row_to_company`, `row_to_audit_log`, `row_to_event_envelope`).
- [`crates/choruz-application/src/types.rs`](../../crates/choruz-application/src/types.rs): request and response structs (`SendMessageRequest`, `MessagePage`, `ConversationUnread`, `ConversationBootstrapEntry`, `SyncChange`, ...) re-exported from `crates/choruz-application/src/lib.rs`.
- [`crates/choruz-application/src/lib.rs`](../../crates/choruz-application/src/lib.rs): `ChatApp`, the in-memory shell held in `ApiState` for process-local event delivery; PostgreSQL remains the only durable source of truth.
- [`crates/choruz-store`](../../crates/choruz-store/src/lib.rs): `EventStore` ([`pool.rs`](../../crates/choruz-store/src/pool.rs), deadpool over `tokio-postgres`), [`conversation_events.rs`](../../crates/choruz-store/src/conversation_events.rs) (`ConversationEvent`, `ConversationEventRow`, `ThreadFlags`, `THREAD_FLAG_SQL`, `thread_flag_sql_for`), [`event_outbox.rs`](../../crates/choruz-store/src/event_outbox.rs) (`OutboxEntry`, `OutboxRow`), `cdc_poller.rs`, `redis_pool.rs`.
- [`crates/choruz-domain/src/lib.rs`](../../crates/choruz-domain/src/lib.rs): `Principal`, `PrincipalType`, `ChannelVisibility`, `Company`, `CompanyMember`, `Conversation`, `ConversationType`, `ConversationMember`, `Message`, `ReadReceipt`, `AuditLog`, `EventEnvelope`.
- [`crates/choruz-infrastructure/src/lib.rs`](../../crates/choruz-infrastructure/src/lib.rs): `init_tracing(service_name, log_format)`; [`crates/choruz-common/src/lib.rs`](../../crates/choruz-common/src/lib.rs): `AppError`, `AppResult`, `PgConfig::from_env`, `new_id`, `now`.
- Schema: [`migrations/`](../../migrations/0001_init.sql), applied by [`infra/host/migrate.sh`](../../infra/host/migrate.sh).

Tables and the module that owns them:

| Table | Created in | Owner |
|---|---|---|
| `principal` (`workspace_id`, `type`, `name`, `secret_hash`, `disabled`, `deleted_at`, `channel_visibility`) | `0001_init.sql`, `0025_channel_kanban_board.sql` | `principals.rs` |
| `company`, `company_member` | `0011b_company_tables.sql` | `companies.rs` |
| `conversation` (`workspace_id`, `type`, `name`, `creator_id`, `total_msg_count`) | `0001_init.sql`, `0016_unread_counts.sql` | `conversations.rs`, `messages.rs` |
| `conversation_member` (`conv_id`, `principal_id`, `joined_at`, `removed_at`, `msg_count`, `mention_count`, `last_viewed_at`) | `0001_init.sql`, `0016_unread_counts.sql` | `conversations.rs`, `messages.rs` |
| `conversation_pin`, `conversation_archive`, `conversation_hidden` (PK `(principal_id, conv_id)`) | `0023_conversation_pins.sql`, `0030_conversation_archives.sql`, `V034__hidden_agent_sessions.sql` | `conversations.rs` |
| `conversation_activity` (`last_event_seq`, `last_event_id`, `last_activity_at`) | `V025__conversation_activity.sql` | trigger `trg_conversation_activity`; read by `list_conversation_bootstrap_page` |
| `conversation_events` (PK `(conversation_id, seq)`, `event_id` unique, `client_msg_id`, `turn_id`, `reply_event_id`) | `V001__message_pipeline_schema.sql`, `V018__message_threads.sql` | `messages.rs`, `EventStore` |
| `event_outbox` (`aggregate_type`, `aggregate_id`, `payload`, `published`, `claimed_by`, `claim_deadline`) | `V001__message_pipeline_schema.sql`, `V007__outbox_claim_lease.sql` | `messages.rs` writes, `EventStore` claims |
| `outbox_event` (`principal_id`, `event_type`, `payload`, `delivery_seq`, `acknowledged_at`) | `0001_init.sql`, `0014_outbox_event_delivery_seq.sql` | `messages.rs` writes, `events.rs` reads and acks |
| `event_webhook` | `0020_event_webhook_secret.sql` | `events.rs` |
| `thread_read_receipt` (`conversation_id`, `thread_root_id`, `principal_id`, `last_read_seq`) | `V018__message_threads.sql` | `messages.rs` (`mark_thread_viewed`) |
| `attachment` | `V006__attachment_metadata.sql` | `messages.rs` checks, `services/choruz-api-gateway/src/attachments.rs` writes |
| `audit_log` (`workspace_id`, `actor_id`, `action`, `target_type`, `target_id`, `metadata`) | `0001_init.sql` | `audit.rs` |
| `group_workflow_task`, `group_workflow_task_participant`, `group_workflow_event`, `channel_task_sequence` | `0024_hybrid_agent_routing.sql`, `0025_channel_kanban_board.sql` | `group_workflow_tasks.rs` |
| `sync_change`, `sync_device` | `V026__sync_change_log.sql`, `V027__sync_devices.sql` | `sync.rs` (see [sync-feed.md](sync-feed.md)) |

The legacy `message` and `receipt` tables from `0001_init.sql` exist but no `DbService` module reads or writes them; `server_seq` is `conversation_events.seq` and read state is the `conversation_member` counters plus `thread_read_receipt`.

## Data

- `domain::Message { id, workspace_id, conversation_id, sender_id, content, content_type, metadata, edited_at, edited_by, server_seq, idempotency_key, created_at }` is the API view of one `conversation_events` row: `id` is `event_id`, `server_seq` is `seq`, and `idempotency_key` is `client_msg_id` (or `turn_id` for an agent reply surfaced as a bootstrap preview).
- `domain::Conversation` carries `members: BTreeMap<String, ConversationMember>` built from active `conversation_member` rows (`removed_at IS NULL`); `conversation_type` is `direct` or `group`.
- `domain::Principal` maps `principal.type` to `PrincipalType::{Human, Agent}`; `scopes` are derived by `helpers::scopes_for_type`, not stored.
- `SendMessageRequest { actor_id, conversation_id, idempotency_key, content, content_type, metadata, trace_id }`; `metadata.reply_to_id`, `metadata.thread`, `metadata.broadcast`, and `metadata.attachment_id` are interpreted by `send_message`, and `trace_id` is copied into row metadata.
- `ConversationUnread { conversation_id, unread_count, mention_count, thread_unread_count }`; `MessagePage { messages, direction, has_more, next_cursor }`; `TimelineMessages { messages, thread_summaries }`; `ThreadDetail { root, replies }`.
- `EventEnvelope { delivery_seq, event_id, principal_id, event_type, payload, created_at }` is one `outbox_event` row for the polling and webhook consumers.
- Every `event_outbox` row written by the store has `aggregate_type = 'conversation_event'`, `aggregate_id = conversation_id`, `event_type = 'message'`, and a payload with `message_id`, `conversation_id`, `sender_id`, `content`, `content_type`, `seq`, `metadata`, `trace_id`.
- Workspace visibility is one SQL predicate repeated in the list queries: `(co.id IS NULL AND c.workspace_id = <principal workspace>) OR (co.deleted_at IS NULL AND (c.workspace_id = <principal workspace> OR com.principal_id IS NOT NULL))`, where `co` is `company` and `com` is the caller's `company_member` row; `DbService::principal_can_access_workspace` is the Rust form.

## Entry points

- The gateway constructs `application::DbService::new(event_store.clone())` in `router_with_runtime` and exposes it as `ApiState.db`; the pipeline crates construct their own `EventStore` from `common::PgConfig::from_env().to_connect_string()`.
- `DbService::send_message` (called by `POST /v1/messages`) runs: reject blank `content` or `idempotency_key`; `get_principal` and `get_conversation`; workspace check; membership check with automatic `conversation_member` upsert for agents; `EventStore::find_event_by_client_msg_id` dedup returning the existing `Message`; then one transaction that takes `pg_advisory_xact_lock(hashtext(conversation_id))`, canonicalises a thread root through `EventStore::canonicalize_thread_root_in_tx`, validates any `attachment_id`, inserts the `conversation_events` row with `seq = COALESCE(MAX(seq), 0) + 1`, inserts the `event_outbox` row, inserts one `outbox_event` `message.created` row per member and one `app_mention` row per `@mentioned` agent, and commits. After commit it calls `increment_msg_count` and `mark_conversation_viewed` for the sender.
- `ingress::ingest_message` (`POST /v2/ingest`) performs the same advisory-locked `conversation_events` + `event_outbox` insert and bumps `conversation.total_msg_count` inside the transaction.
- Reads: `list_messages` (flat, `since_seq`, `limit`), `list_timeline_messages`, `list_message_page` (keyset on `seq` with `before_seq` / `after_seq`), `get_message`, `list_thread_replies`, `list_all_messages` (export), `get_unread_counts`, `get_unread_counts_for_conversations`, `list_conversations`, `list_conversation_bootstrap_page`, `get_conversation`.
- Read state: `mark_conversation_viewed` sets `conversation_member.msg_count = conversation.total_msg_count`, `mention_count = 0`, `last_viewed_at = NOW()`; `mark_thread_viewed` upserts `thread_read_receipt`.
- Principals: `get_principal`, `list_principals_by_ids`, `authenticate_agent_secret`, `ensure_local_operator`, `find_human_by_username`, `create_human_user`, `create_agent`, `rotate_agent_secret`, `disable_principal`, `soft_delete_principal`, `list_workspace_agents`, `list_accessible_agents`, `list_agents_for_company`.
- Conversations and companies: `create_direct_conversation`, `create_group`, `update_group`, `add_group_members`, `remove_group_member`, `delete_conversation`, `pin_conversation` / `unpin_conversation`, `archive_conversation` / `unarchive_conversation`, `hide_agent_session` / `restore_hidden_agent_session`, `create_company`, `update_company`, `archive_company`, `unarchive_company`, `delete_company`, `add_company_member`, `remove_company_member`.
- Audit: `record_audit(workspace_id, actor_id, action, target_type, target_id, metadata)` inserts one `audit_log` row; `list_audit_logs(workspace_id)` returns the newest 10,000 for `GET /v1/audit-logs`.
- Events: `push_event`, `list_events`, `ack_events(principal_id, upto_seq)`, `collect_pending_webhook_deliveries`, `mark_webhook_delivered` over `outbox_event` and `event_webhook`.
- Pipeline-facing `EventStore` methods: `insert_conversation_event` (owns its transaction and lock), `insert_conversation_event_with_client` (caller holds the lock), `get_events_after_seq`, `get_event_by_message_id`, `list_messages_by_conversation`, `claim_unpublished_entries` (`FOR UPDATE SKIP LOCKED` with a lease deadline), `mark_published`, `get_outbox_entry`.

## Invariants

- `seq` is dense and monotonic per conversation: it is allocated as `COALESCE(MAX(seq), 0) + 1` while holding `pg_advisory_xact_lock(hashtext(conversation_id))`, and `(conversation_id, seq)` is the primary key. Pinned by `message_pages_cover_history_and_incremental_bursts_without_gaps` in [`services/choruz-api-gateway/src/tests/sync.rs`](../../services/choruz-api-gateway/src/tests/sync.rs).
- Idempotency is keyed on `client_msg_id` alone: the partial unique index `idx_ce_client_msg_id` makes a client key global across senders and conversations, `find_event_by_client_msg_id` returns the existing row on a retry, and a concurrent duplicate surfaces as `AppError::Conflict`. Agent replies dedup on `turn_id` through `idx_ce_turn_id`. Pinned by [`apps/web/tests/e2e/message-dedup.spec.ts`](../../apps/web/tests/e2e/message-dedup.spec.ts) and the `deduplicates optimistic messages with sync confirmations` case in [`apps/web/tests/e2e/websocket.spec.ts`](../../apps/web/tests/e2e/websocket.spec.ts).
- The `conversation_events` row, its `event_outbox` row, and the per-member `outbox_event` rows commit in one transaction; the `trg_conversation_activity` trigger updates `conversation_activity` in that same transaction for `event_type IN ('message', 'message.created', 'reply')`.
- `unread_count = GREATEST(conversation.total_msg_count - conversation_member.msg_count, 0)`; a threaded reply without `broadcast` does not bump `total_msg_count` (`ThreadFlags::bumps_conversation_unread`) and does not mark the sender as viewed. Pinned by `quiet_thread_reply_preserves_sender_unread_state`.
- `reply_event_id` on a threaded reply is always the canonical root (threads are flat); `THREAD_FLAG_SQL` is the single SQL definition of the thread discriminator. Pinned by `timeline_view_filters_quiet_replies_and_rolls_up`.
- Every list query applies the workspace predicate above, so a principal never sees a conversation, agent, or company outside its workspace or company memberships. Pinned by `company_workspace_authorization_guards_hold` and `agent_privacy_surfaces_are_scoped_to_authorized_workspace_context`.
- Principal names are unique per workspace case-insensitively among active rows (`principal_workspace_name_ci_active_key`, `V013`) and human usernames are unique (`principal_human_username_unique_idx`, `V014`); `send_message` logs `duplicate_agent_name_in_conversation` if pre-migration duplicates are members.
- `RateLimiter` is deliberately not `Clone`; `DbService` shares it through `Arc` so every clone of the service sees the same window (`check_rate_limit_*` tests in `crates/choruz-application/src/lib.rs`).
- Pins, archives, and hidden markers are per-principal rows and never affect other members (`conversation_pins_are_scoped_per_user`, `conversation_archive_is_recoverable_user_scoped_and_removes_the_users_pin`, `hide_is_user_scoped_and_removes_an_agent_session_from_normal_markers`).

## Failure modes

- Unique violation on insert: `AppError::Conflict("duplicate message: ...")` → HTTP 409; the client should re-send with the same `idempotency_key` to receive the stored row.
- Blank `content` or `idempotency_key`, or a threaded reply without `reply_to_id`: `AppError::Validation` → 400.
- Human sender that is not an active member: `Forbidden("sender is not a conversation member")` → 403; agents are auto-joined instead.
- Cross-workspace access: `Forbidden("cross-workspace access denied")` → 403.
- Attachment referenced by `metadata.attachment_id` missing → `NotFound`; reused by a principal who neither owns it nor has seen it in a member conversation → `Forbidden`.
- `increment_msg_count` and `mark_conversation_viewed` run after the commit and only log `tracing::warn!` on failure, so unread badges can lag the message by one refresh; `mention_count` bumps are best-effort.
- Pool exhaustion or a PostgreSQL error inside a query maps to `AppError::Internal` → 500 with the message prefixed by the failing operation (`send_message insert:`, `list bootstrap conversations:`, ...).
- Writers to one conversation serialise on the advisory lock, so per-conversation write throughput is bounded by transaction latency; different conversations do not contend.
- When `choruz-pipeline` is down, `event_outbox` rows accumulate with `published = FALSE`; nothing is lost and the poller drains them on restart (`claim_unpublished_entries` re-claims rows whose `claim_deadline` has passed).
- `ChatApp` state is rebuilt from PostgreSQL at boot (`build_app_from_db` in `services/choruz-api-gateway/src/main.rs`). A connection or load failure stops startup instead of serving stale or partial state.

## Tests

- [`services/choruz-api-gateway/src/tests/`](../../services/choruz-api-gateway/src/tests/): PostgreSQL-backed cases that exercise `DbService` through the router, including `message_pages_cover_history_and_incremental_bursts_without_gaps`, `quiet_thread_reply_preserves_sender_unread_state`, `timeline_view_filters_quiet_replies_and_rolls_up`, `get_message_fetches_quote_targets_with_uniform_auth`, `conversation_pin_put_is_idempotent_and_preserves_pinned_at`, `company_workspace_authorization_guards_hold`, `workflow_task_service_synchronizes_assignee_owner_and_versioned_events`, and `native_session_import_runs_end_to_end_and_is_idempotent`.
- Inline unit tests under `#[cfg(test)]` in [`crates/choruz-application/src/lib.rs`](../../crates/choruz-application/src/lib.rs), [`crates/choruz-application/src/db_service/messages.rs`](../../crates/choruz-application/src/db_service/messages.rs), [`crates/choruz-application/src/db_service/group_workflow_tasks.rs`](../../crates/choruz-application/src/db_service/group_workflow_tasks.rs), [`crates/choruz-application/src/types.rs`](../../crates/choruz-application/src/types.rs), [`crates/choruz-store/src/event_outbox.rs`](../../crates/choruz-store/src/event_outbox.rs), and [`crates/choruz-store/src/pool.rs`](../../crates/choruz-store/src/pool.rs).
- Schema: [`infra/host/migration_smoke.sh`](../../infra/host/migration_smoke.sh) and [`infra/host/migrate.sh`](../../infra/host/migrate.sh).
- Browser: [`apps/web/tests/e2e/message-dedup.spec.ts`](../../apps/web/tests/e2e/message-dedup.spec.ts), [`apps/web/tests/e2e/messaging.spec.ts`](../../apps/web/tests/e2e/messaging.spec.ts), [`apps/web/tests/e2e/message-list.spec.ts`](../../apps/web/tests/e2e/message-list.spec.ts), [`apps/web/tests/e2e/threads.spec.ts`](../../apps/web/tests/e2e/threads.spec.ts).

## Related

- [api-gateway.md](api-gateway.md) for the handlers that call `DbService`.
- [sync-feed.md](sync-feed.md) for the `sync_change` triggers fired by these tables.
- [message-pipeline.md](message-pipeline.md) for the `event_outbox` consumer and the writer that inserts `reply` events.
- [threads.md](threads.md) and [channel-tasks.md](channel-tasks.md) for the thread and board tables.
- [`docs/data-model.md`](../data-model.md) for column-level descriptions.
