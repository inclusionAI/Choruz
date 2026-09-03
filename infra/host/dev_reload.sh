#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
cd "${ROOT_DIR}"

# shellcheck disable=SC1091
source "${ROOT_DIR}/infra/host/common.sh"

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'
WEB_RECORD="${RUNTIME_DIR}/web.pid"
WEB_LOG="${LOG_DIR}/web.log"
WEB_PROCESS_REGEX='(^cnode$|^cnext-server$|^n.*/node$|^n.*/pnpm$|^n.*/next$|^n.*/next-server$)'

usage() {
  cat <<'EOF'
Usage: pnpm reload:local [--pull]

Rebuild, migrate, restart, and health-check the complete local Choruz stack.
Use --pull to run `git pull --ff-only` first; it requires a clean worktree.
EOF
}

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  usage
  exit 0
fi
if [[ $# -gt 1 || ( $# -eq 1 && "${1}" != "--pull" ) ]]; then
  usage >&2
  exit 2
fi

if [[ "${1:-}" == "--pull" ]]; then
  if [[ -n "$(git status --porcelain)" ]]; then
    echo -e "${RED}Refusing to pull over a dirty worktree. Commit or stash it first.${NC}" >&2
    exit 2
  fi
  echo -e "${YELLOW}[1/6] Pulling the latest fast-forward update...${NC}"
  git pull --ff-only
else
  echo -e "${YELLOW}[1/6] Using the current working tree...${NC}"
fi

# Build and validate before stopping the healthy stack, minimizing downtime and
# ensuring a broken update cannot replace the running services.
echo -e "${YELLOW}[2/6] Building backend...${NC}"
cargo build --release

echo -e "${YELLOW}[3/6] Checking frontend...${NC}"
pnpm --dir apps/web check

echo -e "${YELLOW}[4/6] Starting storage and applying migrations...${NC}"
pnpm host:start
pnpm db:migrate

echo -e "${YELLOW}[5/6] Restarting application services...${NC}"
bash "${ROOT_DIR}/infra/host/dev_stop.sh"
CHORUZ_SKIP_BUILD=1 CHORUZ_SKIP_MIGRATIONS=1 bash "${ROOT_DIR}/infra/host/dev.sh"

mkdir -p "${LOG_DIR}"
rm -f "${WEB_RECORD}"
nohup bash "${ROOT_DIR}/infra/host/web_dev.sh" >"${WEB_LOG}" 2>&1 &
WEB_PID=$!

cleanup_failed_web() {
  if kill -0 "${WEB_PID}" 2>/dev/null; then
    stop_owned_process "${WEB_PID}" "${WEB_PROCESS_REGEX}" || true
  fi
  rm -f "${WEB_RECORD}"
}
trap cleanup_failed_web ERR

for _ in {1..120}; do
  if ! kill -0 "${WEB_PID}" 2>/dev/null; then
    echo -e "${RED}Frontend exited during startup. See ${WEB_LOG}.${NC}" >&2
    exit 1
  fi
  if curl --silent --fail --output /dev/null "http://127.0.0.1:${CHORUZ_WEB_PORT}/"; then
    break
  fi
  sleep 0.25
done

if ! curl --silent --fail --output /dev/null "http://127.0.0.1:${CHORUZ_WEB_PORT}/"; then
  echo -e "${RED}Frontend did not become healthy. See ${WEB_LOG}.${NC}" >&2
  exit 1
fi
if ! write_process_record "${WEB_RECORD}" "${WEB_PID}"; then
  echo -e "${RED}Could not record the frontend process.${NC}" >&2
  exit 1
fi

trap - ERR
echo -e "${YELLOW}[6/6] Verifying the complete stack...${NC}"
curl --silent --fail --output /dev/null "http://127.0.0.1:${CHORUZ_API_PORT}/readyz"
curl --silent --fail --output /dev/null "http://127.0.0.1:${CHORUZ_PIPELINE_METRICS_PORT}/readyz"
curl --silent --fail --output /dev/null "http://127.0.0.1:${CHORUZ_WEB_PORT}/"

echo -e "${GREEN}Choruz reloaded successfully.${NC}"
echo "  Web:      http://127.0.0.1:${CHORUZ_WEB_PORT}"
echo "  API:      http://127.0.0.1:${CHORUZ_API_PORT}"
echo "  Pipeline: http://127.0.0.1:${CHORUZ_PIPELINE_METRICS_PORT}"
echo "  Web log:  ${WEB_LOG}"
