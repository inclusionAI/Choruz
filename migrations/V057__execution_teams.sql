-- Convert persisted reviewer configurations, including rollback revisions and
-- frozen evaluation populations. Pending calls cannot resume across the schema.
CREATE FUNCTION pg_temp.execution_team(focus JSONB) RETURNS JSONB
LANGUAGE SQL IMMUTABLE AS $$
    SELECT CASE WHEN jsonb_typeof(focus) = 'string' THEN
        jsonb_build_object('order','serial','members',jsonb_build_array(
            jsonb_build_object('name','reviewer','prompt',focus))) ELSE 'null'::jsonb END
$$;

CREATE FUNCTION pg_temp.convert_team(value JSONB) RETURNS JSONB
LANGUAGE plpgsql AS $$
DECLARE result JSONB; key TEXT; item JSONB;
BEGIN
    IF jsonb_typeof(value) = 'array' THEN
        SELECT COALESCE(jsonb_agg(pg_temp.convert_team(element) ORDER BY ordinal),'[]'::jsonb)
        INTO result FROM jsonb_array_elements(value) WITH ORDINALITY AS a(element, ordinal);
        RETURN result;
    ELSIF jsonb_typeof(value) = 'object' THEN
        result := '{}'::jsonb;
        FOR key, item IN SELECT * FROM jsonb_each(value) LOOP
            IF key = 'preflight_focus' THEN
                result := result || jsonb_build_object('team',pg_temp.execution_team(item));
            ELSIF key = 'component' AND item = '"preflight"'::jsonb THEN
                result := result || jsonb_build_object(key,'team');
            ELSE
                result := result || jsonb_build_object(key,pg_temp.convert_team(item));
            END IF;
        END LOOP;
        RETURN result;
    END IF;
    RETURN value;
END
$$;

UPDATE experience_revision SET validation = (validation - 'preflight_role') ||
    jsonb_build_object('team', CASE WHEN jsonb_typeof(validation->'preflight_role') = 'object' THEN
        ((validation->'preflight_role') - 'focus') || jsonb_build_object('config',
            pg_temp.execution_team(validation->'preflight_role'->'focus')) ELSE 'null'::jsonb END)
WHERE validation ? 'preflight_role';

UPDATE experience_evaluation SET candidates = pg_temp.convert_team(candidates),
    optimization = pg_temp.convert_team(optimization);
UPDATE experience_evaluation SET status='cancelled',error_code='execution_team_schema_changed',
    application_status=CASE WHEN application_status='pending' THEN 'cancelled' ELSE application_status END,
    lease_token=NULL,lease_until=NULL,updated_at=NOW()
WHERE status IN ('queued','running');

UPDATE experience_policy SET optimization_settings = jsonb_set(optimization_settings,'{config}',
    (optimization_settings->'config') || '{"evolve_team":false,"max_agents":4}'::jsonb)
WHERE optimization_settings IS NOT NULL;
