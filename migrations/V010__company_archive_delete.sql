-- Add archive and soft-delete support to company table.
-- archived_at: non-null means company is read-only (greyed out in UI).
-- deleted_at: non-null means company is soft-deleted (hidden from list, 30-day recovery).

ALTER TABLE company ADD COLUMN IF NOT EXISTS archived_at TIMESTAMPTZ;
ALTER TABLE company ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;
