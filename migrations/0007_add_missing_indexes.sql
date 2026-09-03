-- 0007: Add missing indexes for common query patterns.
-- All statements are idempotent (IF NOT EXISTS).

CREATE INDEX IF NOT EXISTS idx_audit_log_workspace_created
  ON audit_log (workspace_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_outbox_event_principal_unacked
  ON outbox_event (principal_id, created_at)
  WHERE acknowledged_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_lease_until
  ON agent_turn_leases (lease_until);
