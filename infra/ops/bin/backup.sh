#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
# shellcheck disable=SC1091
source "${ROOT_DIR}/infra/host/common.sh"

require_bin "${CHORUZ_PG_DUMP_BIN}"

TIMESTAMP="$(date +"%Y%m%d%H%M%S")"
TARGET_DIR="${1:-$(backup_dir_path)/${TIMESTAMP}}"
ATTACHMENTS_DIR="$(attachment_dir_path)"

mkdir -p "${TARGET_DIR}"

"${CHORUZ_PG_DUMP_BIN}" \
  -h "${CHORUZ_PG_HOST}" \
  -p "${CHORUZ_PG_PORT}" \
  -U "${CHORUZ_PG_USER}" \
  --clean \
  --if-exists \
  --no-owner \
  --no-privileges \
  "${CHORUZ_PG_DB}" > "${TARGET_DIR}/database.sql"

if [[ -d "${ATTACHMENTS_DIR}" ]]; then
  mkdir -p "${TARGET_DIR}/attachments"
  cp -R "${ATTACHMENTS_DIR}/." "${TARGET_DIR}/attachments/"
fi

# Backup agent tokens
if [ -f "${RUNTIME_DIR}/agent_tokens.json" ]; then
  cp "${RUNTIME_DIR}/agent_tokens.json" "${TARGET_DIR}/agent_tokens.json"
  echo "  agent_tokens.json backed up"
fi

cat > "${TARGET_DIR}/metadata.json" <<EOF
{
  "created_at": "${TIMESTAMP}",
  "product": "choruz",
  "database": "${CHORUZ_PG_DB}",
  "attachment_dir": "${CHORUZ_ATTACHMENT_DIR}"
}
EOF

echo "${TARGET_DIR}"
