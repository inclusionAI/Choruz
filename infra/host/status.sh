#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "${SCRIPT_DIR}/common.sh"

if is_running "postgres"; then
  echo "postgres: running (pid $(read_pid "postgres"))"
else
  echo "postgres: stopped"
fi

echo "attachments: $(attachment_dir_path)"
echo "backups: $(backup_dir_path)"
