-- V016: Switch the dispatch coalescer's partial index from per-(agent, conv)
-- to per-agent.
--
-- V015 created `idx_agent_commands_active_pair (agent_id, conversation_id,
-- status) WHERE status IN (...)` for an earlier version of
-- `find_pending_commands` that coalesced spawns per (agent_id,
-- conversation_id) pair. We've since narrowed the coalescer to per-agent —
-- matching `agent_runtime_bindings`' workspace-scoped `external_session_id`
-- (one session per agent, migration 0018) so concurrent spawns can't race
-- the same session and corrupt conversation history with last-write-wins.
--
-- The new index drops `conversation_id` from the key. The old index would
-- still satisfy the new NOT EXISTS query, but a narrower index is smaller
-- and faster to scan.
--
-- Idempotent with IF NOT EXISTS / IF EXISTS so re-running is safe.
-- CONCURRENTLY so production builds don't block dispatch writes.

CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_agent_commands_active_per_agent
  ON agent_commands (agent_id, status)
  WHERE status IN ('leased', 'started', 'heartbeating', 'retry_scheduled');

DROP INDEX CONCURRENTLY IF EXISTS idx_agent_commands_active_pair;
