-- Runtime hosts let one Company place agent runtimes on multiple Choruz
-- installations without federating their databases. The Company remains the
-- control plane; each host authenticates with a revocable, hashed bearer token.

BEGIN;

CREATE TABLE runtime_host (
    id                 TEXT        PRIMARY KEY,
    company_id         TEXT        NOT NULL REFERENCES company(id) ON DELETE CASCADE,
    name               TEXT        NOT NULL,
    token_hash         TEXT        NOT NULL UNIQUE,
    status             TEXT        NOT NULL DEFAULT 'offline'
                                  CHECK (status IN ('online', 'offline', 'revoked')),
    last_seen_at       TIMESTAMPTZ,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at         TIMESTAMPTZ
);

CREATE UNIQUE INDEX runtime_host_active_name_idx
    ON runtime_host (company_id, lower(name))
    WHERE revoked_at IS NULL;

CREATE INDEX runtime_host_company_idx
    ON runtime_host (company_id, created_at);

CREATE TABLE runtime_host_pairing (
    id                 TEXT        PRIMARY KEY,
    company_id         TEXT        NOT NULL REFERENCES company(id) ON DELETE CASCADE,
    code_hash          TEXT        NOT NULL UNIQUE,
    created_by         TEXT        NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
    expires_at         TIMESTAMPTZ NOT NULL,
    consumed_at        TIMESTAMPTZ,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX runtime_host_pairing_expiry_idx
    ON runtime_host_pairing (expires_at)
    WHERE consumed_at IS NULL;

COMMIT;
