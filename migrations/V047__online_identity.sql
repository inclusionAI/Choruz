CREATE TABLE online_identity (
    principal_id TEXT PRIMARY KEY REFERENCES principal(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    device_id TEXT NOT NULL UNIQUE,
    service_url TEXT NOT NULL,
    session_token TEXT NOT NULL,
    display_name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX online_identity_workspace_idx ON online_identity(workspace_id);
