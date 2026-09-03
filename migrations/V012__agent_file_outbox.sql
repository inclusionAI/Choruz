-- Agent file outbox: stores outbox commands detected from PTY agent workspaces.
-- The pipeline file watcher inserts rows here; the outbox processor consumes them.

BEGIN;

CREATE TABLE IF NOT EXISTS agent_file_outbox (
    id          TEXT PRIMARY KEY,
    agent_id    TEXT NOT NULL,
    workspace_path TEXT NOT NULL,
    command_json JSONB NOT NULL,
    status      TEXT NOT NULL DEFAULT 'pending',   -- pending | processing | done | failed
    error_msg   TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    processed_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_file_outbox_pending
    ON agent_file_outbox (created_at) WHERE status = 'pending';

-- Notify the pipeline when a new outbox command is inserted
CREATE OR REPLACE FUNCTION notify_file_outbox() RETURNS trigger AS $$
BEGIN
    PERFORM pg_notify('echat_file_outbox', NEW.id);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_file_outbox_notify ON agent_file_outbox;
CREATE TRIGGER trg_file_outbox_notify
    AFTER INSERT ON agent_file_outbox
    FOR EACH ROW EXECUTE FUNCTION notify_file_outbox();

COMMIT;
