#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ENV_FILE="${ROOT_DIR}/infra/host/.env"
EXAMPLE_FILE="${ROOT_DIR}/infra/host/env.example"
CHORUZ_API_GATEWAY_PROCESS_REGEX='(^cchoruz-api-gateway$|^n.*/choruz-api-gateway$)'
CHORUZ_PIPELINE_PROCESS_REGEX='(^cchoruz-pipeline$|^n.*/choruz-pipeline$)'

CHORUZ_PG_PORT_OVERRIDE="${CHORUZ_PG_PORT:-}"
CHORUZ_API_PORT_OVERRIDE="${CHORUZ_API_PORT:-}"
CHORUZ_WEB_PORT_OVERRIDE="${CHORUZ_WEB_PORT:-}"
CHORUZ_PIPELINE_METRICS_PORT_OVERRIDE="${CHORUZ_PIPELINE_METRICS_PORT:-}"

is_auto_port_worktree() {
  if [[ "${ROOT_DIR}" == *"/.worktrees/"* ]]; then
    return 0
  fi

  # Git worktrees usually have a .git file that points at the primary repo's
  # gitdir. Support sibling/tmp worktrees too, not only the repo-local folder.
  [[ -f "${ROOT_DIR}/.git" ]] && grep -q '^gitdir: ' "${ROOT_DIR}/.git"
}

host_port_overrides_present() {
  [[ -n "${CHORUZ_PG_PORT_OVERRIDE}" \
    || -n "${CHORUZ_API_PORT_OVERRIDE}" \
    || -n "${CHORUZ_WEB_PORT_OVERRIDE}" \
    || -n "${CHORUZ_PIPELINE_METRICS_PORT_OVERRIDE}" ]]
}

host_port_allocation_is_enabled() {
  [[ "${CHORUZ_ENV:-development}" != "production" ]] && ! host_port_overrides_present
}

host_env_allocation_root() {
  local root
  root="$(host_env_value CHORUZ_PORT_ALLOCATION_ROOT)"
  if [[ -z "${root}" ]]; then
    root="$(host_env_value CHORUZ_WORKTREE_ROOT)"
  fi
  printf '%s\n' "${root}"
}

host_env_has_required_generated_keys() {
  local key example_value actual_value
  if [[ -z "$(host_env_allocation_root)" ]]; then
    return 1
  fi

  while IFS="=" read -r key example_value; do
    [[ -z "${key}" ]] && continue
    if ! grep -q "^${key}=" "${ENV_FILE}"; then
      return 1
    fi
    actual_value="$(host_env_value "${key}")"
    if [[ -n "${example_value}" && -z "${actual_value}" ]]; then
      return 1
    fi
  done < <(awk -F= '/^[A-Za-z_][A-Za-z0-9_]*=/ { print $1 "=" $2 }' "${EXAMPLE_FILE}")

  for key in \
    CHORUZ_PG_PORT \
    CHORUZ_API_PORT \
    CHORUZ_WEB_PORT \
    CHORUZ_PIPELINE_METRICS_PORT
  do
    if ! host_env_port_is_valid "${key}"; then
      return 1
    fi
  done

  return 0
}

host_env_value() {
  local key="$1"
  sed -n "s/^${key}=//p" "${ENV_FILE}" | tail -n 1
}

host_env_port_is_valid() {
  local key="$1"
  local value
  value="$(host_env_value "${key}")"
  [[ "${value}" =~ ^[0-9]+$ ]] && (( value > 0 && value <= 65535 ))
}

port_listener_pids() {
  local port="$1"
  lsof -nP -tiTCP:"${port}" -sTCP:LISTEN 2>/dev/null || true
}

port_listener_belongs_to_worktree() {
  local pid="$1"
  local cwd
  cwd="$(lsof -a -p "${pid}" -d cwd -Fn 2>/dev/null | sed -n 's/^n//p' | head -n 1)"
  [[ -n "${cwd}" && ( "${cwd}" == "${ROOT_DIR}" || "${cwd}" == "${ROOT_DIR}/"* ) ]]
}

