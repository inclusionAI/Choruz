-- Gemini CLI is no longer supported. Enforce the replacement allowlist for
-- new and updated rows without scanning the existing bindings under an
-- ACCESS EXCLUSIVE lock, then remove legacy Gemini runtime bindings. Agent
-- principals and conversation history remain intact.
ALTER TABLE agent_runtime_bindings
    ADD CONSTRAINT agent_runtime_bindings_driver_type_without_gemini_check
    CHECK (driver_type = ANY (ARRAY[
        'claude_print'::text,
        'claude_terminal'::text,
        'codex_exec'::text,
        'codex_app_server'::text,
        'codex_terminal'::text,
        'acp'::text,
        'webhook_agent'::text
    ])) NOT VALID;

DELETE FROM agent_runtime_bindings
WHERE driver_type = 'gemini_terminal';
