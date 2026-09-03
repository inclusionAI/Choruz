-- The product has one active human and any number of agents. Select the
-- canonical installation user before converting legacy admin principals.
BEGIN;

CREATE TEMP TABLE _choruz_canonical_human ON COMMIT DROP AS
SELECT id
FROM principal
WHERE type IN ('human', 'admin')
  AND disabled = FALSE
  AND deleted_at IS NULL
ORDER BY CASE WHEN type = 'human' THEN 0 ELSE 1 END, created_at ASC, id ASC
LIMIT 1;

UPDATE principal SET type = 'human' WHERE type = 'admin';

-- All companies and registered conversations belong to the installation
-- user. Preserve the relationship as real membership rows instead of relying
-- on a principal-type authorization bypass.
UPDATE company
SET owner_id = canonical.id,
    updated_at = NOW()
FROM _choruz_canonical_human canonical
WHERE company.owner_id <> canonical.id;

INSERT INTO company_member (company_id, principal_id, role, joined_at)
SELECT company.id, canonical.id, 'owner', NOW()
FROM company
CROSS JOIN _choruz_canonical_human canonical
ON CONFLICT (company_id, principal_id) DO UPDATE
SET role = 'owner';

INSERT INTO conversation_member (conv_id, principal_id, role, joined_at)
SELECT conversation.id, canonical.id, 'member', NOW()
FROM conversation
CROSS JOIN _choruz_canonical_human canonical
ON CONFLICT (conv_id, principal_id) DO UPDATE
SET role = 'member', removed_at = NULL;

-- Old local builds could create a bootstrap admin alongside a signed-up
-- human. Keep those rows for referential history, but only the canonical user
-- remains active after the single-user migration.
UPDATE principal
SET deleted_at = COALESCE(deleted_at, NOW()),
    disabled = TRUE,
    updated_at = NOW()
WHERE type = 'human'
  AND deleted_at IS NULL
  AND id <> (SELECT id FROM _choruz_canonical_human);

ALTER TABLE principal DROP CONSTRAINT principal_type_check;
ALTER TABLE principal
    ADD CONSTRAINT principal_type_check CHECK (type IN ('human', 'agent'));

ALTER TABLE conversation_member DROP COLUMN role;
ALTER TABLE company_member DROP COLUMN role;

COMMIT;
