-- Validate the replacement constraint in a later transaction so the table
-- scan does not hold the write-blocking lock required to add the constraint.
ALTER TABLE agent_runtime_bindings
    VALIDATE CONSTRAINT agent_runtime_bindings_driver_type_without_gemini_check;

ALTER TABLE agent_runtime_bindings
    DROP CONSTRAINT IF EXISTS agent_runtime_bindings_driver_type_check;

ALTER TABLE agent_runtime_bindings
    RENAME CONSTRAINT agent_runtime_bindings_driver_type_without_gemini_check
    TO agent_runtime_bindings_driver_type_check;
