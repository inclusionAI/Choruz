-- Validate the expanded allowlist without blocking ordinary reads/writes,
-- then swap constraint names atomically so validation is never absent.
ALTER TABLE agent_runtime_bindings
    VALIDATE CONSTRAINT agent_runtime_bindings_driver_type_check_v2;

BEGIN;

ALTER TABLE agent_runtime_bindings
    DROP CONSTRAINT agent_runtime_bindings_driver_type_check;

ALTER TABLE agent_runtime_bindings
    RENAME CONSTRAINT agent_runtime_bindings_driver_type_check_v2
    TO agent_runtime_bindings_driver_type_check;

COMMIT;
