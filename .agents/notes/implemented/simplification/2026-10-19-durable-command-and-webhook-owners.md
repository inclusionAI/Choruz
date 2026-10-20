# Agent Note: Commit mutations and deliver events through their database owners

Status: implemented

## Problem

Workspace migration and webhook configuration mutate a process-local `ChatApp` before best-effort persistence. A storage failure can return success and leave memory ahead of PostgreSQL. Message publication also announces the persisted event to an independent memory queue. Webhook delivery merges that queue with database events by sequence even though the queues allocate different sequence spaces and payloads.

## Decision

`DbService::migrate_principal_workspace`, `migrate_conversation_workspace` and `set_event_webhook` authorize against current database identities and workspace access. Each command commits its row and audit in one transaction. The gateway updates an identity projection only after a successful commit; projections do not authorize these commands. Conversation migration moves only its conversation row, without rewriting historical event rows.

`services/choruz-api-gateway/src/webhook.rs` reads only canonical `outbox_event` rows and `event_webhook` configuration. Publication from HTTP and Online shares that durable path without a second announcement. Receiver updates preserve the delivery cursor. Polling acknowledgement does not acknowledge a pending webhook. A failed delivery blocks later events for that principal; other principals can proceed. Cursor persistence errors fail the flush. Retrying after an uncertain receiver acknowledgement can redeliver the same event ID, so receivers retain their idempotency responsibility.

The gateway does not load webhook configuration or align a memory event counter at startup. Its metrics read durable counts rather than inferring them from incomplete identity projections. The standalone in-memory `ChatApp` API is not a second gateway delivery source.

The [PostgreSQL durability decision](2026-09-03-remove-app-snapshot.md) remains active: it owns startup failure and backup boundaries. The [bridge retry decision](../bug-fix/2026-09-07-partial-webhook-delivery-retries.md) remains active: it owns receiver-side partial fan-out, not gateway queue selection.

## Alternatives considered

**Propagate errors from the projection helper.** This still mutates memory before the database rejects the write and cannot atomically commit the audit. Commands belong in the transactional application service.

**Reconcile two event queues with a larger deduplication key.** This retains two payload and cursor authorities for the same persisted message. The database already stores the event required by the receiver.

## Consequences

Storage errors are visible to callers instead of successful responses that disappear after restart. The gateway removes its projection persistence helper and memory webhook merge. Database-backed regression scenarios inject storage rejection, verify rollback, retry through fresh process state and check canonical content and cursor recovery. External HTTP acknowledgement and local cursor commit are not a distributed transaction; exactly-once receiver effects still require deduplication by event ID.
