#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
# shellcheck disable=SC1091
source "${ROOT_DIR}/infra/host/common.sh"

BACKUP_PATH="${1:-}"
ATTACHMENTS_DIR="$(attachment_dir_path)"

if [[ -z "${BACKUP_PATH}" ]]; then
  BACKUP_PATH="$(find "$(backup_dir_path)" -mindepth 1 -maxdepth 1 -type d | sort | tail -n 1)"
fi

if [[ -z "${BACKUP_PATH}" || ! -d "${BACKUP_PATH}" ]]; then
  echo "backup path is required and must point to an existing directory" >&2
  exit 1
fi
if [[ ! -s "${BACKUP_PATH}/database.sql" || ! -r "${BACKUP_PATH}/database.sql" ]]; then
  echo "backup path must contain a readable, non-empty database.sql" >&2
  exit 1
fi
if [[ ! -s "${BACKUP_PATH}/metadata.json" || ! -r "${BACKUP_PATH}/metadata.json" ]]; then
  echo "backup path must contain readable backup metadata" >&2
  exit 1
fi
if ! grep -Fqx '  "product": "choruz",' "${BACKUP_PATH}/metadata.json" \
  || ! grep -Fqx "  \"database\": \"${CHORUZ_PG_DB}\"," "${BACKUP_PATH}/metadata.json"; then
  echo "backup metadata does not describe the configured Choruz database" >&2
  exit 1
fi

require_bin "${CHORUZ_PSQL_BIN}"
require_bin "${CHORUZ_CREATEDB_BIN}"
require_bin "${CHORUZ_DROPDB_BIN}"

echo "Stopping services before restore..."
if command -v systemctl &>/dev/null; then
  for service in choruz-api-gateway choruz-pipeline choruz-web-app; do
    if systemctl cat "${service}" >/dev/null 2>&1; then
      systemctl stop "${service}"
    fi
  done
elif command -v launchctl &>/dev/null; then
  for label in com.choruz.api-gateway com.choruz.pipeline com.choruz.web-app; do
    domain="gui/$(id -u)/${label}"
    if launchctl print "${domain}" >/dev/null 2>&1; then
      launchctl bootout "${domain}"
    fi
  done
fi
# Stop only the exact host-native cluster; never signal a PID from a mutable
# file. pg_ctl owns the cluster identity through its exact -D directory.
PG_DATA_DIR="${DATA_DIR}/postgres"
case "${CHORUZ_PG_HOST}" in
  127.0.0.1|localhost|::1)
    HOST_NATIVE_POSTGRES=true
    ;;
  *)
    HOST_NATIVE_POSTGRES=false
    ;;
esac
if [[ "${HOST_NATIVE_POSTGRES}" == true && -f "${PG_DATA_DIR}/postmaster.pid" ]]; then
  require_bin "${CHORUZ_PG_CTL_BIN}"
  "${CHORUZ_PG_CTL_BIN}" -D "${PG_DATA_DIR}" stop -m fast

  # Fail closed if another process still owns the configured endpoint, then
  # restart only this exact cluster for the restore itself.
  if "${CHORUZ_PSQL_BIN}" -h "${CHORUZ_PG_HOST}" -p "${CHORUZ_PG_PORT}" -U "${CHORUZ_PG_USER}" -d postgres -c 'SELECT 1' >/dev/null 2>&1; then
    echo "PostgreSQL is still reachable after exact-cluster stop; refusing destructive restore" >&2
    exit 1
  fi
  "${CHORUZ_PG_CTL_BIN}" -D "${PG_DATA_DIR}" -l "${LOG_DIR}/postgres.log" -o "-p ${CHORUZ_PG_PORT}" start >/dev/null
  for _ in {1..10}; do
    "${CHORUZ_PSQL_BIN}" -h "${CHORUZ_PG_HOST}" -p "${CHORUZ_PG_PORT}" -U "${CHORUZ_PG_USER}" -d postgres -c 'SELECT 1' >/dev/null 2>&1 && break
    sleep 1
  done
  if ! "${CHORUZ_PSQL_BIN}" -h "${CHORUZ_PG_HOST}" -p "${CHORUZ_PG_PORT}" -U "${CHORUZ_PG_USER}" -d postgres -c 'SELECT 1' >/dev/null 2>&1; then
    echo "host-native PostgreSQL did not restart for restore" >&2
    exit 1
  fi
  HOST_POSTGRES_PID="$(sed -n '1p' "${PG_DATA_DIR}/postmaster.pid")"
  if [[ ! "${HOST_POSTGRES_PID}" =~ ^[0-9]+$ ]]; then
    echo "host-native PostgreSQL did not publish a valid PID" >&2
    exit 1
  fi
  write_pid postgres "${HOST_POSTGRES_PID}"
fi

if ! "${CHORUZ_PSQL_BIN}" -h "${CHORUZ_PG_HOST}" -p "${CHORUZ_PG_PORT}" -U "${CHORUZ_PG_USER}" -d postgres -c 'SELECT 1' >/dev/null 2>&1; then
  echo "PostgreSQL is not reachable for restore" >&2
  exit 1
fi

drop_database_strict "${CHORUZ_PG_DB}"
ensure_database_strict "${CHORUZ_PG_DB}"

pg_exec --single-transaction \
  -d "${CHORUZ_PG_DB}" \
  -f "${BACKUP_PATH}/database.sql" >/dev/null

rm -rf "${ATTACHMENTS_DIR}"
mkdir -p "${ATTACHMENTS_DIR}"
if [[ -d "${BACKUP_PATH}/attachments" ]]; then
  cp -R "${BACKUP_PATH}/attachments/." "${ATTACHMENTS_DIR}/"
fi

if [ -f "${BACKUP_PATH}/agent_tokens.json" ]; then
  cp "${BACKUP_PATH}/agent_tokens.json" "${RUNTIME_DIR}/agent_tokens.json"
  chmod 600 "${RUNTIME_DIR}/agent_tokens.json"
  echo "  agent_tokens.json restored"
else
  rm -f "${RUNTIME_DIR}/agent_tokens.json"
fi

echo "restored ${BACKUP_PATH}"
