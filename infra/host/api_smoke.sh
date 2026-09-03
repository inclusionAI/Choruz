#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "${SCRIPT_DIR}/common.sh"

API_LOG="${LOG_DIR}/api-gateway.log"
API_PID_FILE="${PID_DIR}/api-gateway-smoke.pid"
API_PORT="${CHORUZ_API_PORT:-3000}"

kill_tree() {
  local pid="$1"
  local child
  for child in $(pgrep -P "${pid}" 2>/dev/null || true); do
    kill_tree "${child}"
  done
  kill "${pid}" >/dev/null 2>&1 || true
  wait "${pid}" 2>/dev/null || true
}

cleanup() {
  bash "${SCRIPT_DIR}/stop.sh" >/dev/null 2>&1 || true
  if [[ -f "${API_PID_FILE}" ]]; then
    local pid
    pid="$(cat "${API_PID_FILE}")"
    kill_tree "${pid}"
    rm -f "${API_PID_FILE}"
  fi
}

trap cleanup EXIT

fail_with_logs() {
  local message="$1"
  echo "${message}" >&2
  if [[ -f "${API_LOG}" ]]; then
    echo "--- tail ${API_LOG} ---" >&2
    tail -n 120 "${API_LOG}" >&2 || true
  fi
  exit 1
}

bash "${SCRIPT_DIR}/start.sh"
bash "${SCRIPT_DIR}/migrate.sh" reset
bash "${SCRIPT_DIR}/migrate.sh" up

cargo build -p choruz-api-gateway

OPERATOR_USER="${CHORUZ_OPERATOR_USER:-operator}"
OPERATOR_PASSWORD="${CHORUZ_OPERATOR_PASSWORD:-choruz-local}"

RUST_LOG=warn \
  CHORUZ_DATABASE_URL="postgres://${CHORUZ_PG_USER}@${CHORUZ_PG_HOST}:${CHORUZ_PG_PORT}/${CHORUZ_PG_DB}" \
  CHORUZ_PG_HOST="${CHORUZ_PG_HOST}" \
  CHORUZ_PG_PORT="${CHORUZ_PG_PORT}" \
  CHORUZ_PG_USER="${CHORUZ_PG_USER}" \
  CHORUZ_PG_DB="${CHORUZ_PG_DB}" \
  CHORUZ_PG_PASSWORD="${CHORUZ_PG_PASSWORD:-}" \
  CHORUZ_ATTACHMENT_DIR="$(attachment_dir_path)" \
  CHORUZ_API_HOST="${CHORUZ_API_HOST}" \
  CHORUZ_API_PORT="${API_PORT}" \
  CHORUZ_OPERATOR_USER="${OPERATOR_USER}" \
  CHORUZ_OPERATOR_PASSWORD="${OPERATOR_PASSWORD}" \
  "${ROOT_DIR}/target/debug/choruz-api-gateway" > "${API_LOG}" 2>&1 &
echo "$!" > "${API_PID_FILE}"

for _ in {1..180}; do
  if curl -fsS "http://127.0.0.1:${API_PORT}/healthz" >/dev/null 2>&1; then
    break
  fi
  if ! kill -0 "$(cat "${API_PID_FILE}")" >/dev/null 2>&1; then
    fail_with_logs "choruz-api-gateway exited before becoming ready on port ${API_PORT}"
  fi
  sleep 1
done

curl -fsS "http://127.0.0.1:${API_PORT}/healthz" \
  || fail_with_logs "choruz-api-gateway failed to become ready on port ${API_PORT}"
echo
curl -fsS "http://127.0.0.1:${API_PORT}/metrics" \
  || fail_with_logs "choruz-api-gateway metrics endpoint failed on port ${API_PORT}"
echo

LOGIN_RESPONSE="$(curl -fsS "http://127.0.0.1:${API_PORT}/v1/auth/local/login" \
  -H 'content-type: application/json' \
  -d "{\"username\":\"${OPERATOR_USER}\",\"password\":\"${OPERATOR_PASSWORD}\"}")" \
  || fail_with_logs "choruz-api-gateway login smoke failed on port ${API_PORT}"
SESSION_TOKEN="$(python3 - <<'PY' "${LOGIN_RESPONSE}"
import json
import sys

print(json.loads(sys.argv[1])["session_token"])
PY
)"

curl -fsS "http://127.0.0.1:${API_PORT}/v1/console" \
  -H "authorization: Bearer ${SESSION_TOKEN}" >/dev/null \
  || fail_with_logs "choruz-api-gateway console smoke failed on port ${API_PORT}"

echo "api smoke passed"
