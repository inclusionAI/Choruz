#!/usr/bin/env bash
#
# Source this script before DB-backed tests. It creates a temporary PostgreSQL
# database, applies migrations, exports CHORUZ_TEST_DATABASE_URL, and drops the
# database when the parent shell exits.

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  echo "setup-test-database.sh must be sourced, not executed." >&2
  exit 2
fi

setup_temp_test_database() {
  if [ -n "${CHORUZ_TEST_DATABASE_URL:-}" ]; then
    return 0
  fi

  local require_database="${CHORUZ_REQUIRE_TEST_DATABASE:-0}"

  if ! command -v psql >/dev/null 2>&1; then
    if [ "$require_database" = "1" ]; then
      echo "psql is required but was not found" >&2
      return 1
    fi
    return 0
  fi

  local host="${CHORUZ_PG_HOST:-127.0.0.1}"
  local port="${CHORUZ_PG_PORT:-5432}"
  local user="${CHORUZ_PG_USER:-${USER:-postgres}}"
  local password="${CHORUZ_PG_PASSWORD:-}"
  PREFLIGHT_TEST_DB_NAME="choruz_preflight_$(date +%s)_$$"
  PREFLIGHT_TEST_DB_HOST="$host"
  PREFLIGHT_TEST_DB_PORT="$port"
  PREFLIGHT_TEST_DB_USER="$user"
  local admin_args=(-h "$host" -p "$port" -U "$user" -d postgres)

  if ! psql "${admin_args[@]}" -Atqc "SELECT 1" >/dev/null 2>&1; then
    if [ "$require_database" = "1" ]; then
      echo "PostgreSQL is required but was not reachable at $host:$port" >&2
      return 1
    fi
    return 0
  fi

  echo ""
  echo "==> creating temporary test database $PREFLIGHT_TEST_DB_NAME"
  psql "${admin_args[@]}" -v ON_ERROR_STOP=1 -c "CREATE DATABASE $PREFLIGHT_TEST_DB_NAME"

  local test_args=(-h "$host" -p "$port" -U "$user" -d "$PREFLIGHT_TEST_DB_NAME")
  for migration in migrations/*.sql; do
    psql "${test_args[@]}" -v ON_ERROR_STOP=1 -f "$migration" >/dev/null
  done

  local url="host=$host port=$port user=$user dbname=$PREFLIGHT_TEST_DB_NAME"
  if [ -n "$password" ]; then
    url="$url password=$password"
  fi
  export CHORUZ_TEST_DATABASE_URL="$url"

  cleanup_temp_test_database() {
    echo ""
    echo "==> dropping temporary test database $PREFLIGHT_TEST_DB_NAME"
    psql -h "$PREFLIGHT_TEST_DB_HOST" -p "$PREFLIGHT_TEST_DB_PORT" -U "$PREFLIGHT_TEST_DB_USER" -d postgres -v ON_ERROR_STOP=1 -c \
      "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '$PREFLIGHT_TEST_DB_NAME' AND pid <> pg_backend_pid();" >/dev/null || true
    psql -h "$PREFLIGHT_TEST_DB_HOST" -p "$PREFLIGHT_TEST_DB_PORT" -U "$PREFLIGHT_TEST_DB_USER" -d postgres -v ON_ERROR_STOP=1 -c "DROP DATABASE IF EXISTS $PREFLIGHT_TEST_DB_NAME" >/dev/null || true
  }
  trap cleanup_temp_test_database EXIT
}

setup_temp_test_database
