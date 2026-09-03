#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "${SCRIPT_DIR}/common.sh"

API_PORT="${CHORUZ_BACKUP_SMOKE_API_PORT:-3300}"
API_LOG="${LOG_DIR}/api-gateway-backup-smoke.log"
API_PID_FILE="${PID_DIR}/api-gateway-backup-smoke.pid"
BACKUP_DIR="$(backup_dir_path)/smoke-$(date +"%Y%m%d%H%M%S")"
ATTACHMENTS_DIR="$(attachment_dir_path)"

cleanup() {
  if [[ -f "${API_PID_FILE}" ]]; then
    local pid
    pid="$(cat "${API_PID_FILE}")"
    kill "${pid}" >/dev/null 2>&1 || true
    wait "${pid}" 2>/dev/null || true
    rm -f "${API_PID_FILE}"
  fi
  bash "${SCRIPT_DIR}/stop.sh" >/dev/null 2>&1 || true
}

trap cleanup EXIT

json_field() {
  local payload="$1"
  local field="$2"
  python3 - <<'PY' "${payload}" "${field}"
import json
import sys

payload = json.loads(sys.argv[1])
field = sys.argv[2]
value = payload
for part in field.split("."):
    value = value[part]
if isinstance(value, (dict, list)):
    print(json.dumps(value))
else:
    print(value)
PY
}

start_api() {
  RUST_LOG=warn CHORUZ_API_PORT="${API_PORT}" cargo run -p choruz-api-gateway > "${API_LOG}" 2>&1 &
  echo "$!" > "${API_PID_FILE}"

  for _ in {1..30}; do
    if curl -fsS "http://127.0.0.1:${API_PORT}/healthz" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done

  echo "choruz-api-gateway failed to start" >&2
  exit 1
}

stop_api() {
  if [[ -f "${API_PID_FILE}" ]]; then
    local pid
    pid="$(cat "${API_PID_FILE}")"
    kill "${pid}" >/dev/null 2>&1 || true
    wait "${pid}" 2>/dev/null || true
    rm -f "${API_PID_FILE}"
  fi
}

login_response() {
  curl -fsS "http://127.0.0.1:${API_PORT}/v1/auth/local/login" \
    -H 'content-type: application/json' \
    -d "{\"username\":\"${CHORUZ_OPERATOR_USER}\",\"password\":\"${CHORUZ_OPERATOR_PASSWORD}\"}"
}

bash "${SCRIPT_DIR}/start.sh"
bash "${SCRIPT_DIR}/migrate.sh" reset
bash "${SCRIPT_DIR}/migrate.sh" up

rm -rf "${ATTACHMENTS_DIR}"
mkdir -p "${ATTACHMENTS_DIR}"

start_api

LOGIN_RESPONSE="$(login_response)"
SESSION_TOKEN="$(json_field "${LOGIN_RESPONSE}" "session_token")"
ADMIN_ID="$(json_field "${LOGIN_RESPONSE}" "principal.id")"

AGENT_RESPONSE="$(curl -fsS "http://127.0.0.1:${API_PORT}/v1/agents" \
  -H "authorization: Bearer ${SESSION_TOKEN}" \
  -H 'content-type: application/json' \
  -d "{\"actor_id\":\"${ADMIN_ID}\",\"name\":\"backup-agent\",\"scopes\":[\"messages:read\",\"messages:write\",\"events:read\"]}")"
AGENT_ID="$(json_field "${AGENT_RESPONSE}" "principal.id")"

CONVERSATION_RESPONSE="$(curl -fsS "http://127.0.0.1:${API_PORT}/v1/conversations/direct" \
  -H "authorization: Bearer ${SESSION_TOKEN}" \
  -H 'content-type: application/json' \
  -d "{\"actor_id\":\"${ADMIN_ID}\",\"peer_principal_id\":\"${AGENT_ID}\"}")"
CONVERSATION_ID="$(json_field "${CONVERSATION_RESPONSE}" "id")"

RUNTIME_BINDING_RESPONSE="$(curl -fsS "http://127.0.0.1:${API_PORT}/v1/runtime/bindings" \
  -H "authorization: Bearer ${SESSION_TOKEN}" \
  -H 'content-type: application/json' \
  -d "{\"conversation_id\":\"${CONVERSATION_ID}\",\"agent_principal_id\":\"${AGENT_ID}\",\"driver_type\":\"claude_print\",\"workspace_path\":\"/tmp/backup-runtime-workspace\",\"config_json\":{\"mention_aliases\":[\"backup-agent\"]}}")"
RUNTIME_BINDING_ID="$(json_field "${RUNTIME_BINDING_RESPONSE}" "id")"

pg_exec -d "${CHORUZ_PG_DB}" <<SQL >/dev/null
UPDATE agent_runtime_bindings
SET last_event_cursor = 17,
    last_acked_event_cursor = 13,
    last_seen_server_seq = 21,
    external_session_id = 'backup-session',
    external_thread_id = 'backup-thread'
WHERE id = '${RUNTIME_BINDING_ID}';

