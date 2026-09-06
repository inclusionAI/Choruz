ALTER TABLE telemetry_event
    ADD COLUMN workspace_id TEXT,
    ADD COLUMN event_id TEXT,
    ADD COLUMN occurred_at TIMESTAMPTZ,
    ADD COLUMN span_id TEXT,
    ADD COLUMN session_id TEXT,
    ADD COLUMN schema_version INTEGER;

UPDATE telemetry_event t SET workspace_id = p.workspace_id
FROM principal p WHERE p.id = t.principal_id;

CREATE UNIQUE INDEX telemetry_event_identity
    ON telemetry_event (workspace_id, principal_id, event_id);
CREATE INDEX telemetry_workspace_time
    ON telemetry_event (workspace_id, occurred_at, id);
