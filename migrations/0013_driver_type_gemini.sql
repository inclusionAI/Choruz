-- Allow 'gemini_terminal' as a valid driver_type on agent_runtime_bindings.
-- The original check constraint only allowed claude_print, claude_terminal,
-- codex_exec, codex_app_server, and acp. Adding gemini_terminal to the
-- allowlist so agents can be created with the Gemini CLI driver.

ALTER TABLE agent_runtime_bindings
    DROP CONSTRAINT IF EXISTS agent_runtime_bindings_driver_type_check;

ALTER TABLE agent_runtime_bindings
    ADD CONSTRAINT agent_runtime_bindings_driver_type_check
    CHECK (driver_type = ANY (ARRAY[
        'claude_print'::text,
        'claude_terminal'::text,
        'codex_exec'::text,
        'codex_app_server'::text,
        'codex_terminal'::text,
        'gemini_terminal'::text,
        'acp'::text
    ]));
