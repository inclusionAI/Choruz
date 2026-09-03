-- These indexes target existing hot tables and must be built without blocking
-- command or binding writes. PostgreSQL forbids CONCURRENTLY inside a
-- transaction, so they intentionally live outside V028's atomic schema change.

CREATE INDEX CONCURRENTLY IF NOT EXISTS agent_runtime_bindings_runtime_host_idx
    ON agent_runtime_bindings ((config_json->>'runtime_host_id'))
    WHERE config_json ? 'runtime_host_id' AND state != 'disabled';

CREATE INDEX CONCURRENTLY IF NOT EXISTS agent_commands_runtime_host_queue_idx
    ON agent_commands ((metadata->>'runtime_host_id'), status, created_at, command_id)
    WHERE metadata ? 'runtime_host_id'
      AND status IN ('pending', 'retry_scheduled', 'leased', 'started', 'heartbeating');
