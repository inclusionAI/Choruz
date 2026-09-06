-- Short-lived RPC jobs let the Company control plane ask a connected runtime
-- host to inspect its own filesystem and native session catalog. Results are
-- bounded, expire quickly, and never contain Harness credentials.

BEGIN;

CREATE TABLE runtime_host_operation (
    id             TEXT        PRIMARY KEY,
    runtime_host_id TEXT       NOT NULL REFERENCES runtime_host(id) ON DELETE CASCADE,
    company_id     TEXT        NOT NULL REFERENCES company(id) ON DELETE CASCADE,
    kind           TEXT        NOT NULL CHECK (kind IN ('filesystem.home', 'filesystem.list', 'workspace_sessions.scan')),
    request_json   JSONB       NOT NULL DEFAULT '{}'::jsonb,
    response_json  JSONB,
    error          TEXT,
    status         TEXT        NOT NULL DEFAULT 'pending'
                               CHECK (status IN ('pending', 'leased', 'completed', 'failed')),
    lease_expires_at TIMESTAMPTZ,
    expires_at     TIMESTAMPTZ NOT NULL,
    created_by     TEXT        NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX runtime_host_operation_claim_idx
    ON runtime_host_operation (runtime_host_id, created_at)
    WHERE status IN ('pending', 'leased');

CREATE INDEX runtime_host_operation_expiry_idx
    ON runtime_host_operation (expires_at);

ALTER TABLE native_session_import
    ADD COLUMN runtime_host_id TEXT REFERENCES runtime_host(id) ON DELETE SET NULL;

DO $$
DECLARE
    legacy_constraint TEXT;
BEGIN
    SELECT conname INTO legacy_constraint
    FROM pg_constraint
    WHERE conrelid = 'native_session_import'::regclass
      AND contype = 'u'
      AND pg_get_constraintdef(oid) = 'UNIQUE (workspace_path, driver_type, native_session_id)';
    IF legacy_constraint IS NOT NULL THEN
        EXECUTE format(
            'ALTER TABLE native_session_import DROP CONSTRAINT %I',
            legacy_constraint
        );
    END IF;
END
$$;

ALTER TABLE native_session_import
    ADD CONSTRAINT native_session_import_device_session_key
    UNIQUE NULLS NOT DISTINCT (runtime_host_id, workspace_path, driver_type, native_session_id);

COMMIT;
