#!/usr/bin/env bash
set -euo pipefail
if [[ $# -eq 0 ]]; then set -- package; fi
exec python3 "$(dirname "${BASH_SOURCE[0]}")/../release.py" "$@"
