#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "${SCRIPT_DIR}/common.sh"

TEMP_DB="choruz_migration_${USER}_$$"
UPGRADE_DB="${TEMP_DB}_upgrade"
MIGRATE_SCRIPT="${SCRIPT_DIR}/migrate.sh"
POSTGRES_WAS_RUNNING=false

if "${CHORUZ_PG_CTL_BIN}" -D "${DATA_DIR}/postgres" status >/dev/null 2>&1; then
  POSTGRES_WAS_RUNNING=true
  # Reconcile a missing/stale pid record instead of trying to start a second
  # postmaster against the same data directory.
  write_pid "postgres" "$(head -n 1 "${DATA_DIR}/postgres/postmaster.pid")"
fi

cleanup() {
  bash "${MIGRATE_SCRIPT}" dropdb "${TEMP_DB}" >/dev/null 2>&1 || true
  bash "${MIGRATE_SCRIPT}" dropdb "${UPGRADE_DB}" >/dev/null 2>&1 || true
  if [[ "${POSTGRES_WAS_RUNNING}" == false ]]; then
    bash "${SCRIPT_DIR}/stop.sh" >/dev/null 2>&1 || true
  fi
}

trap cleanup EXIT

bash "${SCRIPT_DIR}/start.sh"
sleep 2

bash "${MIGRATE_SCRIPT}" up "${TEMP_DB}"
bash "${MIGRATE_SCRIPT}" up "${TEMP_DB}"

(
  cd "${ROOT_DIR}/migrations"
  shasum -a 256 -c "${ROOT_DIR}/scripts/historical-migrations.sha256"
)

CUTOVER_STATE="$(pg_exec -d "${TEMP_DB}" -tAc \
  "SELECT
     (SELECT COUNT(*) FROM information_schema.columns
       WHERE table_schema = 'public' AND table_name = 'bridge_channel_mappings'
         AND column_name = 'choruz_conversation_id'),
     (SELECT COUNT(*) FROM information_schema.columns
       WHERE table_schema = 'public' AND table_name = 'bridge_channel_mappings'
         AND column_name = 'echat_conversation_id'),
     (SELECT COUNT(*) FROM pg_indexes
       WHERE schemaname = 'public' AND indexname = 'idx_bridge_mappings_choruz_conv'),
     (SELECT COUNT(*) FROM pg_indexes
       WHERE schemaname = 'public' AND indexname = 'idx_bridge_mappings_echat_conv'),
     (SELECT COUNT(*) FROM conversation_events
       WHERE content_type = 'application/vnd.choruz.channel-task+json'),
     (SELECT COUNT(*) FROM event_outbox
       WHERE payload->>'content_type' = 'application/vnd.choruz.channel-task+json')")"
if [[ "${CUTOVER_STATE}" != "1|0|1|0|0|0" ]]; then
  echo "unexpected Choruz bridge cutover state: ${CUTOVER_STATE}" >&2
  exit 1
fi

# Simulate the runner's only recoverable failure window: SQL committed while
# the filename marker was not recorded. The second run must be a safe no-op.
pg_exec -d "${TEMP_DB}" -c \
  "DELETE FROM _migrations WHERE filename = 'V019__choruz_database_cutover.sql'" >/dev/null
bash "${MIGRATE_SCRIPT}" up "${TEMP_DB}" >/dev/null
CUTOVER_STATE_AFTER_RECOVERY="$(pg_exec -d "${TEMP_DB}" -tAc \
  "SELECT
     (SELECT COUNT(*) FROM information_schema.columns
       WHERE table_schema = 'public' AND table_name = 'bridge_channel_mappings'
         AND column_name = 'choruz_conversation_id'),
     (SELECT COUNT(*) FROM information_schema.columns
       WHERE table_schema = 'public' AND table_name = 'bridge_channel_mappings'
         AND column_name = 'echat_conversation_id'),
     (SELECT COUNT(*) FROM pg_indexes
       WHERE schemaname = 'public' AND indexname = 'idx_bridge_mappings_choruz_conv'),
     (SELECT COUNT(*) FROM pg_indexes
       WHERE schemaname = 'public' AND indexname = 'idx_bridge_mappings_echat_conv'),
     (SELECT COUNT(*) FROM conversation_events
       WHERE content_type = 'application/vnd.choruz.channel-task+json'),
     (SELECT COUNT(*) FROM event_outbox
       WHERE payload->>'content_type' = 'application/vnd.choruz.channel-task+json')")"
