#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$ROOT_DIR"

# shellcheck disable=SC1091
source "${ROOT_DIR}/infra/host/common.sh"

API_CONNECT_HOST="${CHORUZ_API_HOST}"
if [[ "${API_CONNECT_HOST}" == "0.0.0.0" ]]; then
  API_CONNECT_HOST="127.0.0.1"
fi

API_BASE_URL="http://${API_CONNECT_HOST}:${CHORUZ_API_PORT}"
DATABASE_URL="${CHORUZ_DATABASE_URL:-postgres://${CHORUZ_PG_USER}@${CHORUZ_PG_HOST}:${CHORUZ_PG_PORT}/${CHORUZ_PG_DB}}"

echo "Starting Choruz web UI"
echo "  Web UI:      http://127.0.0.1:${CHORUZ_WEB_PORT}"
echo "  API Gateway: ${API_BASE_URL}"
echo "  Sync WS:     ws://<browser-host>:${CHORUZ_API_PORT}/v1/ws/sync"
echo "  Database:    configured"

CHORUZ_API_PORT="${CHORUZ_API_PORT}" \
CHORUZ_API_BASE_URL="${API_BASE_URL}" \
CHORUZ_DATABASE_URL="${DATABASE_URL}" \
CHORUZ_PIPELINE_METRICS_PORT="${CHORUZ_PIPELINE_METRICS_PORT}" \
CHORUZ_CLAUDE_BINARY="${CHORUZ_CLAUDE_BINARY:-}" \
CHORUZ_CODEX_BINARY="${CHORUZ_CODEX_BINARY:-}" \
CHORUZ_PI_BINARY="${CHORUZ_PI_BINARY:-}" \
CHORUZ_GROK_BINARY="${CHORUZ_GROK_BINARY:-}" \
CHORUZ_OPENCODE_BINARY="${CHORUZ_OPENCODE_BINARY:-}" \
exec pnpm --dir apps/web dev --port "${CHORUZ_WEB_PORT}"
