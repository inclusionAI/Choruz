-- Durable step checkpoints make retried agent provisioning requests resume
-- instead of creating duplicate principals, conversations, or bindings.

CREATE TABLE IF NOT EXISTS agent_provisioning_checkpoint (
  actor_id TEXT NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
  idempotency_key TEXT NOT NULL,
  request_fingerprint TEXT NOT NULL,
  step_results_json JSONB NOT NULL DEFAULT '{}'::jsonb,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (actor_id, idempotency_key)
);

CREATE INDEX IF NOT EXISTS agent_provisioning_checkpoint_updated_idx
  ON agent_provisioning_checkpoint (updated_at DESC);
