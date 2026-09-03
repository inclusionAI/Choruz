#!/usr/bin/env bash
# Show what CI will run for the current branch, and what to run locally first.
#
#   .agents/skills/choruz-pr/pr-plan.sh            # compare against origin/main
#   .agents/skills/choruz-pr/pr-plan.sh <base>     # compare against another ref
#
# Uses the same selectors as .github/workflows/ci.yml, so the answer matches
# the pull request's "Detect changes" job.
set -euo pipefail

ROOT_DIR="$(git rev-parse --show-toplevel)"
BASE="${1:-origin/main}"
cd "$ROOT_DIR"

git fetch -q origin main 2>/dev/null || true
merge_base="$(git merge-base "$BASE" HEAD)"
changed="$( { git diff --name-only "$merge_base" HEAD; git diff --name-only HEAD; git ls-files --others --exclude-standard; } | sort -u)"

if [ -z "$changed" ]; then
  echo "No changes against $BASE."
  exit 0
fi

echo "Changed files (against $BASE):"
printf '  %s\n' $changed
echo

e2e="$(printf '%s\n' "$changed" | python3 .github/scripts/select_e2e_specs.py)"
rust="$(printf '%s\n' "$changed" | python3 .github/scripts/select_rust_packages.py)"
get() { printf '%s\n' "$1" | sed -n "s/^$2=//p"; }

docs_only="$(get "$e2e" docs_only)"
specs="$(get "$e2e" specs)"
shards="$(get "$e2e" shard_count)"
vitest="$(get "$e2e" vitest)"
vitest_files="$(get "$e2e" vitest_files)"
cargo_args="$(get "$rust" cargo_args)"
rust_count="$(get "$rust" count)"

matches() { printf '%s\n' "$changed" | grep -Eq "$1"; }

echo "CI will run:"
if [ "$docs_only" = true ]; then
  echo "  nothing: documentation only (Detect changes + aggregator, ~30 s)"
  exit 0
fi
echo "  Static checks: security scan (always)$(matches '^\.github/(workflows|actions|scripts)/' && printf ', CI policy tests')$(matches '^services/choruz-bridge/' && printf ', bridge build')$(matches '^services/remote-control-gateway/' && printf ', gateway checks')$(matches '^infra/ops/' && printf ', ops lint')$(matches '^(scripts/|infra/host/)' && printf ', host lifecycle tests')$(matches '^(\.agents/notes/|scripts/verify_agent_notes\.py)' && printf ', Agent Notes gate')"
if [ "$rust_count" != 0 ] && [ -n "$cargo_args" ]; then
  echo "  Rust lint + tests: cargo ... $cargo_args"
fi
if matches '^(apps/web/|package\.json|pnpm-lock\.yaml|pnpm-workspace\.yaml|\.github/(workflows|actions|scripts)/)'; then
  case "$vitest" in
    all) echo "  Web: unit tests (all), typecheck, build" ;;
    related) echo "  Web: unit tests related to: $vitest_files; typecheck, build" ;;
    *) echo "  Web: typecheck, build (no unit tests select)" ;;
  esac
fi
if matches '^(migrations/|infra/host/|scripts/historical-migrations\.sha256|crates/|services/|apps/choruz-|Cargo\.|\.cargo/|rust-toolchain|infra/host/setup_test_database\.sh|\.github/(workflows|actions|scripts)/)'; then
  echo "  DB and API smoke"
fi
e2e_paths='^(apps/web/|package\.json|pnpm-lock\.yaml|pnpm-workspace\.yaml|\.github/(workflows|actions|scripts)/|infra/host/|migrations/|crates/|services/|apps/choruz-|Cargo\.|\.cargo/|rust-toolchain|infra/host/setup_test_database\.sh)'
if [ -n "$specs" ] && matches "$e2e_paths"; then
  echo "  Web E2E ($shards shard(s)): $specs"
fi
if matches '^\.github/(workflows|actions|scripts)/'; then
  echo "  Web E2E full suite (workflow change)"
fi
echo
echo "Run locally before opening the PR:"
if matches '^(\.agents/notes/|scripts/verify_agent_notes\.py)'; then
  echo "  python3 scripts/verify_agent_notes.py"
fi
if [ "$rust_count" != 0 ] && [ -n "$cargo_args" ]; then
  echo "  cargo fmt --check && cargo clippy $cargo_args --all-targets -- -D warnings"
  echo "  cargo test $cargo_args"
fi
if matches '^(apps/web/|package\.json|pnpm-lock\.yaml)'; then
  case "$vitest" in
    all) echo "  pnpm web:test" ;;
    related) echo "  pnpm --dir apps/web exec vitest related --run $vitest_files" ;;
  esac
  echo "  pnpm web:check && pnpm web:build"
fi
if [ -n "$specs" ] && matches "$e2e_paths"; then
  echo "  bash infra/host/web_e2e.sh $specs"
fi
echo
echo "Labels: add 'database' or 'security' when the PR type is api/database or security/auth (runs the full e2e suite); 'ci-full' to ask for it on any PR."
