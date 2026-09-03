-- 0023: Persist user-scoped conversation pins.

CREATE TABLE IF NOT EXISTS conversation_pin (
  principal_id TEXT NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
  conv_id TEXT NOT NULL REFERENCES conversation(id) ON DELETE CASCADE,
  pinned_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (principal_id, conv_id)
);

CREATE INDEX IF NOT EXISTS idx_conversation_pin_principal_pinned_at
  ON conversation_pin (principal_id, pinned_at DESC);
