CREATE INDEX IF NOT EXISTS idx_agent_commands_interaction
    ON agent_commands (conversation_id, message_id, created_at, command_id);
