#!/usr/bin/env bash
# Deterministic, disposable evidence for the offline-only conversion guide.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FIXTURE_ROOT=""
PG_PORT="${CHORUZ_REHEARSAL_PG_PORT:-55438}"
POLICY="${1:-all}"

fail() { echo "conversion rehearsal: $*" >&2; exit 1; }
require_bin() { command -v "$1" >/dev/null || fail "required executable not found: $1"; }
sha256() { shasum -a 256 "$1" | awk '{print $1}'; }

cleanup() {
  if [[ -n "$FIXTURE_ROOT" && -f "$FIXTURE_ROOT/postgres/postmaster.pid" ]]; then
    pg_ctl -D "$FIXTURE_ROOT/postgres" -m immediate stop >/dev/null 2>&1 || true
  fi
  [[ -z "$FIXTURE_ROOT" || ! -e "$FIXTURE_ROOT" ]] || rm -rf "$FIXTURE_ROOT"
}
trap cleanup EXIT

within_fixture() {
  local path="$1" root resolved
  root="$(cd "$FIXTURE_ROOT" && pwd -P)"
  [[ "$path" != *"/../"* && "$path" != */.. ]] || fail "refusing parent traversal: $path"
  resolved="$(python3 -c 'import os, sys; print(os.path.realpath(sys.argv[1]))' "$path")"
  [[ "$resolved" == "$root" || "$resolved" == "$root"/* ]] || fail "refusing path outside created fixture root: $path"
}

fixture_path() { local path="$FIXTURE_ROOT/$1"; within_fixture "$path"; printf '%s\n' "$path"; }

assert_absent() { [[ ! -e "$1" ]] || fail "unexpected path remains: $1"; }
assert_present() { [[ -e "$1" ]] || fail "missing expected path: $1"; }

start_postgres() {
  initdb -D "$(fixture_path postgres)" --no-locale --encoding=UTF8 --auth=trust >/dev/null
  pg_ctl -D "$(fixture_path postgres)" -o "-p $PG_PORT -k $(fixture_path socket)" -w start >/dev/null
  createdb -h "$(fixture_path socket)" -p "$PG_PORT" rehearsal
}

psql_fixture() { psql -v ON_ERROR_STOP=1 -h "$(fixture_path socket)" -p "$PG_PORT" -d rehearsal "$@"; }

create_fixture() {
  mkdir -p "$(fixture_path socket)" \
    "$(fixture_path legacy/direct/.echat-outbox/new)" \
    "$(fixture_path legacy/direct/.echat-outbox/cur)" \
    "$(fixture_path legacy/direct/.echat-outbox/tmp)" \
    "$(fixture_path legacy/group/.echat-outbox/new)" \
    "$(fixture_path legacy/attachments)" \
    "$(fixture_path legacy/runtime/bindings)" \
    "$(fixture_path legacy/git/worktrees/session-a)"
  printf '%s\n' 'direct terminal binding' > "$(fixture_path legacy/runtime/bindings/direct.json)"
  printf '%s\n' 'external-session provenance' > "$(fixture_path legacy/runtime/bindings/external.json)"
  printf '%s\n' 'attachment payload' > "$(fixture_path legacy/attachments/fixture.txt)"
  printf '%s\n' 'queued direct command' > "$(fixture_path legacy/direct/.echat-outbox/new/command.json)"
  printf '%s\n' 'cur result' > "$(fixture_path legacy/direct/.echat-outbox/cur/result.json)"
  printf '%s\n' 'tmp envelope' > "$(fixture_path legacy/direct/.echat-outbox/tmp/pending.json)"
  printf '%s\n' 'queued group command' > "$(fixture_path legacy/group/.echat-outbox/new/group.json)"
  printf '%s\n' 'refs/heads/echat/session-a' > "$(fixture_path legacy/git/refs)"
  printf '%s\n' 'echat-bootstrap-marker' > "$(fixture_path legacy/git/worktrees/session-a/.echat-bootstrap)"
  printf '%s\n' 'stopped' > "$(fixture_path writers.state)"
  start_postgres
  psql_fixture <<'SQL'
CREATE TABLE runtime_bindings (id text primary key, workspace_path text, external_session_id text);
CREATE TABLE conversion_queue_results (id text primary key, value text);
INSERT INTO runtime_bindings VALUES ('binding-direct', 'legacy/direct', 'external-session-fixture');
INSERT INTO conversion_queue_results VALUES ('result-1', 'queued result');
SQL
}

verify_stopped_writers() { [[ "$(cat "$(fixture_path writers.state)")" == stopped ]] || fail "legacy writer is still active; refusing before queue or filesystem conversion"; }

backup_fixture() {
  local backup="$(fixture_path backup)"
  mkdir -p "$backup"
  cp -R "$(fixture_path legacy)" "$backup/filesystem"
  pg_dump -h "$(fixture_path socket)" -p "$PG_PORT" rehearsal > "$backup/database.sql"
  (cd "$backup" && find filesystem -type f -print0 | sort -z | xargs -0 shasum -a 256; shasum -a 256 database.sql) > "$backup/SHA256SUMS"
  (cd "$backup" && shasum -a 256 -c SHA256SUMS >/dev/null) || fail "backup checksum verification failed"
}

restore_fixture() {
  local backup="$(fixture_path backup)"
  (cd "$backup" && shasum -a 256 -c SHA256SUMS >/dev/null) || fail "refusing unverified backup restore"
  rm -rf "$(fixture_path restored)"
  cp -R "$backup/filesystem" "$(fixture_path restored)"
  dropdb -h "$(fixture_path socket)" -p "$PG_PORT" rehearsal
  createdb -h "$(fixture_path socket)" -p "$PG_PORT" rehearsal
  psql_fixture -f "$backup/database.sql" >/dev/null
  cmp "$(fixture_path restored/attachments/fixture.txt)" "$backup/filesystem/attachments/fixture.txt"
  [[ "$(psql_fixture -Atc 'SELECT external_session_id FROM runtime_bindings WHERE id = '\''binding-direct'\''')" == external-session-fixture ]] || fail "database restoration did not restore provenance"
}

drain_or_discard() {
  local policy="$1" queue evidence item relative destination legacy_root
  verify_stopped_writers
  evidence="$(fixture_path "evidence/$policy")"; mkdir -p "$evidence"
  legacy_root="$(fixture_path legacy)"
  case "$policy" in
    drain|discard) ;;
    *) fail "queue policy must be drain or discard" ;;
  esac
  while IFS= read -r -d '' queue; do
    while IFS= read -r -d '' item; do
      relative="${item#"$legacy_root"/}"
      destination="$evidence/$relative"
      mkdir -p "$(dirname "$destination")"
      [[ ! -e "$destination" ]] || fail "queue evidence collision: $relative"
      mv "$item" "$destination"
    done < <(find "$queue/new" -type f -print0)
    [[ -d "$queue/new" ]] || fail "source queue is missing its new directory"
    [[ -z "$(find "$queue/new" -mindepth 1 -print -quit)" ]] || fail "source queue was not explicitly handled"
  done < <(find "$(fixture_path legacy)" -type d -name '.echat-outbox' -print0)
}

preflight_collisions() {
  [[ ! -e "$(fixture_path choruz)" ]] || fail "legacy/new collision; refusing before mutation"
  [[ ! -e "$(fixture_path legacy/git/refs-choruz-session-a)" ]] || fail "Git ref collision; refusing before mutation"
  while IFS= read -r -d '' queue; do
    [[ ! -e "${queue%.echat-outbox}.choruz-outbox" ]] || fail "Maildir target collision; refusing before mutation"
  done < <(find "$(fixture_path legacy)" -type d -name '.echat-outbox' -print0)
  [[ ! -e "$(fixture_path legacy/git/worktrees/session-a/.choruz-bootstrap)" ]] || fail "bootstrap target collision; refusing before mutation"
}

assert_collision_refusal() {
  mkdir -p "$(fixture_path choruz)"
  if (preflight_collisions) >/dev/null 2>&1; then fail "target collision was accepted"; fi
  assert_present "$(fixture_path legacy/direct/.echat-outbox/new/command.json)"
  rmdir "$(fixture_path choruz)"
  touch "$(fixture_path legacy/git/refs-choruz-session-a)"
  if (preflight_collisions) >/dev/null 2>&1; then fail "Git-ref collision was accepted"; fi
  assert_present "$(fixture_path legacy/direct/.echat-outbox/new/command.json)"
  rm "$(fixture_path legacy/git/refs-choruz-session-a)"
  mkdir "$(fixture_path legacy/direct/.choruz-outbox)"
  if (preflight_collisions) >/dev/null 2>&1; then fail "Maildir target collision was accepted"; fi
  rmdir "$(fixture_path legacy/direct/.choruz-outbox)"
  touch "$(fixture_path legacy/git/worktrees/session-a/.choruz-bootstrap)"
  if (preflight_collisions) >/dev/null 2>&1; then fail "bootstrap target collision was accepted"; fi
  rm "$(fixture_path legacy/git/worktrees/session-a/.choruz-bootstrap)"
}

convert_fixture() {
  verify_stopped_writers
  local old="$(fixture_path legacy)" new="$(fixture_path choruz)"
  preflight_collisions
  mv "$old" "$new"
  find "$new" -depth -name '.echat-outbox' -exec sh -c 'mv "$1" "${1%.echat-outbox}.choruz-outbox"' _ {} \;
  mv "$new/git/refs" "$new/git/refs-choruz-session-a"
  mv "$new/git/worktrees/session-a/.echat-bootstrap" "$new/git/worktrees/session-a/.choruz-bootstrap"
  find "$new" -type f -exec perl -pi -e 's/e[c]hat/choruz/g' {} +
  ! rg --hidden --no-ignore -l -i 'echat' "$new" >/dev/null || fail "target contents contain an unintended legacy identifier"
  ! find "$new" -print | rg -i 'echat' >/dev/null || fail "target paths contain an unintended legacy identifier"
}

run_policy() {
  local policy="$1"
  create_fixture
  printf '%s\n' running > "$(fixture_path writers.state)"
  if (drain_or_discard "$policy") >/dev/null 2>&1; then fail "running writer did not fail closed"; fi
  printf '%s\n' stopped > "$(fixture_path writers.state)"
  backup_fixture
  restore_fixture
  assert_present "$(fixture_path restored/direct/.echat-outbox/new/command.json)"
  assert_collision_refusal
  drain_or_discard "$policy"
  assert_present "$(fixture_path "evidence/$policy/group/.echat-outbox/new/group.json")"
  convert_fixture
  assert_present "$(fixture_path choruz/direct/.choruz-outbox/cur/result.json)"
  assert_present "$(fixture_path choruz/direct/.choruz-outbox/tmp/pending.json)"
  restore_fixture
  assert_present "$(fixture_path restored/direct/.echat-outbox/new/command.json)"
  echo "conversion rehearsal passed: $policy"
}

require_bin initdb; require_bin pg_ctl; require_bin createdb; require_bin dropdb; require_bin psql; require_bin pg_dump; require_bin shasum; require_bin rg; require_bin perl; require_bin python3
case "$POLICY" in
  drain|discard) FIXTURE_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/choruz-conversion.XXXXXX")"; FIXTURE_ROOT="$(cd "$FIXTURE_ROOT" && pwd -P)"; run_policy "$POLICY" ;;
  all) FIXTURE_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/choruz-conversion.XXXXXX")"; FIXTURE_ROOT="$(cd "$FIXTURE_ROOT" && pwd -P)"; run_policy drain; cleanup; FIXTURE_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/choruz-conversion.XXXXXX")"; FIXTURE_ROOT="$(cd "$FIXTURE_ROOT" && pwd -P)"; run_policy discard ;;
  *) fail "usage: $0 [drain|discard|all]" ;;
esac
