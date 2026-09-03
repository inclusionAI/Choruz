-- Telemetry events from frontend (clicks, keydowns, errors, custom spans)
CREATE TABLE IF NOT EXISTS telemetry_event (
    id          BIGSERIAL PRIMARY KEY,
    principal_id TEXT NOT NULL,
    trace_id    TEXT,
    name        TEXT NOT NULL,
    duration_ms BIGINT,
    data        JSONB,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_telemetry_principal_time
    ON telemetry_event (principal_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_telemetry_name_time
    ON telemetry_event (name, created_at DESC);