if [[ "${CUTOVER_STATE_AFTER_RECOVERY}" != "${CUTOVER_STATE}" ]]; then
  echo "V019 marker-gap recovery changed the cutover schema state" >&2
  exit 1
fi
V019_MARKER="$(pg_exec -d "${TEMP_DB}" -tAc \
  "SELECT COUNT(*) FROM _migrations WHERE filename = 'V019__choruz_database_cutover.sql'")"
if [[ "${V019_MARKER}" != "1" ]]; then
  echo "V019 marker-gap recovery failed" >&2
  exit 1
fi

assert_notification() {
  local channel="$1"
  local payload="$2"
  local statement="$3"
  local listener_output
  listener_output="$(mktemp)"

  pg_exec -d "${TEMP_DB}" -c "LISTEN ${channel};" -c "SELECT pg_sleep(1);" \
    >"${listener_output}" 2>&1 &
  local listener_pid=$!
  for _ in {1..20}; do
    grep -Fq "LISTEN" "${listener_output}" && break
    sleep 0.05
  done
  pg_exec -d "${TEMP_DB}" -c "${statement}" >/dev/null
  wait "${listener_pid}"

  if ! grep -Eq "^Asynchronous notification \\\"${channel}\\\" with payload \\\"${payload}\\\" received from server process with PID [0-9]+\\.$" "${listener_output}"; then
    cat "${listener_output}" >&2
    rm -f "${listener_output}"
    echo "missing ${channel} notification payload ${payload}" >&2
    exit 1
  fi
  rm -f "${listener_output}"
}

# These are temporary, disposable connection-level LISTEN tests. They verify
# immediate trigger delivery and the exact payload without adding an
# application listener for the historical file-outbox trigger.
assert_notification "choruz_outbox" "1" \
  "INSERT INTO event_outbox (aggregate_type, aggregate_id, event_type, payload)
   VALUES ('phase3b', 'phase3b-outbox', 'phase3b', '{}'::jsonb)"
assert_notification "choruz_commands" "phase3b-command" \
  "INSERT INTO agent_commands
     (command_id, route_id, session_key, agent_id, conversation_id, message_id, turn_id, prompt)
   VALUES
     ('phase3b-command', 'phase3b-route', 'phase3b-session', 'phase3b-agent',
      'phase3b-conversation', 'phase3b-message', 'phase3b-turn', 'phase3b')"
assert_notification "choruz_file_outbox" "phase3b-file" \
  "INSERT INTO agent_file_outbox (id, agent_id, workspace_path, command_json)
   VALUES ('phase3b-file', 'phase3b-agent', '/tmp/phase3b', '{}'::jsonb)"

assert_no_notification() {
  local channel="$1"
  local statement="$2"
  local listener_output
  listener_output="$(mktemp)"
  pg_exec -d "${TEMP_DB}" -c "LISTEN ${channel};" -c "SELECT pg_sleep(1);" \
    >"${listener_output}" 2>&1 &
  local listener_pid=$!
  for _ in {1..20}; do
    grep -Fq "LISTEN" "${listener_output}" && break
    sleep 0.05
  done
  pg_exec -d "${TEMP_DB}" -c "${statement}" >/dev/null
  wait "${listener_pid}"
  if grep -Fq "Asynchronous notification" "${listener_output}"; then
    cat "${listener_output}" >&2
    rm -f "${listener_output}"
    echo "legacy channel ${channel} received a notification" >&2
    exit 1
  fi
  rm -f "${listener_output}"
}

assert_no_notification "echat_outbox" \
  "INSERT INTO event_outbox (aggregate_type, aggregate_id, event_type, payload)
   VALUES ('phase3b', 'legacy-channel', 'phase3b', '{}'::jsonb)"

CHORUZ_LISTENER_TEST_DATABASE_URL="postgres://${CHORUZ_PG_USER}@${CHORUZ_PG_HOST}:${CHORUZ_PG_PORT}/${TEMP_DB}" \
  cargo test -p choruz-store -p choruz-pipeline listener_wakes -- --nocapture

