-- 0022: Index conversation_member by principal_id for active rows.
--
-- The existing partial unique index `conversation_member_active_idx` is
-- (conv_id, principal_id) WHERE removed_at IS NULL — useful when the
-- query has conv_id as the leading filter, but Postgres cannot use it
-- for principal_id-only lookups (b-tree scans only the leading column).
--
-- Hot caller: pg_event_source.rs fanout query
--   SELECT conv_id FROM conversation_member
--   WHERE principal_id = $1 AND removed_at IS NULL
-- runs once per delivered event; without this index it falls back to a
-- seq scan on the conversation_member table.
--
-- CONCURRENTLY so production index build does not lock the table.
-- The migration runner uses tokio_postgres `batch_execute`, which sends
-- this single statement via the simple query protocol — no implicit
-- transaction wraps it, so CONCURRENTLY is allowed.

CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_conversation_member_principal_active
  ON conversation_member (principal_id)
  WHERE removed_at IS NULL;
