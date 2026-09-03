#!/bin/bash
set -euo pipefail

# Choruz stop script
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
# shellcheck disable=SC1091
source "${ROOT_DIR}/infra/host/common.sh"

GREEN='\033[0;32m'
NC='\033[0m'
log() { echo -e "${GREEN}[choruz]${NC} $1"; }

stop_pid_file() {
  local label="$1"
  local pid_file="$2"
  local process_regex="$3"
  if [[ -f "${pid_file}" ]]; then
    local pid
    pid="$(read_process_record_pid "${pid_file}")"
    if [[ -n "${pid}" ]] && kill -0 "${pid}" 2>/dev/null; then
      if process_record_matches "${pid_file}" "${process_regex}"; then
        stop_owned_process "${pid}" "${process_regex}" || true
        log "${label} stopped"
      else
        log "${label} pid file was stale or did not match this worktree"
      fi
    else
      log "${label} not running"
    fi
    rm -f "${pid_file}"
  else
    log "${label} not configured"
  fi
}

stop_port_listener() {
  local label="$1"
  local port="$2"
  local process_regex="$3"
  local stopped=false
  local pid
  for pid in $(lsof -ti ":${port}" 2>/dev/null || true); do
    if process_matches_worktree "${pid}" "${process_regex}"; then
      stop_owned_process "${pid}" "${process_regex}" || true
      stopped=true
    fi
  done
  if [[ "${stopped}" == true ]]; then
    log "${label} stopped on port ${port}"
  fi
}

stop_pid_file "pipeline watchdog" "${ROOT_DIR}/.choruz-runtime/watchdog.pid" '^n.*/infra/host/pipeline_watchdog\.sh$'
stop_pid_file "choruz-api-gateway" "${ROOT_DIR}/.choruz-runtime/api-gateway.pid" "${CHORUZ_API_GATEWAY_PROCESS_REGEX}"
stop_pid_file "choruz-pipeline" "${ROOT_DIR}/.choruz-runtime/choruz-pipeline.pid" "${CHORUZ_PIPELINE_PROCESS_REGEX}"
stop_pid_file "frontend" "${ROOT_DIR}/.choruz-runtime/web.pid" '(^cnode$|^cnext-server$|^n.*/node$|^n.*/pnpm$|^n.*/next$|^n.*/next-server$)'
stop_port_listener "choruz-api-gateway" "${CHORUZ_API_PORT}" "${CHORUZ_API_GATEWAY_PROCESS_REGEX}"
stop_port_listener "choruz-pipeline" "${CHORUZ_PIPELINE_METRICS_PORT}" "${CHORUZ_PIPELINE_PROCESS_REGEX}"
stop_port_listener "frontend" "${CHORUZ_WEB_PORT}" '(^cnode$|^cnext-server$|^n.*/next$|^n.*/next-server$)'

# The configured web port can change between reloads. Sweep only Next
# processes whose cwd is this worktree, so a stale pid record or old port
# cannot leave a `.next/dev` lock behind while preserving other worktrees.
stale_frontend_stopped=false
for pid in $(pgrep -f 'next dev|next-server' 2>/dev/null || true); do
  if process_matches_worktree "${pid}" '(^cnode$|^cnext-server$|^n.*/node$|^n.*/pnpm$|^n.*/next$|^n.*/next-server$)'; then
    stop_owned_process "${pid}" '(^cnode$|^cnext-server$|^n.*/node$|^n.*/pnpm$|^n.*/next$|^n.*/next-server$)' || true
    stale_frontend_stopped=true
  fi
done
if [[ "${stale_frontend_stopped}" == true ]]; then
  log "stale frontend processes stopped"
fi

log "choruz stopped"
