CREATE TABLE runtime_native_sources (
    binding_id TEXT NOT NULL REFERENCES agent_runtime_bindings(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL,
    binding_generation BIGINT NOT NULL,
    anchor JSONB NOT NULL,
    context JSONB NOT NULL,
    captured_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (binding_id, session_id, binding_generation)
);

CREATE FUNCTION retain_runtime_native_source() RETURNS TRIGGER AS $$
BEGIN
    IF NEW.config_json->'terminal_session'->>'session_id' IS NOT NULL THEN
        INSERT INTO runtime_native_sources(binding_id,session_id,binding_generation,anchor,context)
        VALUES(NEW.id,NEW.config_json->'terminal_session'->>'session_id',
            COALESCE((NEW.config_json->'terminal_session'->>'binding_generation')::BIGINT,0),
            NEW.config_json->'terminal_session',
            jsonb_build_object('driver_type',NEW.driver_type,'workspace_path',NEW.workspace_path,
                'runtime_host_id',NEW.config_json->'runtime_host_id',
                'harness_account_id',NEW.config_json->'harness_account_id'))
        ON CONFLICT(binding_id,session_id,binding_generation) DO UPDATE
            SET anchor=EXCLUDED.anchor,context=EXCLUDED.context;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER retain_runtime_native_source
AFTER INSERT OR UPDATE OF config_json ON agent_runtime_bindings
FOR EACH ROW EXECUTE FUNCTION retain_runtime_native_source();

INSERT INTO runtime_native_sources(binding_id,session_id,binding_generation,anchor,context)
SELECT id,config_json->'terminal_session'->>'session_id',
    COALESCE((config_json->'terminal_session'->>'binding_generation')::BIGINT,0),
    config_json->'terminal_session',
    jsonb_build_object('driver_type',driver_type,'workspace_path',workspace_path,
        'runtime_host_id',config_json->'runtime_host_id',
        'harness_account_id',config_json->'harness_account_id')
FROM agent_runtime_bindings
WHERE config_json->'terminal_session'->>'session_id' IS NOT NULL;