# Apply the current migration head (everything before V019) to a second
# disposable database, seed durable data, and then exercise the real forward
# SQL. This is deliberately separate from the fresh-schema path above.
ensure_database "${UPGRADE_DB}"
pg_exec -d "${UPGRADE_DB}" -c \
  "CREATE TABLE _migrations (filename TEXT PRIMARY KEY, applied_at TIMESTAMPTZ DEFAULT NOW())" >/dev/null
for migration in "${ROOT_DIR}"/migrations/*.sql; do
  filename="$(basename "${migration}")"
  [[ "${filename}" == "V019__choruz_database_cutover.sql" ]] && continue
  pg_exec -d "${UPGRADE_DB}" -f "${migration}" >/dev/null
done
pg_exec -d "${UPGRADE_DB}" <<'SQL' >/dev/null
INSERT INTO bridge_channel_mappings
  (platform, platform_channel_id, echat_conversation_id, platform_channel_name)
VALUES ('slack', 'phase3b-upgrade', 'phase3b-conversation', 'Phase 3B');
INSERT INTO conversation_events
  (conversation_id, seq, event_id, event_type, sender_id, content_type)
VALUES
  ('phase3b-conversation', 1, 'phase3b-event', 'channel_task.created', 'system',
   'application/vnd.choruz.channel-task+json');
INSERT INTO event_outbox (aggregate_type, aggregate_id, event_type, payload)
VALUES
  ('conversation_event', 'phase3b-event', 'channel_task.created',
   '{"content_type":"application/vnd.choruz.channel-task+json"}'::jsonb);
SQL

# Both columns (even if every existing value matches) and both index names are
# unsafe partial states. The migration must abort before changing either one.
pg_exec -d "${UPGRADE_DB}" -c \
  "ALTER TABLE bridge_channel_mappings ADD COLUMN choruz_conversation_id TEXT; \
   UPDATE bridge_channel_mappings SET choruz_conversation_id = echat_conversation_id" >/dev/null
if pg_exec -d "${UPGRADE_DB}" -f "${ROOT_DIR}/migrations/V019__choruz_database_cutover.sql" >/dev/null 2>&1; then
  echo "V019 accepted duplicate bridge columns with matching values" >&2
  exit 1
fi
pg_exec -d "${UPGRADE_DB}" -c \
  "UPDATE bridge_channel_mappings SET choruz_conversation_id = 'different-conversation'" >/dev/null
if pg_exec -d "${UPGRADE_DB}" -f "${ROOT_DIR}/migrations/V019__choruz_database_cutover.sql" >/dev/null 2>&1; then
  echo "V019 accepted divergent bridge columns" >&2
  exit 1
fi
pg_exec -d "${UPGRADE_DB}" -c \
  "ALTER TABLE bridge_channel_mappings DROP COLUMN choruz_conversation_id; \
   CREATE INDEX idx_bridge_mappings_choruz_conv ON bridge_channel_mappings (echat_conversation_id)" >/dev/null
if pg_exec -d "${UPGRADE_DB}" -f "${ROOT_DIR}/migrations/V019__choruz_database_cutover.sql" >/dev/null 2>&1; then
  echo "V019 accepted conflicting bridge indexes" >&2
  exit 1
fi
pg_exec -d "${UPGRADE_DB}" -c "DROP INDEX idx_bridge_mappings_choruz_conv" >/dev/null

# Simulate one trigger already replaced, then require V019 to replace the
# remaining functions and migrate the durable channel-task discriminators.
pg_exec -d "${UPGRADE_DB}" <<'SQL' >/dev/null
CREATE OR REPLACE FUNCTION notify_outbox_insert() RETURNS trigger AS $$
BEGIN
  PERFORM pg_notify('choruz_outbox', NEW.id::text);
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;
SQL
pg_exec -d "${UPGRADE_DB}" -f "${ROOT_DIR}/migrations/V019__choruz_database_cutover.sql" >/dev/null
UPGRADE_RESULT="$(pg_exec -d "${UPGRADE_DB}" -tAc \
  "SELECT
     (SELECT choruz_conversation_id FROM bridge_channel_mappings WHERE platform_channel_id = 'phase3b-upgrade'),
     (SELECT content_type FROM conversation_events WHERE event_id = 'phase3b-event'),
     (SELECT payload->>'content_type' FROM event_outbox WHERE aggregate_id = 'phase3b-event'),
     (SELECT pg_get_functiondef('notify_outbox_insert()'::regprocedure) LIKE '%choruz_outbox%'),
     (SELECT pg_get_functiondef('notify_command_insert()'::regprocedure) LIKE '%choruz_commands%'),
     (SELECT pg_get_functiondef('notify_file_outbox()'::regprocedure) LIKE '%choruz_file_outbox%')")"
EXPECTED_UPGRADE_RESULT="phase3b-conversation|application/vnd.choruz.channel-task+json|application/vnd.choruz.channel-task+json|t|t|t"
if [[ "${UPGRADE_RESULT}" != "${EXPECTED_UPGRADE_RESULT}" ]]; then
  echo "unexpected V019 upgrade result: ${UPGRADE_RESULT}" >&2
  exit 1
fi

TABLES="$(pg_exec -d "${TEMP_DB}" -tAc \
  "SELECT string_agg(table_name, ',' ORDER BY table_name)
   FROM information_schema.tables
   WHERE table_schema = 'public'
     AND table_name IN (
       'agent_runtime_bindings',
       'agent_turn_leases',
       'conversation_runtime_policies',
       'principal',
       'conversation',
       'conversation_member',
       'message',
       'receipt',
       'audit_log',
       'outbox_event',
       'app_snapshot'
     )")"
INDEXES="$(pg_exec -d "${TEMP_DB}" -tAc \
  "SELECT string_agg(indexname, ',' ORDER BY indexname)
   FROM pg_indexes
   WHERE schemaname = 'public'
     AND indexname IN (
       'agent_runtime_bindings_agent_idx',
       'agent_runtime_bindings_conversation_idx',
       'agent_runtime_bindings_state_idx',
       'agent_turn_leases_owner_idx',
       'conversation_runtime_policies_mode_idx',
       'conversation_member_active_idx',
       'message_idempotency_idx',
       'message_server_seq_idx'
     )")"

EXPECTED_TABLES="agent_runtime_bindings,agent_turn_leases,audit_log,conversation,conversation_member,conversation_runtime_policies,message,outbox_event,principal,receipt"
EXPECTED_INDEXES="agent_runtime_bindings_agent_idx,agent_runtime_bindings_conversation_idx,agent_runtime_bindings_state_idx,agent_turn_leases_owner_idx,conversation_member_active_idx,conversation_runtime_policies_mode_idx,message_idempotency_idx,message_server_seq_idx"

if [[ "${TABLES}" != "${EXPECTED_TABLES}" ]]; then
  echo "unexpected migration tables: ${TABLES}" >&2
  exit 1
fi

if [[ "${INDEXES}" != "${EXPECTED_INDEXES}" ]]; then
  echo "unexpected migration indexes: ${INDEXES}" >&2
  exit 1
fi

pg_exec -d "${TEMP_DB}" <<'SQL' >/dev/null
INSERT INTO principal (id, workspace_id, type, name)
VALUES ('principal-utf8', 'ws-utf8', 'human', 'Emoji 😀 café résumé');
SQL

UTF8_VALUE="$(pg_exec -d "${TEMP_DB}" -tAc \
  "SELECT name FROM principal WHERE id = 'principal-utf8'")"
if [[ "${UTF8_VALUE}" != "Emoji 😀 café résumé" ]]; then
  echo "utf8 verification failed: ${UTF8_VALUE}" >&2
  exit 1
fi

bash "${MIGRATE_SCRIPT}" reset "${TEMP_DB}"

REMAINING_TABLES="$(pg_exec -d "${TEMP_DB}" -tAc \
  "SELECT count(*)
   FROM information_schema.tables
   WHERE table_schema = 'public'
     AND table_name IN (
       'agent_runtime_bindings',
       'agent_turn_leases',
       'conversation_runtime_policies',
       'principal',
       'conversation',
       'conversation_member',
       'message',
       'receipt',
       'audit_log',
       'outbox_event',
       'app_snapshot'
     )")"

if [[ "${REMAINING_TABLES}" != "0" ]]; then
  echo "migration reset left tables behind: ${REMAINING_TABLES}" >&2
  exit 1
fi

echo "migration smoke passed for ${TEMP_DB}"
