#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SCRIPT="${ROOT_DIR}/infra/host/web_e2e.sh"

grep -Fq 'CHORUZ_WEB_PORT="${WEB_PORT}" \' "${SCRIPT}"
echo "web_e2e exports its dynamically allocated web port"