worktree_port_is_available() {
  local port="$1"
  local pid
  for pid in $(port_listener_pids "${port}"); do
    if ! port_listener_belongs_to_worktree "${pid}"; then
      return 1
    fi
  done
  return 0
}

worktree_port_set_is_available() {
  local port
  for port in "$@"; do
    if ! worktree_port_is_available "${port}"; then
      return 1
    fi
  done
  return 0
}

allocate_worktree_ports() {
  local checksum="$1"
  local attempt offset pg_port api_port web_port pipeline_port
  for ((attempt = 0; attempt < 10000; attempt++)); do
    offset=$(((checksum + attempt) % 10000))
    pg_port=$((54000 + offset))
    api_port=$((30000 + offset))
    web_port=$((40000 + offset))
    pipeline_port=$((50000 + offset))
    if worktree_port_set_is_available "${pg_port}" "${api_port}" "${web_port}" "${pipeline_port}"; then
      printf '%s %s %s %s\n' "${pg_port}" "${api_port}" "${web_port}" "${pipeline_port}"
      return 0
    fi
  done
  echo "unable to allocate an available Choruz port set for ${ROOT_DIR}" >&2
  return 1
}

host_env_generated_ports_are_available() {
  worktree_port_set_is_available \
    "$(host_env_value CHORUZ_PG_PORT)" \
    "$(host_env_value CHORUZ_API_PORT)" \
    "$(host_env_value CHORUZ_WEB_PORT)" \
    "$(host_env_value CHORUZ_PIPELINE_METRICS_PORT)"
}

host_example_port_set_is_available() {
  worktree_port_set_is_available \
    "$(sed -n 's/^CHORUZ_PG_PORT=//p' "${EXAMPLE_FILE}" | tail -n 1)" \
    "$(sed -n 's/^CHORUZ_API_PORT=//p' "${EXAMPLE_FILE}" | tail -n 1)" \
    "$(sed -n 's/^CHORUZ_WEB_PORT=//p' "${EXAMPLE_FILE}" | tail -n 1)" \
    "$(sed -n 's/^CHORUZ_PIPELINE_METRICS_PORT=//p' "${EXAMPLE_FILE}" | tail -n 1)"
}

maybe_generate_host_env() {
  if ! host_port_allocation_is_enabled; then
    return 0
  fi

  if [[ -f "${ENV_FILE}" ]]; then
    local current_root
    current_root="$(host_env_allocation_root)"
    if [[ "${current_root}" == "${ROOT_DIR}" ]] \
      && host_env_has_required_generated_keys \
      && host_env_generated_ports_are_available; then
      return 0
    fi

    if [[ -z "${current_root}" ]]; then
      return 0
    fi

    local backup_file
    backup_file="${ENV_FILE}.backup.$(date +%Y%m%d%H%M%S)"
    mv "${ENV_FILE}" "${backup_file}"
    echo "backed up stale or unavailable generated host config: ${backup_file}" >&2
  elif ! is_auto_port_worktree && host_example_port_set_is_available; then
    return 0
  fi

  local checksum allocation pg_port api_port web_port pipeline_port tmp_file
  checksum="$(printf '%s' "${ROOT_DIR}" | cksum | awk '{print $1}')"
  if ! allocation="$(allocate_worktree_ports "${checksum}")"; then
    return 1
  fi
  read -r pg_port api_port web_port pipeline_port <<< "${allocation}"
  tmp_file="${ENV_FILE}.tmp"

  {
    echo "# Generated for ${ROOT_DIR}. DO NOT COMMIT."
    echo "CHORUZ_PORT_ALLOCATION_ROOT=${ROOT_DIR}"
    if is_auto_port_worktree; then
      echo "CHORUZ_WORKTREE_ROOT=${ROOT_DIR}"
    fi
    awk \
    -v pg_port="${pg_port}" \
    -v api_port="${api_port}" \
    -v web_port="${web_port}" \
    -v pipeline_port="${pipeline_port}" \
    '
      /^CHORUZ_PORT_ALLOCATION_ROOT=/ { next }
      /^CHORUZ_WORKTREE_ROOT=/ { next }
      /^CHORUZ_PG_PORT=/ { print "CHORUZ_PG_PORT=" pg_port; next }
      /^CHORUZ_API_PORT=/ { print "CHORUZ_API_PORT=" api_port; next }
      /^CHORUZ_WEB_PORT=/ { print "CHORUZ_WEB_PORT=" web_port; next }
      /^CHORUZ_PIPELINE_METRICS_PORT=/ { print "CHORUZ_PIPELINE_METRICS_PORT=" pipeline_port; next }
      { print }
    ' "${EXAMPLE_FILE}"
  } > "${tmp_file}"
  mv "${tmp_file}" "${ENV_FILE}"
  chmod 600 "${ENV_FILE}"
  echo "generated host port config: ${ENV_FILE}" >&2
}

