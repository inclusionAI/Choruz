CREATE TABLE harness_account_login (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES harness_account(id) ON DELETE CASCADE,
    company_id TEXT NOT NULL REFERENCES company(id) ON DELETE CASCADE,
    runtime_host_id TEXT NOT NULL REFERENCES runtime_host(id) ON DELETE CASCADE,
    driver_type TEXT NOT NULL CHECK (driver_type IN ('claude_terminal', 'codex_terminal')),
    state TEXT NOT NULL DEFAULT 'queued'
        CHECK (state IN ('queued', 'awaiting_browser', 'authorizing', 'verified', 'failed', 'cancelled', 'expired')),
    authorization_url TEXT,
    user_code TEXT,
    callback_code TEXT,
    error TEXT,
    created_by TEXT NOT NULL REFERENCES principal(id),
    claimed_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ NOT NULL,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX harness_account_login_open_account_idx
    ON harness_account_login (account_id)
    WHERE state IN ('queued', 'awaiting_browser', 'authorizing');

CREATE INDEX harness_account_login_host_claim_idx
    ON harness_account_login (runtime_host_id, created_at)
    WHERE state = 'queued';

COMMENT ON TABLE harness_account_login IS
    'Short-lived remote Harness login handoffs. OAuth tokens and PKCE verifiers never enter this table; they remain in the runtime host process/profile.';
