-- Hide is a recoverable, per-user view preference for direct Agent sessions.
-- It does not stop the Agent or alter the shared conversation.

CREATE TABLE IF NOT EXISTS conversation_hidden (
  principal_id TEXT NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
  conv_id TEXT NOT NULL REFERENCES conversation(id) ON DELETE CASCADE,
  hidden_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (principal_id, conv_id)
);

CREATE INDEX IF NOT EXISTS idx_conversation_hidden_principal_hidden_at
  ON conversation_hidden (principal_id, hidden_at DESC);

DROP TRIGGER IF EXISTS trg_conversation_hidden_sync_change ON conversation_hidden;
CREATE TRIGGER trg_conversation_hidden_sync_change
  AFTER INSERT OR DELETE ON conversation_hidden
  FOR EACH ROW EXECUTE FUNCTION emit_sidebar_marker_sync_change('hidden');
