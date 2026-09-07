#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ "${CHORUZ_SMOKE_ENTRY:-}" != "${SCRIPT_DIR}/recovery_smoke.sh" ]]; then
  exec node "${SCRIPT_DIR}/isolated-smoke.mjs" "${SCRIPT_DIR}/recovery_smoke.sh" "$@"
fi
source "${SCRIPT_DIR}/common.sh"
export CHORUZ_PG_USER
bash "${SCRIPT_DIR}/start.sh"
bash "${SCRIPT_DIR}/migrate.sh" up
cargo build -p choruz-api-gateway -p choruz-pipeline
exec node "${SCRIPT_DIR}/recovery_smoke.mjs"
