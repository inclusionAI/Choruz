CREATE TABLE experience_task_profile (
    binding_id TEXT PRIMARY KEY REFERENCES experience_policy(binding_id) ON DELETE CASCADE,
    enabled BOOLEAN NOT NULL DEFAULT FALSE,
    shared_task_history BOOLEAN NOT NULL DEFAULT FALSE,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE experience_task_link (
    binding_id TEXT PRIMARY KEY REFERENCES agent_runtime_bindings(id) ON DELETE CASCADE,
    profile_binding_id TEXT NOT NULL REFERENCES experience_task_profile(binding_id) ON DELETE CASCADE,
    CHECK (binding_id <> profile_binding_id)
);
CREATE INDEX experience_task_link_profile ON experience_task_link(profile_binding_id);

-- Shared tasks reference the existing policy; they never copy its evolving state.
CREATE FUNCTION effective_experience_binding(workspace TEXT, requested TEXT, owner TEXT DEFAULT NULL)
RETURNS TEXT LANGUAGE SQL STABLE AS $$
    SELECT COALESCE((
        SELECT p.binding_id FROM experience_task_link link
        JOIN experience_policy p ON p.binding_id=link.profile_binding_id
        JOIN agent_runtime_bindings source ON source.id=p.binding_id
        JOIN agent_runtime_bindings target ON target.id=link.binding_id
        JOIN conversation source_conversation ON source_conversation.id=source.conversation_id
        JOIN conversation c ON c.id=target.conversation_id
        WHERE link.binding_id=requested AND p.workspace_id=workspace AND c.workspace_id=workspace AND source_conversation.workspace_id=workspace
          AND (owner IS NULL OR p.owner_id=owner)
          AND source.state<>'disabled' AND target.state<>'disabled'
          AND source.driver_type=target.driver_type AND source.workspace_path=target.workspace_path
          AND COALESCE(source.config_json->'runtime_host_id','null'::jsonb)=COALESCE(target.config_json->'runtime_host_id','null'::jsonb)
          AND COALESCE(source.config_json->'harness_account_id','null'::jsonb)=COALESCE(target.config_json->'harness_account_id','null'::jsonb)
          AND NULLIF(BTRIM(source.config_json->>'model'),'') IS NOT DISTINCT FROM NULLIF(BTRIM(target.config_json->>'model'),'')
    ),requested)
$$;