maybe_generate_host_env

if [[ -f "${ENV_FILE}" ]]; then
  # shellcheck disable=SC1090
  source "${ENV_FILE}"
else
  # shellcheck disable=SC1090
  source "${EXAMPLE_FILE}"
fi

if [[ -n "${CHORUZ_PG_PORT_OVERRIDE}" ]]; then
  CHORUZ_PG_PORT="${CHORUZ_PG_PORT_OVERRIDE}"
fi
if [[ -n "${CHORUZ_API_PORT_OVERRIDE}" ]]; then
  CHORUZ_API_PORT="${CHORUZ_API_PORT_OVERRIDE}"
fi
if [[ -n "${CHORUZ_WEB_PORT_OVERRIDE}" ]]; then
  CHORUZ_WEB_PORT="${CHORUZ_WEB_PORT_OVERRIDE}"
fi
if [[ -n "${CHORUZ_PIPELINE_METRICS_PORT_OVERRIDE}" ]]; then
  CHORUZ_PIPELINE_METRICS_PORT="${CHORUZ_PIPELINE_METRICS_PORT_OVERRIDE}"
fi

if [[ -z "${CHORUZ_PG_USER:-}" ]]; then
  CHORUZ_PG_USER="$(id -un)"
fi

RUNTIME_DIR="${ROOT_DIR}/${CHORUZ_RUNTIME_DIR}"
DATA_DIR="${ROOT_DIR}/${CHORUZ_DATA_DIR}"
LOG_DIR="${ROOT_DIR}/${CHORUZ_LOG_DIR}"
PID_DIR="${RUNTIME_DIR}/pids"

mkdir -p "${DATA_DIR}" "${LOG_DIR}" "${PID_DIR}"

require_bin() {
  local binary_name="$1"
  if ! command -v "${binary_name}" >/dev/null 2>&1; then
    echo "missing required binary: ${binary_name}" >&2
    exit 1
  fi
}

resolve_bin() {
  local current_value="$1"
  shift
  if command -v "${current_value}" >/dev/null 2>&1; then
    echo "${current_value}"
    return 0
  fi

  if command -v brew >/dev/null 2>&1; then
    local candidate
    for candidate in "$@"; do
      if [[ -x "${candidate}" ]]; then
        echo "${candidate}"
        return 0
      fi
    done
  fi

  echo "${current_value}"
}

CHORUZ_PG_BIN="$(resolve_bin "${CHORUZ_PG_BIN}" "/opt/homebrew/opt/postgresql@16/bin/postgres" "/opt/homebrew/opt/postgresql@17/bin/postgres")"
CHORUZ_INITDB_BIN="$(resolve_bin "${CHORUZ_INITDB_BIN}" "/opt/homebrew/opt/postgresql@16/bin/initdb" "/opt/homebrew/opt/postgresql@17/bin/initdb")"
CHORUZ_PG_CTL_BIN="$(resolve_bin "${CHORUZ_PG_CTL_BIN}" "/opt/homebrew/opt/postgresql@16/bin/pg_ctl" "/opt/homebrew/opt/postgresql@17/bin/pg_ctl")"
CHORUZ_CREATEDB_BIN="$(resolve_bin "${CHORUZ_CREATEDB_BIN}" "/opt/homebrew/opt/postgresql@16/bin/createdb" "/opt/homebrew/opt/postgresql@17/bin/createdb")"
CHORUZ_DROPDB_BIN="$(resolve_bin "${CHORUZ_DROPDB_BIN:-dropdb}" "/opt/homebrew/opt/postgresql@16/bin/dropdb" "/opt/homebrew/opt/postgresql@17/bin/dropdb")"
CHORUZ_PSQL_BIN="$(resolve_bin "${CHORUZ_PSQL_BIN:-psql}" "/opt/homebrew/opt/postgresql@16/bin/psql" "/opt/homebrew/opt/postgresql@17/bin/psql")"
CHORUZ_PG_DUMP_BIN="$(resolve_bin "${CHORUZ_PG_DUMP_BIN:-pg_dump}" "/opt/homebrew/opt/postgresql@16/bin/pg_dump" "/opt/homebrew/opt/postgresql@17/bin/pg_dump")"

