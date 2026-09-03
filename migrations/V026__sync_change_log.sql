-- Durable, non-destructive dashboard change feed.
--
-- Unlike outbox_event, these rows are never acknowledged globally. Every
-- browser/device advances its own cursor, so one connected client cannot hide
-- changes from another one.

CREATE TABLE IF NOT EXISTS sync_change (
    cursor          BIGSERIAL PRIMARY KEY,
    principal_id    TEXT NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
    event_type      TEXT NOT NULL,
    entity_type     TEXT NOT NULL,
    entity_id       TEXT NOT NULL,
    conversation_id TEXT,
    payload         JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_sync_change_principal_cursor
    ON sync_change (principal_id, cursor);

-- Message delivery is already written once per visible principal in the same
-- transaction as conversation_events. Mirror only the canonical message event;
-- webhook acknowledgements do not affect this independent feed.
CREATE OR REPLACE FUNCTION mirror_message_to_sync_change()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.event_type = 'message.created' THEN
        INSERT INTO sync_change (
            principal_id, event_type, entity_type, entity_id,
            conversation_id, payload, created_at
        ) VALUES (
            NEW.principal_id,
            NEW.event_type,
            'message',
            COALESCE(NEW.payload->>'message_id', NEW.id),
            NEW.payload->>'conversation_id',
            NEW.payload,
            NEW.created_at
        );
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_outbox_message_sync_change ON outbox_event;
CREATE TRIGGER trg_outbox_message_sync_change
    AFTER INSERT ON outbox_event
    FOR EACH ROW EXECUTE FUNCTION mirror_message_to_sync_change();

-- Shared conversation metadata changes fan out to every active member. A
-- delete is captured BEFORE cascading membership rows disappear.
CREATE OR REPLACE FUNCTION emit_conversation_sync_change()
RETURNS TRIGGER AS $$
DECLARE
    changed_row conversation%ROWTYPE;
    kind TEXT;
BEGIN
    IF TG_OP = 'DELETE' THEN
        changed_row := OLD;
        kind := 'conversation.deleted';
    ELSE
        changed_row := NEW;
        kind := 'conversation.updated';
    END IF;

    INSERT INTO sync_change (
        principal_id, event_type, entity_type, entity_id,
        conversation_id, payload
    )
    SELECT
        cm.principal_id,
        kind,
        'conversation',
        changed_row.id,
        changed_row.id,
        jsonb_build_object('conversation_id', changed_row.id)
    FROM conversation_member cm
    WHERE cm.conv_id = changed_row.id AND cm.removed_at IS NULL;

    RETURN changed_row;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_conversation_sync_change ON conversation;
CREATE TRIGGER trg_conversation_sync_change
    AFTER UPDATE ON conversation
    FOR EACH ROW EXECUTE FUNCTION emit_conversation_sync_change();

DROP TRIGGER IF EXISTS trg_conversation_delete_sync_change ON conversation;
CREATE TRIGGER trg_conversation_delete_sync_change
    BEFORE DELETE ON conversation
    FOR EACH ROW EXECUTE FUNCTION emit_conversation_sync_change();

-- A membership row tells the affected principal to add/update/remove the
-- conversation locally. Writers also touch the conversation once after a
-- completed batch, producing one fan-out event for existing members without
-- the quadratic event storm caused by fanning out every inserted row.
CREATE OR REPLACE FUNCTION emit_membership_sync_change()
RETURNS TRIGGER AS $$
DECLARE
    changed_conv_id TEXT;
    affected_principal TEXT;
BEGIN
    IF TG_OP = 'DELETE' THEN
        changed_conv_id := OLD.conv_id;
        affected_principal := OLD.principal_id;
    ELSE
        changed_conv_id := NEW.conv_id;
        affected_principal := NEW.principal_id;
    END IF;

    INSERT INTO sync_change (
        principal_id, event_type, entity_type, entity_id,
        conversation_id, payload
    ) VALUES (
        affected_principal,
        'conversation.members_changed',
        'conversation',
        changed_conv_id,
        changed_conv_id,
        jsonb_build_object(
            'conversation_id', changed_conv_id,
            'affected_principal_id', affected_principal
        )
    );

    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_conversation_member_sync_change ON conversation_member;
CREATE TRIGGER trg_conversation_member_sync_change
    AFTER INSERT OR UPDATE OF removed_at OR DELETE ON conversation_member
    FOR EACH ROW EXECUTE FUNCTION emit_membership_sync_change();

-- Pin and archive markers are private per-user state and therefore emit only
-- to their owner.
CREATE OR REPLACE FUNCTION emit_sidebar_marker_sync_change()
RETURNS TRIGGER AS $$
DECLARE
    changed_row RECORD;
    kind TEXT;
    entity_kind TEXT;
BEGIN
    IF TG_OP = 'DELETE' THEN
        changed_row := OLD;
    ELSE
        changed_row := NEW;
    END IF;
    entity_kind := TG_ARGV[0];
    kind := 'conversation.' || entity_kind || CASE
        WHEN TG_OP = 'DELETE' THEN '_removed'
        ELSE '_set'
    END;

    INSERT INTO sync_change (
        principal_id, event_type, entity_type, entity_id,
        conversation_id, payload
    ) VALUES (
        changed_row.principal_id,
        kind,
        entity_kind,
        changed_row.conv_id,
        changed_row.conv_id,
        jsonb_build_object('conversation_id', changed_row.conv_id)
    );

    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_conversation_pin_sync_change ON conversation_pin;
CREATE TRIGGER trg_conversation_pin_sync_change
    AFTER INSERT OR DELETE ON conversation_pin
    FOR EACH ROW EXECUTE FUNCTION emit_sidebar_marker_sync_change('pin');

DROP TRIGGER IF EXISTS trg_conversation_archive_sync_change ON conversation_archive;
CREATE TRIGGER trg_conversation_archive_sync_change
    AFTER INSERT OR DELETE ON conversation_archive
    FOR EACH ROW EXECUTE FUNCTION emit_sidebar_marker_sync_change('archive');

-- Read markers are private state, but must still converge across two tabs or
-- devices owned by the same principal.
CREATE OR REPLACE FUNCTION emit_conversation_read_sync_change()
RETURNS TRIGGER AS $$
BEGIN
    INSERT INTO sync_change (
        principal_id, event_type, entity_type, entity_id,
        conversation_id, payload
    ) VALUES (
        NEW.principal_id,
        'conversation.read_state_changed',
        'read_state',
        NEW.conv_id,
        NEW.conv_id,
        jsonb_build_object('conversation_id', NEW.conv_id)
    );
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_conversation_read_sync_change ON conversation_member;
CREATE TRIGGER trg_conversation_read_sync_change
    AFTER UPDATE OF msg_count, mention_count ON conversation_member
    FOR EACH ROW
    WHEN (OLD.msg_count IS DISTINCT FROM NEW.msg_count
       OR OLD.mention_count IS DISTINCT FROM NEW.mention_count)
    EXECUTE FUNCTION emit_conversation_read_sync_change();

CREATE OR REPLACE FUNCTION emit_thread_read_sync_change()
RETURNS TRIGGER AS $$
DECLARE
    changed_row RECORD;
BEGIN
    IF TG_OP = 'DELETE' THEN
        changed_row := OLD;
    ELSE
        changed_row := NEW;
    END IF;
    INSERT INTO sync_change (
        principal_id, event_type, entity_type, entity_id,
        conversation_id, payload
    ) VALUES (
        changed_row.principal_id,
        'thread.read_state_changed',
        'thread_read_state',
        changed_row.thread_root_id,
        changed_row.conversation_id,
        jsonb_build_object(
            'conversation_id', changed_row.conversation_id,
            'thread_root_id', changed_row.thread_root_id
        )
    );
    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_thread_read_sync_change ON thread_read_receipt;
CREATE TRIGGER trg_thread_read_sync_change
    AFTER INSERT OR UPDATE OR DELETE ON thread_read_receipt
    FOR EACH ROW EXECUTE FUNCTION emit_thread_read_sync_change();

-- Kanban/task state is scoped by conversation membership.
CREATE OR REPLACE FUNCTION emit_channel_task_sync_change()
RETURNS TRIGGER AS $$
DECLARE
    changed_row RECORD;
    kind TEXT;
BEGIN
    IF TG_OP = 'DELETE' THEN
        changed_row := OLD;
        kind := 'channel_task.deleted';
    ELSIF TG_OP = 'INSERT' THEN
        changed_row := NEW;
        kind := 'channel_task.created';
    ELSE
        changed_row := NEW;
        kind := 'channel_task.updated';
    END IF;
    INSERT INTO sync_change (
        principal_id, event_type, entity_type, entity_id,
        conversation_id, payload
    )
    SELECT
        cm.principal_id,
        kind,
        'channel_task',
        changed_row.id,
        changed_row.conversation_id,
        jsonb_build_object(
            'conversation_id', changed_row.conversation_id,
            'task_id', changed_row.id
        )
    FROM conversation_member cm
    WHERE cm.conv_id = changed_row.conversation_id AND cm.removed_at IS NULL;
    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_channel_task_sync_change ON group_workflow_task;
CREATE TRIGGER trg_channel_task_sync_change
    AFTER INSERT OR UPDATE OR DELETE ON group_workflow_task
    FOR EACH ROW EXECUTE FUNCTION emit_channel_task_sync_change();

-- Resolve every active human who can see a workspace/company. Payloads stay
-- identifier-only; clients fetch canonical authorized resources afterward.
CREATE OR REPLACE FUNCTION sync_workspace_recipients(target_workspace_id TEXT)
RETURNS TABLE(principal_id TEXT) AS $$
    SELECT DISTINCT p.id
    FROM principal p
    LEFT JOIN company c ON c.id = target_workspace_id AND c.deleted_at IS NULL
    LEFT JOIN company_member cm
      ON cm.company_id = target_workspace_id AND cm.principal_id = p.id
    WHERE p.type = 'human'
      AND p.disabled = FALSE
      AND p.deleted_at IS NULL
      AND (
          p.workspace_id = target_workspace_id
          OR c.owner_id = p.id
          OR cm.principal_id IS NOT NULL
      );
$$ LANGUAGE SQL STABLE;

CREATE OR REPLACE FUNCTION emit_principal_sync_change()
RETURNS TRIGGER AS $$
DECLARE
    changed_row principal%ROWTYPE;
    kind TEXT;
BEGIN
    IF TG_OP = 'DELETE' THEN
        changed_row := OLD;
        kind := 'principal.deleted';
    ELSIF TG_OP = 'INSERT' THEN
        changed_row := NEW;
        kind := 'principal.created';
    ELSE
        changed_row := NEW;
        kind := 'principal.updated';
    END IF;
    INSERT INTO sync_change (
        principal_id, event_type, entity_type, entity_id, payload
    )
    SELECT
        recipients.principal_id,
        kind,
        'principal',
        changed_row.id,
        jsonb_build_object('principal_id', changed_row.id)
    FROM sync_workspace_recipients(changed_row.workspace_id) recipients;
    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_principal_lifecycle_sync_change ON principal;
CREATE TRIGGER trg_principal_lifecycle_sync_change
    AFTER INSERT OR DELETE ON principal
    FOR EACH ROW EXECUTE FUNCTION emit_principal_sync_change();

DROP TRIGGER IF EXISTS trg_principal_update_sync_change ON principal;
CREATE TRIGGER trg_principal_update_sync_change
    AFTER UPDATE OF workspace_id, type, name, avatar_url,
        channel_visibility, disabled, deleted_at ON principal
    FOR EACH ROW EXECUTE FUNCTION emit_principal_sync_change();

CREATE OR REPLACE FUNCTION emit_company_sync_change()
RETURNS TRIGGER AS $$
DECLARE
    changed_row company%ROWTYPE;
    kind TEXT;
BEGIN
    IF TG_OP = 'DELETE' THEN
        changed_row := OLD;
        kind := 'company.deleted';
    ELSIF TG_OP = 'INSERT' THEN
        changed_row := NEW;
        kind := 'company.created';
    ELSE
        changed_row := NEW;
        kind := 'company.updated';
    END IF;
    INSERT INTO sync_change (
        principal_id, event_type, entity_type, entity_id, payload
    )
    SELECT
        recipients.principal_id,
        kind,
        'company',
        changed_row.id,
        jsonb_build_object('company_id', changed_row.id)
    FROM sync_workspace_recipients(changed_row.id) recipients;
    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_company_sync_change ON company;
CREATE TRIGGER trg_company_sync_change
    AFTER INSERT OR UPDATE ON company
    FOR EACH ROW EXECUTE FUNCTION emit_company_sync_change();

DROP TRIGGER IF EXISTS trg_company_delete_sync_change ON company;
CREATE TRIGGER trg_company_delete_sync_change
    BEFORE DELETE ON company
    FOR EACH ROW EXECUTE FUNCTION emit_company_sync_change();

CREATE OR REPLACE FUNCTION emit_company_member_sync_change()
RETURNS TRIGGER AS $$
DECLARE
    changed_company_id TEXT;
    affected_principal TEXT;
BEGIN
    IF TG_OP = 'DELETE' THEN
        changed_company_id := OLD.company_id;
        affected_principal := OLD.principal_id;
    ELSE
        changed_company_id := NEW.company_id;
        affected_principal := NEW.principal_id;
    END IF;
    INSERT INTO sync_change (
        principal_id, event_type, entity_type, entity_id, payload
    )
    SELECT
        recipient_id,
        'company.members_changed',
        'company',
        changed_company_id,
        jsonb_build_object(
            'company_id', changed_company_id,
            'affected_principal_id', affected_principal
        )
    FROM (
        SELECT recipients.principal_id AS recipient_id
        FROM sync_workspace_recipients(changed_company_id) recipients
        UNION
        SELECT affected_principal
    ) recipients;
    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_company_member_sync_change ON company_member;
CREATE TRIGGER trg_company_member_sync_change
    AFTER INSERT OR UPDATE OR DELETE ON company_member
    FOR EACH ROW EXECUTE FUNCTION emit_company_member_sync_change();

CREATE OR REPLACE FUNCTION emit_runtime_binding_sync_change()
RETURNS TRIGGER AS $$
DECLARE
    changed_row agent_runtime_bindings%ROWTYPE;
    target_workspace_id TEXT;
    kind TEXT;
BEGIN
    IF TG_OP = 'DELETE' THEN
        changed_row := OLD;
        kind := 'runtime_binding.deleted';
    ELSIF TG_OP = 'INSERT' THEN
        changed_row := NEW;
        kind := 'runtime_binding.created';
    ELSE
        changed_row := NEW;
        kind := 'runtime_binding.updated';
    END IF;
    SELECT p.workspace_id INTO target_workspace_id
    FROM principal p WHERE p.id = changed_row.agent_principal_id;
    IF target_workspace_id IS NOT NULL THEN
        INSERT INTO sync_change (
            principal_id, event_type, entity_type, entity_id,
            conversation_id, payload
        )
        SELECT
            recipients.principal_id,
            kind,
            'runtime_binding',
            changed_row.id,
            changed_row.conversation_id,
            jsonb_build_object(
                'binding_id', changed_row.id,
                'agent_principal_id', changed_row.agent_principal_id,
                'conversation_id', changed_row.conversation_id
            )
        FROM sync_workspace_recipients(target_workspace_id) recipients;
    END IF;
    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_runtime_binding_lifecycle_sync_change ON agent_runtime_bindings;
CREATE TRIGGER trg_runtime_binding_lifecycle_sync_change
    AFTER INSERT OR DELETE ON agent_runtime_bindings
    FOR EACH ROW EXECUTE FUNCTION emit_runtime_binding_sync_change();

DROP TRIGGER IF EXISTS trg_runtime_binding_update_sync_change ON agent_runtime_bindings;
CREATE TRIGGER trg_runtime_binding_update_sync_change
    AFTER UPDATE OF conversation_id, agent_principal_id, driver_type,
        workspace_path, git_worktree_path, external_session_id,
        external_thread_id, state, last_error, config_json
    ON agent_runtime_bindings
    FOR EACH ROW EXECUTE FUNCTION emit_runtime_binding_sync_change();
