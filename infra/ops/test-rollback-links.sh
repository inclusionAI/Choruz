#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck disable=SC1090
source "${ROOT_DIR}/infra/ops/bin/rollback.sh"

TEMP_DIR="$(mktemp -d)"
cleanup() {
  rm -rf "${TEMP_DIR}"
}
trap cleanup EXIT

KNOWN_GOOD_DIR="${TEMP_DIR}/known-good"
TARGET_DIR="${TEMP_DIR}/target"
mkdir -p "${KNOWN_GOOD_DIR}" "${TARGET_DIR}"
CURRENT_LINK="${TEMP_DIR}/current"
ln -s "${KNOWN_GOOD_DIR}" "${CURRENT_LINK}"

set_current_target "${TARGET_DIR}"

[[ "$(readlink "${CURRENT_LINK}")" == "${TARGET_DIR}" ]] || {
  echo "rollback target link was not replaced" >&2
  exit 1
}
[[ ! -e "${KNOWN_GOOD_DIR}/current" && ! -e "${KNOWN_GOOD_DIR}/current.next" ]] || {
  echo "rollback link replacement dereferenced the active release directory" >&2
  exit 1
}

echo "rollback link replacement is exact and non-dereferencing"
