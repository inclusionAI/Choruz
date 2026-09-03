#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

bash -n \
  "${ROOT_DIR}/infra/ops/check.sh" \
  "${ROOT_DIR}/infra/ops/bin/backup.sh" \
  "${ROOT_DIR}/infra/ops/bin/release.sh" \
  "${ROOT_DIR}/infra/ops/bin/restore.sh" \
  "${ROOT_DIR}/infra/ops/bin/rollback.sh" \
  "${ROOT_DIR}/infra/ops/test-rollback-links.sh" \
  "${ROOT_DIR}/infra/ops/test-selectors.sh"

bash "${ROOT_DIR}/infra/ops/test-selectors.sh"
bash "${ROOT_DIR}/infra/ops/test-rollback-links.sh"

for plist in "${ROOT_DIR}"/infra/ops/launchd/*.plist; do
  if command -v plutil >/dev/null 2>&1; then
    plutil -lint "${plist}" >/dev/null
  else
    # Linux has no plutil; Python's plistlib rejects the same malformed files.
    python3 -c 'import plistlib, sys; plistlib.load(open(sys.argv[1], "rb"))' "${plist}"
  fi
done

for unit in "${ROOT_DIR}"/infra/ops/systemd/*.service; do
  grep -q '^\[Unit\]$' "${unit}"
  grep -q '^\[Service\]$' "${unit}"
  timer="${unit%.service}.timer"
  if [[ -f "${timer}" ]]; then
    grep -q '^\[Timer\]$' "${timer}"
    grep -q '^\[Install\]$' "${timer}"
  else
    grep -q '^\[Install\]$' "${unit}"
  fi
done

for unit in \
  "${ROOT_DIR}/infra/ops/systemd/choruz-api-gateway.service" \
  "${ROOT_DIR}/infra/ops/systemd/choruz-pipeline.service" \
  "${ROOT_DIR}/infra/ops/systemd/choruz-web-app.service"; do
  grep -q '^User=choruz$' "${unit}"
  grep -q '^Group=choruz$' "${unit}"
  grep -q '^LogsDirectory=choruz$' "${unit}"
done

echo "ops templates look valid"