attachment_dir_path() {
  echo "${ROOT_DIR}/${CHORUZ_ATTACHMENT_DIR}"
}

backup_dir_path() {
  echo "${ROOT_DIR}/${CHORUZ_BACKUP_DIR}"
}

write_pid() {
  local service="$1"
  local pid="$2"
  echo "${pid}" > "${PID_DIR}/${service}.pid"
}

read_pid() {
  local service="$1"
  if [[ -f "${PID_DIR}/${service}.pid" ]]; then
    cat "${PID_DIR}/${service}.pid"
  fi
}

is_running() {
  local service="$1"
  local pid
  pid="$(read_pid "${service}")"
  [[ -n "${pid}" ]] && kill -0 "${pid}" >/dev/null 2>&1
}

process_metadata_match() {
  local pid="$1"
  local process_regex="$2"
  local metadata
  metadata="$(lsof +c 0 -p "${pid}" -FnPc 2>/dev/null || true)"
  [[ -n "${metadata}" ]] && printf '%s\n' "${metadata}" | grep -Eq "${process_regex}"
}

process_cwd_matches_root() {
  local pid="$1"
  local cwd
  cwd="$(lsof -a -p "${pid}" -d cwd -Fn 2>/dev/null | sed -n 's/^n//p' | head -n 1)"
  [[ -n "${cwd}" && ( "${cwd}" == "${ROOT_DIR}" || "${cwd}" == "${ROOT_DIR}/"* ) ]]
}

process_matches_worktree() {
  local pid="$1"
  local process_regex="$2"
  process_metadata_match "${pid}" "${process_regex}" && process_cwd_matches_root "${pid}"
}

process_start_time() {
  local pid="$1"
  ps -p "${pid}" -o lstart= 2>/dev/null | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//'
}

write_process_record() {
  local record_file="$1"
  local pid="$2"
  local start_time
  local tmp_file="${record_file}.tmp.$$"
  start_time="$(process_start_time "${pid}")"
  if [[ -z "${start_time}" ]]; then
    echo "cannot record process ${pid}: start time unavailable" >&2
    return 1
  fi
  printf '%s\n%s\n' "${pid}" "${start_time}" > "${tmp_file}"
  mv "${tmp_file}" "${record_file}"
}

read_process_record_pid() {
  local record_file="$1"
  sed -n '1p' "${record_file}" 2>/dev/null || true
}

process_record_matches() {
  local record_file="$1"
  local process_regex="$2"
  local pid saved_start current_start
  pid="$(read_process_record_pid "${record_file}")"
  [[ -n "${pid}" ]] || return 1
  process_matches_worktree "${pid}" "${process_regex}" || return 1
  saved_start="$(sed -n '2p' "${record_file}" 2>/dev/null || true)"
  [[ -n "${saved_start}" ]] || return 1
  current_start="$(process_start_time "${pid}")"
  [[ -n "${current_start}" && "${current_start}" == "${saved_start}" ]]
}

