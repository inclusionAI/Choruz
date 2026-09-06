-- Execution leases own headless progress; bindings expose it through the
-- existing bootstrap and sync feed, not a second executor state machine.
CREATE OR REPLACE FUNCTION refresh_headless_binding_progress(target_agent TEXT) RETURNS VOID AS $$
DECLARE
    binding_id TEXT;
    is_active BOOLEAN;
BEGIN
    -- Serialize the aggregate read after the binding lock. A different
    -- conversation finishing must not erase this agent's newly leased work.
    SELECT id INTO binding_id FROM agent_runtime_bindings
    WHERE agent_principal_id = target_agent AND state IN ('idle', 'running')
    FOR UPDATE;
    IF binding_id IS NULL THEN
        RETURN;
    END IF;

    SELECT EXISTS (SELECT 1 FROM session_registry
        WHERE agent_id = target_agent AND status = 'active') INTO is_active;

    UPDATE agent_runtime_bindings
    SET state = CASE WHEN is_active THEN 'running' ELSE 'idle' END,
        updated_at = NOW()
    WHERE id = binding_id
        AND state IS DISTINCT FROM CASE WHEN is_active THEN 'running' ELSE 'idle' END;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION sync_headless_binding_progress() RETURNS TRIGGER AS $$
BEGIN
    PERFORM refresh_headless_binding_progress(NEW.agent_id);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER headless_session_progress
    AFTER UPDATE OF status, epoch ON session_registry
    FOR EACH ROW
    WHEN (OLD.status = 'active' OR NEW.status = 'active')
    EXECUTE FUNCTION sync_headless_binding_progress();

-- Reconcile work already leased when this migration is applied.
UPDATE session_registry s SET status = 'idle', executor_node_id = NULL,
    last_heartbeat_at = NULL, updated_at = NOW()
WHERE s.status = 'active' AND NOT EXISTS (
    SELECT 1 FROM agent_commands c WHERE c.session_key = s.session_key
        AND c.current_epoch = s.epoch
        AND c.status IN ('leased', 'started', 'heartbeating', 'succeeded')
);

DO $$
DECLARE
    agent TEXT;
BEGIN
    FOR agent IN SELECT DISTINCT agent_id FROM session_registry
        WHERE status = 'active'
    LOOP
        PERFORM refresh_headless_binding_progress(agent);
    END LOOP;
END;
$$;
