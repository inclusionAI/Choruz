#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "${SCRIPT_DIR}/common.sh"

API_LOG="${LOG_DIR}/api-gateway-web-e2e.log"
PIPELINE_LOG="${LOG_DIR}/choruz-pipeline-web-e2e.log"
WEB_LOG="${LOG_DIR}/web-e2e.log"
API_PID_FILE="${PID_DIR}/api-gateway-web-e2e.pid"
PIPELINE_PID_FILE="${PID_DIR}/choruz-pipeline-web-e2e.pid"
WEB_PID_FILE="${PID_DIR}/web-e2e.pid"
API_PORT="${CHORUZ_API_PORT:-3000}"
WEB_PORT="${CHORUZ_WEB_PORT:-3100}"
PIPELINE_PORT="${CHORUZ_PIPELINE_METRICS_PORT:-3020}"
HOST_DATABASE_URL="postgres://${CHORUZ_PG_USER}@${CHORUZ_PG_HOST}:${CHORUZ_PG_PORT}/${CHORUZ_PG_DB}"

if [[ "$#" -eq 0 && "${CHORUZ_WEB_E2E_FULL:-0}" != "1" ]]; then
  set -- tests/e2e/app-smoke.spec.ts
fi

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
  for pid_file in "${WEB_PID_FILE}" "${PIPELINE_PID_FILE}" "${API_PID_FILE}"; do
    if [[ -f "${pid_file}" ]]; then
      local pid
      pid="$(cat "${pid_file}")"
      kill_tree "${pid}"
      rm -f "${pid_file}"
    fi
  done
  bash "${SCRIPT_DIR}/stop.sh" >/dev/null 2>&1 || true
}

trap cleanup EXIT

fail_with_logs() {
  local message="$1"
  echo "${message}" >&2
  for log in "${API_LOG}" "${PIPELINE_LOG}" "${WEB_LOG}"; do
    if [[ -f "${log}" ]]; then
      echo "--- tail ${log} ---" >&2
      tail -n 80 "${log}" >&2 || true
    fi
  done
  exit 1
}

require_pid_running() {
  local pid_file="$1"
  local label="$2"
  local pid
  if [[ ! -f "${pid_file}" ]]; then
    fail_with_logs "${label} pid file was not created"
  fi
  pid="$(cat "${pid_file}")"
  if [[ -z "${pid}" ]] || ! kill -0 "${pid}" >/dev/null 2>&1; then
    fail_with_logs "${label} process exited before readiness checks completed"
  fi
}

bash "${SCRIPT_DIR}/start.sh"
bash "${SCRIPT_DIR}/migrate.sh" reset
bash "${SCRIPT_DIR}/migrate.sh" up

cargo build -p choruz-api-gateway -p choruz-pipeline

RUST_LOG=warn \
  CHORUZ_DATABASE_URL="${HOST_DATABASE_URL}" \
  CHORUZ_ATTACHMENT_DIR="$(attachment_dir_path)" \
  CHORUZ_API_HOST="${CHORUZ_API_HOST}" \
  CHORUZ_API_PORT="${API_PORT}" \
  "${ROOT_DIR}/target/debug/choruz-api-gateway" > "${API_LOG}" 2>&1 &
echo "$!" > "${API_PID_FILE}"

INTERNAL_PROVISION_TOKEN="${CHORUZ_INTERNAL_PROVISION_TOKEN:-choruz-local-internal-provision}"

RUST_LOG=warn \
  CHORUZ_DATABASE_URL="${HOST_DATABASE_URL}" \
  CHORUZ_API_BASE_URL="http://127.0.0.1:${API_PORT}" \
  CHORUZ_WEB_BASE_URL="http://127.0.0.1:${WEB_PORT}" \
  CHORUZ_PIPELINE_METRICS_HOST="${CHORUZ_API_HOST}" \
  CHORUZ_INTERNAL_PROVISION_TOKEN="${INTERNAL_PROVISION_TOKEN}" \
  CHORUZ_PIPELINE_METRICS_PORT="${PIPELINE_PORT}" \
  "${ROOT_DIR}/target/debug/choruz-pipeline" > "${PIPELINE_LOG}" 2>&1 &
echo "$!" > "${PIPELINE_PID_FILE}"

for _ in {1..180}; do
  if curl -fsS "http://127.0.0.1:${API_PORT}/healthz" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
curl -fsS "http://127.0.0.1:${API_PORT}/healthz" >/dev/null 2>&1 \
  || fail_with_logs "choruz-api-gateway failed to become ready on port ${API_PORT}"
require_pid_running "${API_PID_FILE}" "choruz-api-gateway"

for _ in {1..60}; do
  status="$(curl -sS -o /dev/null -w "%{http_code}" "http://127.0.0.1:${PIPELINE_PORT}/ws/fanout" 2>/dev/null || true)"
  if [[ "${status}" != "000" ]]; then
    break
  fi
  sleep 1
done
status="$(curl -sS -o /dev/null -w "%{http_code}" "http://127.0.0.1:${PIPELINE_PORT}/ws/fanout" 2>/dev/null || true)"
if [[ "${status}" = "000" ]]; then
  fail_with_logs "pipeline fanout server failed to become ready on port ${PIPELINE_PORT}"
fi
require_pid_running "${PIPELINE_PID_FILE}" "choruz-pipeline"

CHORUZ_API_BASE_URL="http://127.0.0.1:${API_PORT}" \
CHORUZ_API_URL="http://127.0.0.1:${API_PORT}" \
CHORUZ_API_PORT="${API_PORT}" \
CHORUZ_WEB_PORT="${WEB_PORT}" \
CHORUZ_DATABASE_URL="${HOST_DATABASE_URL}" \
CHORUZ_INTERNAL_PROVISION_TOKEN="${INTERNAL_PROVISION_TOKEN}" \
CHORUZ_CODEX_BINARY="${CHORUZ_CODEX_BINARY:-/usr/bin/true}" \
  pnpm --dir "${SCRIPT_DIR}/../../apps/web" dev --hostname 127.0.0.1 --port "${WEB_PORT}" > "${WEB_LOG}" 2>&1 &
echo "$!" > "${WEB_PID_FILE}"

for _ in {1..60}; do
  if curl -fsS "http://127.0.0.1:${WEB_PORT}/" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
curl -fsS "http://127.0.0.1:${WEB_PORT}/" >/dev/null 2>&1 \
  || fail_with_logs "web app failed to become ready on port ${WEB_PORT}"
require_pid_running "${WEB_PID_FILE}" "web app"

CHORUZ_API_PORT="${API_PORT}" \
CHORUZ_WEB_PORT="${WEB_PORT}" \
CHORUZ_API_BASE_URL="http://127.0.0.1:${API_PORT}" \
CHORUZ_WEB_BASE_URL="http://127.0.0.1:${WEB_PORT}" \
  pnpm --dir "${SCRIPT_DIR}/../../apps/web" e2e "$@"