stop_owned_process() {
  local pid="$1"
  local process_regex="$2"
  if ! kill -0 "${pid}" 2>/dev/null; then
    return 0
  fi
  if ! process_matches_worktree "${pid}" "${process_regex}"; then
    return 2
  fi

  kill -TERM "${pid}" 2>/dev/null || true
  for _ in {1..30}; do
    if ! kill -0 "${pid}" 2>/dev/null; then
      wait "${pid}" 2>/dev/null || true
      return 0
    fi
    sleep 0.1
  done
  if process_matches_worktree "${pid}" "${process_regex}"; then
    kill -KILL "${pid}" 2>/dev/null || true
  fi
  wait "${pid}" 2>/dev/null || true
  for _ in {1..20}; do
    if ! kill -0 "${pid}" 2>/dev/null; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

service_ready() {
  local pid="$1"
  local process_regex="$2"
  local url="$3"
  local expected_service="$4"
  local body
  process_matches_worktree "${pid}" "${process_regex}" || return 1
  body="$(curl -fsS --max-time 2 "${url}" 2>/dev/null)" || return 1
  if command -v jq >/dev/null 2>&1; then
    jq -e --arg service "${expected_service}" \
      '.status == "ready" and .service == $service and .protocol_version == 1' \
      >/dev/null 2>&1 <<<"${body}"
    return
  fi
  if command -v python3 >/dev/null 2>&1; then
    python3 -c \
      'import json,sys; data=json.load(sys.stdin); sys.exit(0 if data.get("status") == "ready" and data.get("service") == sys.argv[1] and data.get("protocol_version") == 1 else 1)' \
      "${expected_service}" <<<"${body}"
    return
  fi
  local compact
  compact="$(printf '%s' "${body}" | tr -d '[:space:]')"
  [[ "${compact}" == *'"status":"ready"'* ]] || return 1
  [[ "${compact}" == *"\"service\":\"${expected_service}\""* ]] || return 1
  [[ "${compact}" == *'"protocol_version":1'* ]]
}

wait_for_service_ready() {
  local pid="$1"
  local process_regex="$2"
  local url="$3"
  local expected_service="$4"
  local attempts="${5:-40}"
  for ((attempt = 1; attempt <= attempts; attempt++)); do
    service_ready "${pid}" "${process_regex}" "${url}" "${expected_service}" && return 0
    kill -0 "${pid}" 2>/dev/null || return 1
    sleep 0.25
  done
  return 1
}

stop_pid() {
  local service="$1"
  local pid
  pid="$(read_pid "${service}")"
  if [[ -n "${pid}" ]] && kill -0 "${pid}" >/dev/null 2>&1; then
    kill "${pid}"
    wait "${pid}" 2>/dev/null || true
  fi
  rm -f "${PID_DIR:?}/${service}.pid"
}

pg_exec() {
  "${CHORUZ_PSQL_BIN}" \
    --set ON_ERROR_STOP=1 \
    -h "${CHORUZ_PG_HOST}" \
    -p "${CHORUZ_PG_PORT}" \
    -U "${CHORUZ_PG_USER}" \
    "$@"
}

ensure_database() {
  local database_name="$1"
  "${CHORUZ_CREATEDB_BIN}" \
    -p "${CHORUZ_PG_PORT}" \
    -h "${CHORUZ_PG_HOST}" \
    -U "${CHORUZ_PG_USER}" \
    "${database_name}" >/dev/null 2>&1 || true
}

drop_database() {
  local database_name="$1"
  "${CHORUZ_DROPDB_BIN}" \
    --if-exists \
    -h "${CHORUZ_PG_HOST}" \
    -p "${CHORUZ_PG_PORT}" \
    -U "${CHORUZ_PG_USER}" \
    "${database_name}" >/dev/null 2>&1 || true
}

# Restore is destructive, so it must not share the best-effort database setup
# helpers used by local startup. Callers that replace a database must observe a
# failure to drop or recreate it before they restore any data.
ensure_database_strict() {
  local database_name="$1"
  "${CHORUZ_CREATEDB_BIN}" \
    -h "${CHORUZ_PG_HOST}" \
    -p "${CHORUZ_PG_PORT}" \
    -U "${CHORUZ_PG_USER}" \
    "${database_name}"
}

drop_database_strict() {
  local database_name="$1"
  "${CHORUZ_DROPDB_BIN}" \
    --if-exists \
    -h "${CHORUZ_PG_HOST}" \
    -p "${CHORUZ_PG_PORT}" \
    -U "${CHORUZ_PG_USER}" \
    "${database_name}"
}
