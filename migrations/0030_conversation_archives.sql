-- 0030: Persist recoverable, user-scoped conversation archives.

CREATE TABLE IF NOT EXISTS conversation_archive (
  principal_id TEXT NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
  conv_id TEXT NOT NULL REFERENCES conversation(id) ON DELETE CASCADE,
  archived_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (principal_id, conv_id)
);

CREATE INDEX IF NOT EXISTS idx_conversation_archive_principal_archived_at
  ON conversation_archive (principal_id, archived_at DESC);
