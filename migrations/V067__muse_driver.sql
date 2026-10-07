BEGIN;

ALTER TABLE agent_runtime_bindings
    DROP CONSTRAINT agent_runtime_bindings_driver_type_check;
ALTER TABLE agent_runtime_bindings
    ADD CONSTRAINT agent_runtime_bindings_driver_type_check
    CHECK (driver_type IN (
        'claude_print', 'claude_terminal', 'codex_exec', 'codex_app_server',
        'codex_terminal', 'muse_terminal', 'pi_terminal', 'grok_terminal',
        'opencode_terminal', 'mathcode_terminal', 'acp', 'webhook_agent'
    ));

COMMIT;
