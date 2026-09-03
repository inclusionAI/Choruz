#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd -P)"
runner="${repo_root}/infra/host/smoke/real-harness-platform-smoke.ts"

if [[ "${CHORUZ_REAL_HARNESS_SMOKE:-0}" != "1" ]]; then
  printf '%s\n' 'Refusing to invoke real Harnesses without CHORUZ_REAL_HARNESS_SMOKE=1.' >&2
  printf '%s\n' 'See docs/testing/real-harness-platform-smoke.md for prerequisites.' >&2
  exit 2
fi

exec node --disable-warning=ExperimentalWarning --disable-warning=MODULE_TYPELESS_PACKAGE_JSON \
  --experimental-strip-types "${runner}" "$@"
