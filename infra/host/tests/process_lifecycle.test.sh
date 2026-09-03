#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
# shellcheck disable=SC1091
source "${ROOT_DIR}/infra/host/common.sh"

if ! printf 'cchoruz-api-gateway\n' | grep -Eq "${CHORUZ_API_GATEWAY_PROCESS_REGEX}"; then
  echo "API gateway process record does not match its executable" >&2
  exit 1
fi
if ! printf 'n/tmp/target/release/choruz-api-gateway\n' | grep -Eq "${CHORUZ_API_GATEWAY_PROCESS_REGEX}"; then
  echo "API gateway executable path does not match its process pattern" >&2
  exit 1
fi

CHORUZ_ENV=production
if host_port_allocation_is_enabled; then
  echo "production unexpectedly enabled automatic port allocation" >&2
  exit 1
fi
CHORUZ_ENV=development
CHORUZ_PG_PORT_OVERRIDE=""
CHORUZ_WEB_PORT_OVERRIDE=""
CHORUZ_PIPELINE_METRICS_PORT_OVERRIDE=""
CHORUZ_API_PORT_OVERRIDE=3011
if host_port_allocation_is_enabled; then
  echo "an explicit port unexpectedly enabled automatic port allocation" >&2
  exit 1
fi
CHORUZ_API_PORT_OVERRIDE=""
if ! host_port_allocation_is_enabled; then
  echo "development without port overrides did not enable automatic allocation" >&2
  exit 1
fi

port_listener_pids() {
  case "$1" in
    30000) printf '%s\n' foreign-listener ;;
  esac
}
port_listener_belongs_to_worktree() {
  [[ "$1" == "owned-listener" ]]
}

if worktree_port_set_is_available 30000 40000; then
  echo "foreign listener was accepted as a worktree port" >&2
  exit 1
fi

allocated_ports="$(allocate_worktree_ports 0)"
if [[ "${allocated_ports}" != "54001 30001 40001 50001" ]]; then
  echo "allocator did not skip the occupied port set: ${allocated_ports}" >&2
  exit 1
fi

port_listener_pids() {
  printf '%s\n' owned-listener
}
if ! worktree_port_set_is_available 30000 40000; then
  echo "worktree-owned listeners were treated as foreign conflicts" >&2
  exit 1
fi

record_file="$(mktemp)"
child_pid=""
cleanup() {
  if [[ -n "${child_pid}" ]] && kill -0 "${child_pid}" 2>/dev/null; then
    kill -KILL "${child_pid}" 2>/dev/null || true
    wait "${child_pid}" 2>/dev/null || true
  fi
  rm -f "${record_file}"
}
trap cleanup EXIT

sleep 30 &
child_pid=$!
write_process_record "${record_file}" "${child_pid}"
if ! process_record_matches "${record_file}" '(^csleep$|^n.*/sleep$)'; then
  echo "fresh process record was rejected" >&2
  exit 1
fi

set +e
stop_owned_process "${child_pid}" '(^cnot-sleep$|^n.*/not-sleep$)'
mismatch_status=$?
set -e
if [[ "${mismatch_status}" -ne 2 ]]; then
  echo "foreign process mismatch returned ${mismatch_status}, expected 2" >&2
  exit 1
fi
if ! kill -0 "${child_pid}" 2>/dev/null; then
  echo "foreign process was stopped despite ownership mismatch" >&2
  exit 1
fi

record_before="$(cat "${record_file}")"
if write_process_record "${record_file}" 999999999 2>/dev/null; then
  echo "recorded a process without a start time" >&2
  exit 1
fi
if [[ "$(cat "${record_file}")" != "${record_before}" ]]; then
  echo "failed process-record write replaced the valid record" >&2
  exit 1
fi

saved_pid="$(read_process_record_pid "${record_file}")"
printf '%s\n' "${saved_pid}" > "${record_file}"
if process_record_matches "${record_file}" '(^csleep$|^n.*/sleep$)'; then
  echo "one-line process record was accepted" >&2
  exit 1
fi

printf '%s\n%s\n' "${saved_pid}" "stale-start-time" > "${record_file}"
if process_record_matches "${record_file}" '(^csleep$|^n.*/sleep$)'; then
  echo "stale process record was accepted" >&2
  exit 1
fi

write_process_record "${record_file}" "${child_pid}"
stop_owned_process "${child_pid}" '(^csleep$|^n.*/sleep$)'
if kill -0 "${child_pid}" 2>/dev/null; then
  echo "owned process survived graceful stop" >&2
  exit 1
fi
child_pid=""

echo "process lifecycle checks passed"
