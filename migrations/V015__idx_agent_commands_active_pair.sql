-- V015: Partial index supporting the dispatch coalescer.
--
-- Naming note: V015 so it sorts after V001 (which creates agent_commands).
-- An earlier 0023_* version of this file failed on fresh DBs because 00##
-- prefixes sort lexically before V###, so the index ran before its table.
--
-- Background: `find_pending_commands` was changed (in echat-session/src/store.rs)
-- to skip pending commands whose (agent_id, conversation_id) pair already has
-- a command in flight or queued for retry. Without this index the NOT EXISTS
-- subquery in that query would seq-scan agent_commands on every dispatch tick
-- (every ~1s), since the table grows monotonically.
--
-- Partial index design: only index rows in non-terminal-non-pending states.
-- That keeps the index tiny (typically ≤ #(active agents) × #(active convs))
-- regardless of how many lifetime rows agent_commands has, so the lookup is
-- effectively O(1).
--
-- CONCURRENTLY so production index build doesn't block the dispatch loop's
-- writes. The migration runner uses tokio_postgres `batch_execute` (simple
-- query protocol) on a single-statement file, no implicit transaction wraps
-- it, so CONCURRENTLY is allowed.

CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_agent_commands_active_pair
  ON agent_commands (agent_id, conversation_id, status)
  WHERE status IN ('leased', 'started', 'heartbeating', 'retry_scheduled');
