#!/usr/bin/env bash
set -euo pipefail

# Starts all host-native dependencies using direct processes and local data directories.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "${SCRIPT_DIR}/common.sh"

require_bin "${CHORUZ_PG_BIN}"
require_bin "${CHORUZ_INITDB_BIN}"
require_bin "${CHORUZ_PG_CTL_BIN}"
require_bin "${CHORUZ_CREATEDB_BIN}"
require_bin "${CHORUZ_PSQL_BIN}"

PG_DATA_DIR="${DATA_DIR}/postgres"
mkdir -p "${PG_DATA_DIR}" "$(attachment_dir_path)" "$(backup_dir_path)"

if [[ ! -f "${PG_DATA_DIR}/PG_VERSION" ]]; then
  "${CHORUZ_INITDB_BIN}" -D "${PG_DATA_DIR}" -U "${CHORUZ_PG_USER}" >/dev/null
fi

postgres_is_ready() {
  is_running "postgres" &&
    "${CHORUZ_PSQL_BIN}" \
      --set ON_ERROR_STOP=1 \
      -h "${CHORUZ_PG_HOST}" \
      -p "${CHORUZ_PG_PORT}" \
      -U "${CHORUZ_PG_USER}" \
      -d "${CHORUZ_PG_DB}" \
      -c 'SELECT 1' >/dev/null 2>&1
}

# A PID file alone is insufficient: a worktree can retain a running Postgres
# instance after its generated port changes. Verify the configured listener
# before deciding that the dependency is healthy.
if ! postgres_is_ready; then
  if is_running "postgres"; then
    "${CHORUZ_PG_CTL_BIN}" -D "${PG_DATA_DIR}" stop -m fast >/dev/null 2>&1 || true
  fi
  "${CHORUZ_PG_CTL_BIN}" -D "${PG_DATA_DIR}" -l "${LOG_DIR}/postgres.log" -o "-p ${CHORUZ_PG_PORT}" start >/dev/null
  sleep 2
  write_pid "postgres" "$(head -n 1 "${PG_DATA_DIR}/postmaster.pid")"
fi

"${CHORUZ_CREATEDB_BIN}" -p "${CHORUZ_PG_PORT}" -U "${CHORUZ_PG_USER}" "${CHORUZ_PG_DB}" >/dev/null 2>&1 || true

echo "host-native dependencies started (postgres + local filesystem)"
