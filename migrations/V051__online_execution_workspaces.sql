CREATE TABLE online_execution_workspace (
    link_id TEXT NOT NULL REFERENCES online_group_link(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL UNIQUE REFERENCES conversation(id),
    input_seq BIGINT NOT NULL DEFAULT 0,
    output_seq BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (link_id, workspace_id)
);

INSERT INTO online_execution_workspace(link_id, workspace_id, conversation_id, input_seq, output_seq)
SELECT l.id, c.workspace_id, c.id, l.agent_in_seq, l.agent_out_seq
FROM online_group_link l JOIN conversation c ON c.id = 'online-group:' || l.id
WHERE l.role = 'guest';

ALTER TABLE online_group_link DROP COLUMN agent_in_seq;
ALTER TABLE online_group_link DROP COLUMN agent_out_seq;
