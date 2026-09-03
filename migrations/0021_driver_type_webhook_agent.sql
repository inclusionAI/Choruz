-- Allow `webhook_agent` as a driver_type for agent_runtime_bindings.
-- External AI / Hermes / OpenClaw / custom HTTP agents are installed as
-- echat principals with this driver so that the pipeline knows **not** to
-- spawn a local CLI — the agent lives behind a webhook and is pushed
-- events via `event_webhook`, then replies back through the REST API
-- using its bearer secret.

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
        'acp'::text,
        'webhook_agent'::text
    ]));
