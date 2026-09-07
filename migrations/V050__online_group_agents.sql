ALTER TABLE principal DROP CONSTRAINT online_guest_has_no_login;
ALTER TABLE principal ADD CONSTRAINT online_guest_has_no_login
    CHECK (NOT online_guest OR secret_hash IS NULL);

CREATE TABLE online_group_agent (
    link_id TEXT NOT NULL REFERENCES online_group_link(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL,
    agent_id TEXT NOT NULL REFERENCES principal(id),
    remote_id TEXT NOT NULL,
    context JSONB NOT NULL DEFAULT '{}',
    status TEXT NOT NULL CHECK (status IN ('pending','active','removed','error')),
    error TEXT,
    generation BIGINT NOT NULL DEFAULT 1,
    start_seq BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (link_id, agent_id),
    UNIQUE (link_id, remote_id)
);

ALTER TABLE online_group_link ADD COLUMN agent_in_seq BIGINT NOT NULL DEFAULT 0;
ALTER TABLE online_group_link ADD COLUMN agent_out_seq BIGINT NOT NULL DEFAULT 0;
