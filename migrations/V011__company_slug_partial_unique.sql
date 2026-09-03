-- Replace the absolute UNIQUE constraint on company.slug with a partial unique
-- index that only covers non-deleted rows. This allows re-using a slug after
-- soft-deleting the original company.

ALTER TABLE company DROP CONSTRAINT IF EXISTS company_slug_key;
CREATE UNIQUE INDEX IF NOT EXISTS company_slug_active_key ON company (slug) WHERE deleted_at IS NULL;
