# Agent Note: Share message publication and workspace access

Status: implemented

## Problem

Message producers maintained separate unread and delivery writes. Quiet Agent thread replies could commit without a message sync event, while human messages could commit before a failed unread update. Routing and webhook delivery also interpreted mention casing differently. Cron and runtime access accepted a native workspace even when its company was deleted.

## Decision

[`publish_conversation_message`](../../../../migrations/V069__canonical_message_publication.sql) owns publication for canonical message events. Its event transaction updates timeline unread state and sender read state, then emits one durable message delivery per active member. Quiet thread replies emit delivery without clearing timeline read state. The router queue, attempt fences and cron occurrence deduplication retain their distinct owners. The reply-only sync projection and producer-specific message fanout/count updates are removed.

[`choruz-common::mentions`](../../../../crates/choruz-common/src/mentions.rs) owns case-insensitive mention token boundaries and longest-name matching. Routing policy and webhook subscriptions remain distinct consumers.

`DbService::accessible_workspace_ids` uses the existing workspace access predicate, excludes deleted companies and propagates lookup failures. Gateway runtime and cron authorization delegate to it; a native workspace without a company row remains valid.

## Alternatives considered

**A shared Rust INSERT helper alone.** Direct SQL producers and fenced session commits would still bypass projection policy. The event trigger covers those entries without moving their transaction or authorization fences.

**Bootstrap refresh as message delivery.** A snapshot carries only a bounded last-message preview and cannot represent consecutive outputs or quiet thread replies reliably.

**Merge every routing and access rule.** Provider triggers, webhook subscriptions and conversation membership express different contracts. Only their common lexical and workspace-access primitives are shared.

## Consequences

Unread or member-delivery failure aborts publication rather than silently accepting partial state. Every message event, including direct SQL imports and synthetic cron announcements, uses the same delivery envelope. Applied migrations remain frozen. Tests exercise quiet and consecutive Agent messages, HTTP transaction rollback, mixed-case mentions and deleted-company authorization; closing a test-owned pool verifies that lookup failure is not an empty success.
