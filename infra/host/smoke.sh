#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "${SCRIPT_DIR}/common.sh"

cleanup() {
  bash "${SCRIPT_DIR}/stop.sh" >/dev/null 2>&1 || true
}

trap cleanup EXIT

bash "${SCRIPT_DIR}/start.sh"
sleep 2

check_port() {
  local service="$1"
  local port="$2"
  if nc -z 127.0.0.1 "${port}"; then
    echo "${service}: port ${port} reachable"
  else
    echo "${service}: port ${port} unreachable" >&2
    exit 1
  fi
}

check_port postgres "${CHORUZ_PG_PORT:-5432}"
test -d "$(attachment_dir_path)"
test -d "$(backup_dir_path)"

bash "${SCRIPT_DIR}/status.sh"
