#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "${SCRIPT_DIR}/common.sh"

API_LOG="${LOG_DIR}/api-gateway-perf.log"
API_PID_FILE="${PID_DIR}/api-gateway-perf.pid"
API_PORT="${CHORUZ_API_PORT:-3000}"

stop_pid_file() {
  local pid_file="$1"
  if [[ -f "${pid_file}" ]]; then
    local pid
    pid="$(cat "${pid_file}")"
    local children
    children="$(pgrep -P "${pid}" 2>/dev/null || true)"
    if [[ -n "${children}" ]]; then
      kill ${children} >/dev/null 2>&1 || true
    fi
    kill "${pid}" >/dev/null 2>&1 || true
    wait "${pid}" 2>/dev/null || true
    rm -f "${pid_file}"
  fi
}

cleanup() {
  stop_pid_file "${API_PID_FILE}"
  bash "${SCRIPT_DIR}/stop.sh" >/dev/null 2>&1 || true
}

trap cleanup EXIT

fail_with_logs() {
  local message="$1"
  echo "${message}" >&2
  for log in "${API_LOG}"; do
    if [[ -f "${log}" ]]; then
      echo "--- tail ${log} ---" >&2
      tail -n 80 "${log}" >&2 || true
    fi
  done
  exit 1
}

if ! command -v k6 >/dev/null 2>&1; then
  bash "${ROOT_DIR}/infra/host/perf/install_k6.sh"
fi

bash "${SCRIPT_DIR}/start.sh"
bash "${SCRIPT_DIR}/migrate.sh" reset
bash "${SCRIPT_DIR}/migrate.sh" up

# The k6 thresholds describe the shipped binary; a debug build misses them on
# small runners. CHORUZ_PERF_CARGO_PROFILE=dev keeps the quick local loop.
CARGO_PROFILE="${CHORUZ_PERF_CARGO_PROFILE:-release}"

# Build before starting the server so the readiness wait below only covers
# process start-up, not a compile (a cold release build takes minutes).
cargo build --profile "${CARGO_PROFILE}" -p choruz-api-gateway

RUST_LOG=warn \
  CHORUZ_DATABASE_URL="postgres://${CHORUZ_PG_USER}@127.0.0.1:${CHORUZ_PG_PORT}/${CHORUZ_PG_DB}" \
  CHORUZ_ATTACHMENT_DIR="$(attachment_dir_path)" \
  CHORUZ_API_PORT="${API_PORT}" \
  cargo run --profile "${CARGO_PROFILE}" -p choruz-api-gateway > "${API_LOG}" 2>&1 &
echo "$!" > "${API_PID_FILE}"

for _ in {1..180}; do
  if curl -fsS "http://127.0.0.1:${API_PORT}/healthz" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
curl -fsS "http://127.0.0.1:${API_PORT}/healthz" >/dev/null 2>&1 \
  || fail_with_logs "choruz-api-gateway failed to become ready on port ${API_PORT}"

CHORUZ_BASE_URL="http://127.0.0.1:${API_PORT}" \
CHORUZ_WS_BASE_URL="ws://127.0.0.1:${API_PORT}" \
CHORUZ_OPERATOR_USER="${CHORUZ_OPERATOR_USER:-operator}" \
CHORUZ_OPERATOR_PASSWORD="${CHORUZ_OPERATOR_PASSWORD:-choruz-local}" \
K6_VUS="${K6_VUS:-100}" \
K6_ITERATIONS="${K6_ITERATIONS:-100}" \
K6_TIMEOUT_MS="${K6_TIMEOUT_MS:-1500}" \
  k6 run "${ROOT_DIR}/infra/host/perf/ws-smoke.js"
