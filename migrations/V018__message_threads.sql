-- V018: Message threads (RFC: docs/rfc/message-threads.md).
--
-- Threads reuse the existing conversation_events.reply_event_id column:
-- a threaded reply is a normal message row with reply_event_id pointing
-- at the THREAD ROOT (canonicalized — never at another reply) and
-- metadata.thread = true (JSON boolean). Legacy quote-replies keep
-- reply_event_id with no thread flag and are unaffected.
--
-- V-series (not 0NNN): the index below targets conversation_events,
-- which V001 creates; migrations apply in lexicographic order and the
-- 0NNN series runs before every V-file.
--
-- This migration adds only what the read paths need: a partial index
-- for thread reply lookups / rollups, and a lazy per-principal receipt
-- table for thread-level unread.

BEGIN;

-- Backs `list_thread_replies` (replies of one root, ordered), the
-- per-page rollup GROUP BY in the timeline view, and the per-poll
-- thread-unread LATERAL in get_unread_counts. Partial on BOTH the
-- reply pointer and the thread flag: every consumer of this index also
-- filters on the thread discriminator (THREAD_FLAG_SQL in echat-store —
-- the predicate below must stay textually identical to it), so legacy
-- quote-replies and non-reply rows never enter the index. This keeps
-- the unread LATERAL's full-thread walk proportional to genuine thread
-- traffic instead of all replies ever written.
CREATE INDEX IF NOT EXISTS idx_conversation_events_thread
  ON conversation_events (conversation_id, reply_event_id, seq)
  WHERE reply_event_id IS NOT NULL
    AND COALESCE(metadata->'thread' = 'true'::jsonb, false);

-- Thread read receipts (pattern: receipt table from 0001 + unread
-- counters from 0016). Rows are created lazily on first view of a
-- thread — never pre-populated per member, so the table stays
-- proportional to actual thread engagement.
--
-- All three foreign keys cascade so receipts cannot outlive their
-- conversation, their thread root, or their principal (mirrors 0001's
-- receipt table, which cascades on conversation delete).
-- conversation_events.event_id carries a UNIQUE constraint (V001), so
-- the thread_root_id FK is valid.
CREATE TABLE IF NOT EXISTS thread_read_receipt (
  conversation_id TEXT NOT NULL REFERENCES conversation(id) ON DELETE CASCADE,
  thread_root_id  TEXT NOT NULL REFERENCES conversation_events(event_id) ON DELETE CASCADE,
  principal_id    TEXT NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
  last_read_seq   BIGINT NOT NULL DEFAULT 0,
  last_read_at    TIMESTAMPTZ,
  PRIMARY KEY (conversation_id, thread_root_id, principal_id)
);

-- Backs "all my thread unreads in this conversation" aggregation.
CREATE INDEX IF NOT EXISTS idx_thread_read_receipt_principal
  ON thread_read_receipt (principal_id, conversation_id);

-- Backs the thread_root_id ON DELETE CASCADE: conversation deletion
-- removes conversation_events rows BEFORE the conversation row
-- (see delete_conversation), so the per-event referential-integrity
-- cascade fires while receipts still exist — without this index each
-- deleted event would sequentially scan the whole cross-conversation
-- receipt table. The PK leads with conversation_id and the index above
-- leads with principal_id; neither serves a thread_root_id probe.
CREATE INDEX IF NOT EXISTS idx_thread_read_receipt_root
  ON thread_read_receipt (thread_root_id);

COMMIT;
