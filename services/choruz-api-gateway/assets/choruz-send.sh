#!/bin/bash
# .choruz/send — Maildir-style outbox command writer
# Usage: .choruz/send '{"type":"send","group":"team","content":"hello"}'
#
# Atomically writes the command to .choruz-outbox/new/ using the
# write-to-tmp-then-rename pattern. The platform's outbox watcher
# picks up files from new/, processes them, and deletes them.
#
# This prevents data loss when multiple commands are written rapidly —
# each command gets its own file, no overwrites possible.

set -euo pipefail

if [ $# -eq 0 ]; then
  echo "Usage: .choruz/send '{\"type\":\"send\",\"group\":\"...\",\"content\":\"...\"}'" >&2
  exit 1
fi

# Resolve outbox directory relative to the script's location unless the
# pipeline injected an absolute bound outbox. This keeps delivery stable when
# an agent changes cwd to inspect a project and still calls the helper.
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORKSPACE_DIR="$(dirname "$SCRIPT_DIR")"
if [[ "${CHORUZ_OUTBOX_DIR:-}" = /* ]]; then
  OUTBOX_DIR="$CHORUZ_OUTBOX_DIR"
else
  OUTBOX_DIR="$WORKSPACE_DIR/.choruz-outbox"
fi

mkdir -p "$OUTBOX_DIR/tmp" "$OUTBOX_DIR/new"

# Write to tmp/ first (not visible to consumer).
# macOS mktemp requires XXXXXX at the end of template (no suffix allowed).
TMPFILE=$(mktemp "$OUTBOX_DIR/tmp/cmd-XXXXXX")
echo "$1" > "$TMPFILE"

# Assign a monotonic sequence number at publish time. The outbox watcher sorts
# filenames lexicographically, so this preserves the order of rapid multi-step
# workflows such as provision_agent, provision_agent, create_group.
LOCKDIR="$OUTBOX_DIR/.lock"
until mkdir "$LOCKDIR" 2>/dev/null; do
  sleep 0.01
done
cleanup_lock() {
  rmdir "$LOCKDIR" 2>/dev/null || true
}
trap cleanup_lock EXIT

SEQ_FILE="$OUTBOX_DIR/.seq"
SEQ="0"
if [ -f "$SEQ_FILE" ]; then
  read -r SEQ < "$SEQ_FILE" || SEQ="0"
fi
SEQ_NUM=$((10#$SEQ + 1))
SEQ_PADDED=$(printf "%020d" "$SEQ_NUM")
printf "%s\n" "$SEQ_PADDED" > "$SEQ_FILE"

# Atomic rename to new/ with .json extension (visible to consumer).
RANDOM_PART="$(basename "$TMPFILE")"
mv "$TMPFILE" "$OUTBOX_DIR/new/cmd-${SEQ_PADDED}-${RANDOM_PART}.json"
