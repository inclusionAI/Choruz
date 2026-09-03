-- 0031: Remote-control pairing, preferences, and revocable device capabilities.

CREATE TABLE IF NOT EXISTS remote_control_pairing (
  id TEXT PRIMARY KEY,
  principal_id TEXT NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
  code_hash TEXT NOT NULL UNIQUE,
  expires_at TIMESTAMPTZ NOT NULL,
  consumed_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_remote_control_pairing_expiry
  ON remote_control_pairing (expires_at)
  WHERE consumed_at IS NULL;

CREATE TABLE IF NOT EXISTS remote_control_device (
  id TEXT PRIMARY KEY,
  principal_id TEXT NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  session_key_wrapped TEXT NOT NULL,
  paired_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  last_seen_at TIMESTAMPTZ,
  revoked_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_remote_control_device_principal
  ON remote_control_device (principal_id, paired_at DESC)
  WHERE revoked_at IS NULL;
