-- Support fair dispatch ordering across agent queues without scanning
-- terminal command history. The partial index matches the pending-only
-- window used by PgSessionStore::find_pending_commands.

CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_agent_commands_pending_fair
  ON agent_commands (agent_id, created_at, command_id)
  WHERE status = 'pending';
