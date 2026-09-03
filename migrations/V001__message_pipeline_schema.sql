-- ==========================================================================
-- V001: Message Pipeline Schema
-- ==========================================================================
-- Full-pipeline schema for the echat durable message bus (latest_plan.md §4).
--
-- This migration creates all tables required by the new message pipeline:
--   - conversation_events  (append-only conversation event store)
--   - event_outbox         (CDC source for the durable bus)
--   - mailbox_visibility   (agent visibility plane)
--   - route_decisions      (audit trail for routing decisions)
--   - session_registry     (executor session state)
--   - agent_commands       (command state machine)
--   - effect_journal       (tool gateway idempotent effect log)
--   - client_cursors       (client read position tracking)
--   - dead_letters         (unprocessable events)
--
-- Existing tables (conversations, conversation_members, principals, etc.)
-- are assumed to already exist and are referenced via TEXT foreign keys.
-- ==========================================================================

BEGIN;

-- ============================================================
-- Conversation Event Store (append-only log per conversation)
-- ============================================================

CREATE TABLE IF NOT EXISTS conversation_events (
    conversation_id TEXT        NOT NULL,
    seq             BIGINT      NOT NULL,
    event_id        TEXT        NOT NULL UNIQUE,
    event_type      TEXT        NOT NULL,       -- message, reply, system, reaction, edit, delete
    sender_id       TEXT        NOT NULL,
    content         TEXT,
    content_type    TEXT        NOT NULL DEFAULT 'text',
    metadata        JSONB       NOT NULL DEFAULT '{}',
    -- Identity-based dedup keys
    client_msg_id   TEXT,                       -- user messages (retry dedup)
    turn_id         TEXT,                       -- agent replies (commit dedup)
    reply_event_id  TEXT,                       -- alias for event_id on reply events
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (conversation_id, seq)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_ce_client_msg_id
    ON conversation_events (client_msg_id) WHERE client_msg_id IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_ce_turn_id
    ON conversation_events (turn_id) WHERE turn_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_ce_conversation_created
    ON conversation_events (conversation_id, created_at);

-- ============================================================
-- Event Outbox (CDC source -> Kafka / in-process bus)
-- ============================================================

CREATE TABLE IF NOT EXISTS event_outbox (
    id              BIGSERIAL   PRIMARY KEY,
    aggregate_type  TEXT        NOT NULL,       -- 'conversation_event'
    aggregate_id    TEXT        NOT NULL,       -- conversation_id
    event_type      TEXT        NOT NULL,
    payload         JSONB       NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    published       BOOLEAN     NOT NULL DEFAULT FALSE
);

CREATE INDEX IF NOT EXISTS idx_event_outbox_unpublished
    ON event_outbox (id) WHERE published = FALSE;

-- ============================================================
-- Mailbox Visibility (which messages each agent can see)
-- ============================================================

CREATE TABLE IF NOT EXISTS mailbox_visibility (
    agent_id        TEXT        NOT NULL,
    conversation_id TEXT        NOT NULL,
    message_id      TEXT        NOT NULL,
    event_seq       BIGINT      NOT NULL,
    visible_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (agent_id, conversation_id, message_id)
);

-- ============================================================
-- Route Decisions (audit + observability)
-- ============================================================

CREATE TABLE IF NOT EXISTS route_decisions (
    route_id        TEXT        PRIMARY KEY DEFAULT gen_random_uuid()::text,
    message_id      TEXT        NOT NULL,
    agent_id        TEXT        NOT NULL,
    conversation_id TEXT        NOT NULL,
    decision        TEXT        NOT NULL CHECK (decision IN ('trigger', 'skip', 'error')),
    reason          TEXT        NOT NULL,
    policy_snapshot JSONB       NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_rd_message
    ON route_decisions (message_id);

CREATE INDEX IF NOT EXISTS idx_rd_agent
    ON route_decisions (agent_id, created_at);

-- Idempotency: one decision per (message, agent) pair
CREATE UNIQUE INDEX IF NOT EXISTS idx_rd_message_agent
    ON route_decisions (message_id, agent_id);

-- ============================================================
-- Session Registry
-- ============================================================

CREATE TABLE IF NOT EXISTS session_registry (
    session_key           TEXT        PRIMARY KEY,  -- format: {agent_id}:{conversation_id}
    agent_id              TEXT        NOT NULL,
    conversation_id       TEXT        NOT NULL,
    executor_node_id      TEXT,                     -- which node currently owns this session
    epoch                 INT         NOT NULL DEFAULT 0,
    status                TEXT        NOT NULL DEFAULT 'idle'
                          CHECK (status IN ('idle', 'active', 'draining', 'dead')),
    workspace_snapshot_id TEXT,
    memory_context_id     TEXT,
    last_heartbeat_at     TIMESTAMPTZ,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_sr_executor
    ON session_registry (executor_node_id);

CREATE INDEX IF NOT EXISTS idx_sr_agent
    ON session_registry (agent_id);

-- ============================================================
-- Agent Commands (state machine)
-- ============================================================

CREATE TABLE IF NOT EXISTS agent_commands (
    command_id         TEXT        PRIMARY KEY DEFAULT gen_random_uuid()::text,
    route_id           TEXT        NOT NULL UNIQUE,
    session_key        TEXT        NOT NULL,
    agent_id           TEXT        NOT NULL,
    conversation_id    TEXT        NOT NULL,
    message_id         TEXT        NOT NULL,
    turn_id            TEXT        NOT NULL UNIQUE,
    status             TEXT        NOT NULL DEFAULT 'pending'
                       CHECK (status IN (
                           'pending', 'leased', 'started', 'heartbeating',
                           'succeeded', 'committed',
                           'retry_scheduled', 'dead_letter'
                       )),
    current_attempt_id TEXT,
    current_epoch      INT,
    attempt_count      INT         NOT NULL DEFAULT 0,
    max_attempts       INT         NOT NULL DEFAULT 5,
    prompt             TEXT        NOT NULL,
    metadata           JSONB       NOT NULL DEFAULT '{}',
    next_retry_at      TIMESTAMPTZ,
    last_error         TEXT,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_ac_pending
    ON agent_commands (status, next_retry_at)
    WHERE status IN ('pending', 'retry_scheduled');

CREATE INDEX IF NOT EXISTS idx_ac_session
    ON agent_commands (session_key, status);

CREATE INDEX IF NOT EXISTS idx_ac_turn
    ON agent_commands (turn_id);

-- ============================================================
-- Agent Results (execution outcome log)
-- ============================================================

CREATE TABLE IF NOT EXISTS agent_results (
    id                    BIGSERIAL   PRIMARY KEY,
    turn_id               TEXT        NOT NULL,
    attempt_id            TEXT        NOT NULL UNIQUE,
    command_id            TEXT        NOT NULL,
    session_key           TEXT        NOT NULL,
    conversation_id       TEXT        NOT NULL,
    agent_id              TEXT        NOT NULL,
    status                TEXT        NOT NULL CHECK (status IN ('succeeded', 'failed')),
    content               TEXT,
    content_type          TEXT,
    error                 TEXT,
    tool_calls_count      INT         NOT NULL DEFAULT 0,
    execution_duration_ms BIGINT      NOT NULL DEFAULT 0,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_ar_turn
    ON agent_results (turn_id);

CREATE INDEX IF NOT EXISTS idx_ar_command
    ON agent_results (command_id);

-- ============================================================
-- Effect Journal (Tool Gateway idempotent effect log)
-- ============================================================

CREATE TABLE IF NOT EXISTS effect_journal (
    tool_call_id            TEXT        PRIMARY KEY,
    attempt_id              TEXT        NOT NULL,
    turn_id                 TEXT        NOT NULL,
    tool_name               TEXT        NOT NULL,
    tool_input              JSONB       NOT NULL,
    tool_output             JSONB,
    external_idempotency_key TEXT,
    status                  TEXT        NOT NULL DEFAULT 'pending'
                            CHECK (status IN ('pending', 'executing', 'succeeded', 'failed')),
    is_mutating             BOOLEAN     NOT NULL DEFAULT TRUE,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at            TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_ej_turn
    ON effect_journal (turn_id);

CREATE INDEX IF NOT EXISTS idx_ej_attempt
    ON effect_journal (attempt_id);

-- ============================================================
-- Client Cursors (read position tracking, replaces old binding cursor)
-- ============================================================

CREATE TABLE IF NOT EXISTS client_cursors (
    client_id       TEXT        NOT NULL,
    conversation_id TEXT        NOT NULL,
    last_seen_seq   BIGINT      NOT NULL DEFAULT 0,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (client_id, conversation_id)
);

-- ============================================================
-- Dead Letters
-- ============================================================

CREATE TABLE IF NOT EXISTS dead_letters (
    id              TEXT        PRIMARY KEY DEFAULT gen_random_uuid()::text,
    source_type     TEXT        NOT NULL,       -- 'command', 'tool_effect', 'delivery'
    source_id       TEXT        NOT NULL,
    payload         JSONB       NOT NULL,
    error           TEXT        NOT NULL,
    attempt_count   INT         NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    resolved_at     TIMESTAMPTZ,
    resolved_by     TEXT
);

CREATE INDEX IF NOT EXISTS idx_dl_unresolved
    ON dead_letters (source_type, created_at)
    WHERE resolved_at IS NULL;

COMMIT;
