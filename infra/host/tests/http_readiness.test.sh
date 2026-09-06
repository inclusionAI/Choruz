#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
source "${ROOT_DIR}/infra/host/common.sh"

process_matches_worktree() { return 0; }
curl() {
  printf '%s' '{"status":"ready","service":"choruz-pipeline","protocol_version":1}'
  if [[ "$*" == *'%{http_code}'* ]]; then
    printf '\n%s' "${response_code}"
  fi
}

response_code=200
service_ready 1 ignored http://owned/readyz choruz-pipeline
for response_code in 302 404 503; do
  if service_ready 1 ignored http://owned/readyz choruz-pipeline; then
    echo "readiness accepted HTTP ${response_code}" >&2
    exit 1
  fi
done
echo "readiness accepts only HTTP 200 with the expected service identity"
