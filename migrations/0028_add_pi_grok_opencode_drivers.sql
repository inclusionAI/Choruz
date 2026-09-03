-- Install the expanded allowlist without removing the currently enforced
-- constraint. PostgreSQL checks new rows against this NOT VALID constraint
-- immediately, while 0029 validates existing rows before the atomic swap.
ALTER TABLE agent_runtime_bindings
    ADD CONSTRAINT agent_runtime_bindings_driver_type_check_v2
    CHECK (driver_type = ANY (ARRAY[
        'claude_print'::text,
        'claude_terminal'::text,
        'codex_exec'::text,
        'codex_app_server'::text,
        'codex_terminal'::text,
        'pi_terminal'::text,
        'grok_terminal'::text,
        'opencode_terminal'::text,
        'acp'::text,
        'webhook_agent'::text
    ])) NOT VALID;
