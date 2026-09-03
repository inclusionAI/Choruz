-- Per-device durable acknowledgements for the dashboard sync stream.
CREATE TABLE IF NOT EXISTS sync_device (
    principal_id TEXT NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
    device_id TEXT NOT NULL,
    ack_cursor BIGINT NOT NULL DEFAULT 0 CHECK (ack_cursor >= 0),
    PRIMARY KEY (principal_id, device_id),
    CHECK (char_length(device_id) BETWEEN 1 AND 128),
    CHECK (device_id !~ '[[:cntrl:]]')
);

-- Notifications are wakeups only. The durable sync_change table remains the
-- source of truth, so missed/coalesced notifications never lose data.
CREATE OR REPLACE FUNCTION notify_sync_change()
RETURNS TRIGGER AS $$
BEGIN
    PERFORM pg_notify('choruz_sync_change', NEW.principal_id);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_sync_change_notify ON sync_change;
CREATE TRIGGER trg_sync_change_notify
    AFTER INSERT ON sync_change
    FOR EACH ROW EXECUTE FUNCTION notify_sync_change();
