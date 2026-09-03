-- Add a monotonically increasing delivery_seq to outbox_event
-- so that consumers can use numeric cursors for event polling.
ALTER TABLE outbox_event ADD COLUMN IF NOT EXISTS delivery_seq BIGSERIAL;

-- Index for efficient list_events: unacknowledged events per principal
-- ordered by delivery_seq.  The partial index from 0007 covers
-- (principal_id, created_at) but we now order by delivery_seq.
CREATE INDEX IF NOT EXISTS idx_outbox_event_principal_seq
  ON outbox_event (principal_id, delivery_seq)
  WHERE acknowledged_at IS NULL;
