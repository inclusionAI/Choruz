#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

cd "${ROOT_DIR}"

cargo deny check advisories licenses bans sources
trivy fs \
  --exit-code 1 \
  --severity HIGH,CRITICAL \
  --scanners vuln,secret,misconfig \
  --skip-dirs .git,node_modules,target,apps/web/.next,releases \
  .
