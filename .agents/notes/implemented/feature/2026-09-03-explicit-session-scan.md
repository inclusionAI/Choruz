# Agent Note: Session import scans on request and selects nothing by default

Status: implemented

## Problem

The Import Sessions modal (`apps/web/components/agents/import-workspace-sessions-modal.tsx`) scanned the chosen folder and every subfolder 300 ms after each path keystroke or Harness toggle, and every scan pre-selected all of its results. Typing a path to a large tree fired several full scans of the host's session stores through the Remote Control bridge before the user had finished, and the import button then offered the whole tree in one click; a hasty confirm imported dozens of Agents into the active Company.

## Decision

A `Scan` button starts the scan for the current folder and Harness set; changing either clears the previous results (status `Ready to scan`) and nothing scans until the button is pressed again. Results start with no session selected; `Select all` and the row checkboxes opt in, and the import button stays disabled at zero.

## Alternatives considered

- **Keep the automatic scan but debounce longer**: rejected. A longer delay still scans on every settled edit, and the user still cannot tell when a scan will start.
- **Auto-scan, select nothing**: rejected. Halves the problem; the surprise scans over a bridge stay.
- **Pre-select only the newest N sessions**: rejected. Any default picks Agents the user did not ask for; an explicit selection is cheap on a list that is already sorted newest first and filterable.

## Consequences

- Import needs two clicks more than before: `Scan`, then a selection.
- `apps/web/tests/e2e/workspace-session-import.spec.ts` asserts no scan request before `Scan`, a fresh `0 selected` after each scan, and a disabled import button at zero.