INSERT INTO conversation_runtime_policies (
  conversation_id,
  auto_mode,
  max_auto_turns,
  require_human_after_n_turns,
  allow_agent_to_agent,
  allow_file_write,
  default_reviewer_agent_id
) VALUES (
  '${CONVERSATION_ID}',
  'metadata_only',
  2,
  2,
  TRUE,
  FALSE,
  '${AGENT_ID}'
)
ON CONFLICT (conversation_id) DO UPDATE
SET auto_mode = EXCLUDED.auto_mode,
    max_auto_turns = EXCLUDED.max_auto_turns,
    require_human_after_n_turns = EXCLUDED.require_human_after_n_turns,
    allow_agent_to_agent = EXCLUDED.allow_agent_to_agent,
    allow_file_write = EXCLUDED.allow_file_write,
    default_reviewer_agent_id = EXCLUDED.default_reviewer_agent_id;
SQL

curl -fsS "http://127.0.0.1:${API_PORT}/v1/messages" \
  -H "authorization: Bearer ${SESSION_TOKEN}" \
  -H 'content-type: application/json' \
  -d "{\"actor_id\":\"${ADMIN_ID}\",\"conversation_id\":\"${CONVERSATION_ID}\",\"idempotency_key\":\"backup-smoke\",\"content\":\"before backup\",\"content_type\":\"text\",\"metadata\":{}}" >/dev/null

ATTACHMENT_RESPONSE="$(curl -fsS "http://127.0.0.1:${API_PORT}/v1/attachments" \
  -H "authorization: Bearer ${SESSION_TOKEN}" \
  -H 'content-type: application/json' \
  -d "{\"actor_id\":\"${ADMIN_ID}\",\"filename\":\"backup.txt\",\"content_type\":\"text/plain\",\"data_base64\":\"YmFja3VwLXNtb2tlLWF0dGFjaG1lbnQ=\"}")"
ATTACHMENT_ID="$(json_field "${ATTACHMENT_RESPONSE}" "id")"

bash "${ROOT_DIR}/infra/ops/bin/backup.sh" "${BACKUP_DIR}" >/dev/null

stop_api

bash "${SCRIPT_DIR}/migrate.sh" reset
rm -rf "${ATTACHMENTS_DIR}"
mkdir -p "${ATTACHMENTS_DIR}"

bash "${ROOT_DIR}/infra/ops/bin/restore.sh" "${BACKUP_DIR}" >/dev/null

start_api

LOGIN_RESPONSE="$(login_response)"
SESSION_TOKEN="$(json_field "${LOGIN_RESPONSE}" "session_token")"
ADMIN_ID="$(json_field "${LOGIN_RESPONSE}" "principal.id")"

CONSOLE_RESPONSE="$(curl -fsS "http://127.0.0.1:${API_PORT}/v1/console" \
  -H "authorization: Bearer ${SESSION_TOKEN}")"
python3 - <<'PY' "${CONSOLE_RESPONSE}" "${CONVERSATION_ID}"
import json
import sys

payload = json.loads(sys.argv[1])
conversation_id = sys.argv[2]
conversations = {item["id"] for item in payload["conversations"]}
if conversation_id not in conversations:
    raise SystemExit("conversation missing after restore")
messages = payload["messages_by_conversation"].get(conversation_id, [])
if not any(message["content"] == "before backup" for message in messages):
    raise SystemExit("message missing after restore")
PY

RUNTIME_RESPONSE="$(curl -fsS "http://127.0.0.1:${API_PORT}/v1/runtime/bindings" \
  -H "authorization: Bearer ${SESSION_TOKEN}")"
python3 - <<'PY' "${RUNTIME_RESPONSE}" "${RUNTIME_BINDING_ID}"
import json
import sys

payload = json.loads(sys.argv[1])
binding_id = sys.argv[2]
bindings = {item["id"]: item for item in payload}
binding = bindings.get(binding_id)
if binding is None:
    raise SystemExit("runtime binding missing after restore")
if binding["workspace_path"] != "/tmp/backup-runtime-workspace":
    raise SystemExit("runtime workspace missing after restore")
if binding["last_event_cursor"] != 17 or binding["last_acked_event_cursor"] != 13:
    raise SystemExit("runtime cursors missing after restore")
if binding["external_session_id"] != "backup-session":
    raise SystemExit("runtime session missing after restore")
if binding["external_thread_id"] != "backup-thread":
    raise SystemExit("runtime thread missing after restore")
PY

POLICY_STATE="$(pg_exec -d "${CHORUZ_PG_DB}" -At -F ':' -c "SELECT auto_mode, max_auto_turns, require_human_after_n_turns, allow_agent_to_agent::INT, allow_file_write::INT FROM conversation_runtime_policies WHERE conversation_id = '${CONVERSATION_ID}'")"
if [[ "${POLICY_STATE}" != "metadata_only:2:2:1:0" ]]; then
  echo "runtime policy restore verification failed" >&2
  exit 1
fi

DOWNLOAD_FILE="${DATA_DIR}/backup-download.bin"
curl -fsS "http://127.0.0.1:${API_PORT}/v1/attachments/${ATTACHMENT_ID}?actor_id=${ADMIN_ID}" \
  -H "authorization: Bearer ${SESSION_TOKEN}" \
  -o "${DOWNLOAD_FILE}"

if [[ "$(cat "${DOWNLOAD_FILE}")" != "backup-smoke-attachment" ]]; then
  echo "attachment restore verification failed" >&2
  exit 1
fi

echo "backup smoke passed"
