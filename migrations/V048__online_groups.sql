ALTER TABLE principal ADD COLUMN online_guest BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE principal ADD CONSTRAINT online_guest_has_no_login
    CHECK (NOT online_guest OR (type = 'human' AND secret_hash IS NULL));
DROP INDEX principal_workspace_name_ci_active_key;
CREATE UNIQUE INDEX principal_workspace_name_ci_active_key
    ON principal (workspace_id, lower(name)) WHERE deleted_at IS NULL AND NOT online_guest;
DROP INDEX principal_human_username_unique_idx;
CREATE UNIQUE INDEX principal_human_username_unique_idx
    ON principal (lower(name)) WHERE type = 'human' AND deleted_at IS NULL AND NOT online_guest;

CREATE TABLE online_group_link (
    id TEXT PRIMARY KEY,
    channel_id TEXT NOT NULL,
    principal_id TEXT NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    peer_account_id TEXT,
    role TEXT NOT NULL CHECK (role IN ('host', 'guest')),
    conversation_id TEXT NOT NULL,
    peer_principal_id TEXT,
    encryption_key TEXT NOT NULL,
    name TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'active', 'revoked')),
    last_seq BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (principal_id, channel_id)
);
CREATE INDEX online_group_link_actor_idx ON online_group_link(principal_id, workspace_id, account_id);

CREATE TABLE online_group_outbox (
    id TEXT PRIMARY KEY,
    link_id TEXT NOT NULL REFERENCES online_group_link(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL,
    body JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX online_group_outbox_link_idx ON online_group_outbox(link_id, created_at);

CREATE TABLE online_group_inbox (
    id TEXT PRIMARY KEY,
    link_id TEXT NOT NULL REFERENCES online_group_link(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL,
    body JSONB NOT NULL,
    processed BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX online_group_inbox_pending_idx ON online_group_inbox(link_id, created_at) WHERE NOT processed;

CREATE TABLE online_group_message (
    link_id TEXT NOT NULL REFERENCES online_group_link(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    server_seq BIGINT NOT NULL,
    body JSONB NOT NULL,
    PRIMARY KEY (link_id, event_id),
    UNIQUE (link_id, server_seq)
);
