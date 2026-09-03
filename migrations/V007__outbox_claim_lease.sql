-- Add lease-based claim columns to event_outbox
ALTER TABLE event_outbox
ADD COLUMN IF NOT EXISTS claimed_by TEXT,
ADD COLUMN IF NOT EXISTS claimed_at TIMESTAMPTZ,
ADD COLUMN IF NOT EXISTS claim_deadline TIMESTAMPTZ,
ADD COLUMN IF NOT EXISTS attempt_count INT NOT NULL DEFAULT 0;

-- Index for efficient claiming of available or expired leases
-- Note: cannot use NOW() in partial index predicate because it's not IMMUTABLE.
-- Filtering by published=FALSE is still a huge win for performance.
CREATE INDEX IF NOT EXISTS idx_event_outbox_unpublished
ON event_outbox (id)
WHERE published = FALSE;
