-- Company slugs are user-facing identifiers within an account, not global
-- installation identifiers. Multiple local accounts may therefore use the
-- same slug while each owner's active companies remain unambiguous.
DROP INDEX IF EXISTS company_slug_active_key;

CREATE UNIQUE INDEX IF NOT EXISTS company_owner_slug_active_key
    ON company (owner_id, slug)
    WHERE deleted_at IS NULL;
