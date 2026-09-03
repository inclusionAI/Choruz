#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RESTORE="${ROOT_DIR}/infra/ops/bin/restore.sh"
RELEASE="${ROOT_DIR}/infra/ops/bin/release.sh"
ROLLBACK="${ROOT_DIR}/infra/ops/bin/rollback.sh"
LEGACY_PRODUCT="e""chat"

# These contracts make the selector policy executable: only the exact Choruz
# service labels and host PID filenames are allowed. A wildcard could stop an
# unrelated process (for example, another launchd agent in the same directory).
for expected in \
  'choruz-api-gateway' \
  'choruz-pipeline' \
  'choruz-web-app' \
  'com.choruz.api-gateway' \
  'com.choruz.pipeline' \
  'com.choruz.web-app' \
  'PG_DATA_DIR="${DATA_DIR}/postgres"' \
  '127.0.0.1|localhost|::1)' \
  '"${CHORUZ_PG_CTL_BIN}" -D "${PG_DATA_DIR}" stop -m fast' \
  'write_pid postgres "${HOST_POSTGRES_PID}"'; do
  grep -Fq "${expected}" "${RESTORE}" || {
    echo "missing exact Choruz selector: ${expected}" >&2
    exit 1
  }
done

if grep -Eq "com\\.choruz\\.\\*|${LEGACY_PRODUCT}-" "${RESTORE}" "${RELEASE}" "${ROLLBACK}"; then
  echo "operations selector contains a wildcard or legacy service name" >&2
  exit 1
fi
if grep -En 'RUNTIME_DIR.*(\*.*\.pid|-i?name([[:space:]]|=).*\*.*\.pid)' "${RESTORE}" "${RELEASE}" "${ROLLBACK}"; then
  echo "operations selector contains a runtime PID wildcard" >&2
  exit 1
fi

grep -Fq 'for service in choruz-api-gateway pipeline web-app; do' "${ROLLBACK}" || {
  echo "rollback must restart every managed Choruz service" >&2
  exit 1
}
awk '/^SERVICES=\(/, /^\)/' "${RELEASE}" | grep -Fxq '  choruz-api-gateway' && \
  awk '/^SERVICES=\(/, /^\)/' "${RELEASE}" | grep -Fxq '  pipeline' && \
  awk '/^SERVICES=\(/, /^\)/' "${RELEASE}" | grep -Fxq '  web-app' && \
  ! awk '/^SERVICES=\(/, /^\)/' "${RELEASE}" | grep -Fq 'choruz-pipeline' || {
  echo "release must expand pipeline to the exact service identity once" >&2
  exit 1
}
grep -Fq 'gui/$(id -u)/com.choruz.${service}' "${RELEASE}" && \
  grep -Fq 'gui/$(id -u)/com.choruz.${service}' "${ROLLBACK}" || {
  echo "managed LaunchAgents must restart in the user launchd domain" >&2
  exit 1
}
if grep -Eq '(^|[[:space:];])(/bin/)?kill([[:space:]]|$)' "${RESTORE}"; then
  echo "restore must not invoke kill directly; it must use exact service controls" >&2
  exit 1
fi
grep -Fq 'drop_database_strict "${CHORUZ_PG_DB}"' "${RESTORE}" && \
  grep -Fq 'ensure_database_strict "${CHORUZ_PG_DB}"' "${RESTORE}" || {
  echo "restore must fail if the target database cannot be recreated" >&2
  exit 1
}
if ! grep -Fq 'readable, non-empty database.sql' "${RESTORE}" \
  || ! grep -Fq 'configured Choruz database' "${RESTORE}" \
  || ! grep -Fq 'pg_exec --single-transaction' "${RESTORE}"; then
  echo "restore must validate the backup before destructive operations" >&2
  exit 1
fi
database_validation_line="$(grep -nF 'if [[ ! -s "${BACKUP_PATH}/database.sql"' "${RESTORE}" | head -n 1 | cut -d: -f1)"
metadata_validation_line="$(grep -nF "if ! grep -Fqx '  \"product\": \"choruz\",'" "${RESTORE}" | head -n 1 | cut -d: -f1)"
drop_database_line="$(grep -nF 'drop_database_strict "${CHORUZ_PG_DB}"' "${RESTORE}" | head -n 1 | cut -d: -f1)"
restore_transaction_line="$(grep -nF 'pg_exec --single-transaction' "${RESTORE}" | head -n 1 | cut -d: -f1)"
if (( database_validation_line >= drop_database_line \
  || metadata_validation_line >= drop_database_line \
  || database_validation_line >= restore_transaction_line \
  || metadata_validation_line >= restore_transaction_line )); then
  echo "restore validation must precede every destructive database operation" >&2
  exit 1
fi
for expected in 'preflight_target()' 'services_healthy()' 'KNOWN_GOOD=' 'rollback restart or health check failed' 'rm -f "${CURRENT_LINK}"'; do
  grep -Fq "${expected}" "${ROLLBACK}" || {
    echo "rollback must restore the known-good target after a failed restart" >&2
    exit 1
  }
done

echo "operations selectors are exact and Choruz-only"
