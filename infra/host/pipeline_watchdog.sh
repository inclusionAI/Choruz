#!/bin/bash
set -euo pipefail

ROOT_DIR="${1:?root dir required}"
cd "${ROOT_DIR}"
# shellcheck disable=SC1091
source "${ROOT_DIR}/infra/host/common.sh"

PROCESS_REGEX='(^cchoruz-pipeline$|^n.*/choruz-pipeline$)'
PIPELINE_BINARY="${ROOT_DIR}/target/release/choruz-pipeline"
PIPELINE_RECORD="${RUNTIME_DIR}/choruz-pipeline.pid"
READY_URL="http://127.0.0.1:${CHORUZ_PIPELINE_METRICS_PORT:?}/readyz"
UNREADY_COUNT=0
RESTART_BACKOFF=2

increase_backoff() {
  RESTART_BACKOFF=$((RESTART_BACKOFF * 2))
  if [ "$RESTART_BACKOFF" -gt 30 ]; then
    RESTART_BACKOFF=30
  fi
}

while true; do
  sleep 5
  PID=$(read_process_record_pid "${PIPELINE_RECORD}")
  if [ -n "$PID" ] && process_record_matches "${PIPELINE_RECORD}" "$PROCESS_REGEX" && service_ready "$PID" "$PROCESS_REGEX" "$READY_URL" "choruz-pipeline"; then
    UNREADY_COUNT=0
    RESTART_BACKOFF=2
    continue
  fi

  UNREADY_COUNT=$((UNREADY_COUNT + 1))
  if [ "$UNREADY_COUNT" -lt 3 ]; then
    continue
  fi

  if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
    if process_record_matches "${PIPELINE_RECORD}" "$PROCESS_REGEX"; then
      echo "[watchdog] Pipeline is alive but unready; restarting PID $PID"
      stop_owned_process "$PID" "$PROCESS_REGEX" || true
    else
      echo "[watchdog] Refusing to signal stale or foreign PID $PID"
    fi
  else
    echo "[watchdog] Pipeline process is not running; restarting"
  fi

  sleep "$RESTART_BACKOFF"
  CHORUZ_DATABASE_URL="${CHORUZ_DATABASE_URL:?}" \
    CHORUZ_API_BASE_URL="${CHORUZ_API_BASE_URL:?}" \
    CHORUZ_WEB_BASE_URL="${CHORUZ_WEB_BASE_URL:?}" \
    CHORUZ_PIPELINE_METRICS_PORT="${CHORUZ_PIPELINE_METRICS_PORT:?}" \
    CHORUZ_PIPELINE_METRICS_HOST="${CHORUZ_PIPELINE_METRICS_HOST:?}" \
    "${PIPELINE_BINARY}" &
  NEW_PID=$!
  if ! write_process_record "${PIPELINE_RECORD}" "$NEW_PID"; then
    echo "[watchdog] Could not record restarted pipeline PID $NEW_PID"
    stop_owned_process "$NEW_PID" "$PROCESS_REGEX" || true
    increase_backoff
    continue
  fi
  if wait_for_service_ready "$NEW_PID" "$PROCESS_REGEX" "$READY_URL" "choruz-pipeline" 80; then
    echo "[watchdog] Pipeline restarted and ready (PID $NEW_PID)"
    UNREADY_COUNT=0
    RESTART_BACKOFF=2
  else
    echo "[watchdog] Restarted pipeline did not become ready (PID $NEW_PID)"
    stop_owned_process "$NEW_PID" "$PROCESS_REGEX" || true
    increase_backoff
  fi
done
