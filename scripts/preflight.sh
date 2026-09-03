#!/usr/bin/env bash
#
# Local pre-push/pre-PR checks for Choruz.
#
# Usage:
#   ./scripts/preflight.sh              # standard checks
#   ./scripts/preflight.sh --quick      # dependency lock + Rust formatting only
#   ./scripts/preflight.sh --standard   # default: quick + core Rust/web checks
#   ./scripts/preflight.sh --full       # standard + slower CI-like smoke checks
#
# Environment:
#   CHORUZ_PREFLIGHT_MODE=quick|standard|full
#   CHORUZ_SKIP_PREFLIGHT=1              # skip, intended for emergencies only

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

MODE="${CHORUZ_PREFLIGHT_MODE:-standard}"

for arg in "$@"; do
  case "$arg" in
    --quick) MODE="quick" ;;
    --standard) MODE="standard" ;;
    --full) MODE="full" ;;
    --help|-h)
      sed -n '1,19p' "$0"
      exit 0
      ;;
    *)
      echo "unknown option: $arg" >&2
      exit 2
      ;;
  esac
done

if [ "${CHORUZ_SKIP_PREFLIGHT:-0}" = "1" ]; then
  echo "CHORUZ_SKIP_PREFLIGHT=1; skipping local preflight."
  exit 0
fi

case "$MODE" in
  quick|standard|full) ;;
  *)
    echo "unknown CHORUZ_PREFLIGHT_MODE: $MODE" >&2
    echo "expected one of: quick, standard, full" >&2
    exit 2
    ;;
esac

run_step() {
  echo ""
  echo "==> $*"
  "$@"
}

echo "Choruz local preflight"
echo "mode: $MODE"
echo "repo: $REPO_ROOT"
echo "branch: $(git branch --show-current 2>/dev/null || echo unknown)"

run_step pnpm install --frozen-lockfile
run_step cargo fmt --check
run_step python3 -m unittest scripts/test_choruz_ui_bridge.py

if [ "$MODE" = "quick" ]; then
  echo ""
  echo "Quick preflight passed."
  exit 0
fi

run_step cargo clippy --workspace --all-targets -- -D warnings
source ./infra/host/setup_test_database.sh
run_step cargo test --workspace
run_step pnpm web:check
run_step pnpm web:test

if [ "$MODE" = "standard" ]; then
  echo ""
  echo "Standard preflight passed."
  exit 0
fi

run_step pnpm db:migration:smoke
run_step pnpm security:check
run_step pnpm web:build
run_step pnpm web:e2e
run_step pnpm api:smoke
run_step pnpm perf:ws:smoke
run_step pnpm ops:check
run_step pnpm release:package

echo ""
echo "Full preflight passed."
