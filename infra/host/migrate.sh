#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "${SCRIPT_DIR}/common.sh"

MODE="${1:-up}"
DATABASE_NAME="${2:-${CHORUZ_PG_DB}}"
MIGRATIONS_DIR="${ROOT_DIR}/migrations"

require_bin "${CHORUZ_PSQL_BIN}"
require_bin "${CHORUZ_CREATEDB_BIN}"
require_bin "${CHORUZ_DROPDB_BIN}"

if [[ ! -d "${MIGRATIONS_DIR}" ]]; then
  echo "missing migrations directory: ${MIGRATIONS_DIR}" >&2
  exit 1
fi

run_up() {
  ensure_database "${DATABASE_NAME}"

  # Ensure migrations tracking table
  pg_exec -d "${DATABASE_NAME}" -c \
    "CREATE TABLE IF NOT EXISTS _migrations (filename TEXT PRIMARY KEY, applied_at TIMESTAMPTZ DEFAULT NOW());" \
    2>/dev/null

  local migration filename already_applied
  shopt -s nullglob
  for migration in "${MIGRATIONS_DIR}"/*.sql; do
    filename=$(basename "${migration}")
    already_applied=$(pg_exec -d "${DATABASE_NAME}" -t -c \
      "SELECT COUNT(*) FROM _migrations WHERE filename = '${filename}';" 2>/dev/null | tr -d ' ')
    if [ "$already_applied" = "0" ] || [ -z "$already_applied" ]; then
      echo "  Applying: $filename"
      pg_exec -d "${DATABASE_NAME}" -f "${migration}" >/dev/null
      pg_exec -d "${DATABASE_NAME}" -c \
        "INSERT INTO _migrations (filename) VALUES ('${filename}') ON CONFLICT DO NOTHING;"
    else
      echo "  Skip (already applied): $filename"
    fi
  done
}

run_reset() {
  pg_exec -d "${DATABASE_NAME}" <<'SQL' >/dev/null
DO $$
DECLARE
  table_record record;
BEGIN
  FOR table_record IN
    SELECT schemaname, tablename
    FROM pg_tables
    WHERE schemaname = 'public'
  LOOP
    EXECUTE format('DROP TABLE IF EXISTS %I.%I CASCADE', table_record.schemaname, table_record.tablename);
  END LOOP;
END
$$;
SQL
}

case "${MODE}" in
  up)
    run_up
    ;;
  down)
    run_reset
    ;;
  reset)
    run_reset
    ;;
  dropdb)
    drop_database "${DATABASE_NAME}"
    ;;
  *)
    echo "usage: $0 [up|down|reset|dropdb] [database_name]" >&2
    exit 1
    ;;
esac
