-- Event webhook configuration. Historically created ad-hoc by runtime
-- code; declared explicitly here so test DBs (which replay only the
-- migrations directory) get the same shape as production.
CREATE TABLE IF NOT EXISTS event_webhook (
    principal_id    TEXT        PRIMARY KEY,
    url             TEXT        NOT NULL,
    event_types     TEXT[]      NOT NULL DEFAULT '{}',
    cursor          BIGINT      NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Webhook signing secret. Present for every webhook so external apps can
-- verify `X-Echat-Signature: sha256=<hmac(secret, body)>`. New webhooks
-- are always created with a freshly-generated 32-byte hex secret (see
-- `ChatApp::set_event_webhook`).
ALTER TABLE event_webhook
    ADD COLUMN IF NOT EXISTS webhook_secret TEXT NOT NULL DEFAULT '';
