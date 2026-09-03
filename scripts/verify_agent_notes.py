#!/usr/bin/env python3

"""Check every Agent Note under .agents/notes against .agents/notes/README.md.

Structure: {lifecycle}/{class}/yyyy-mm-dd-topic.md with a closed set of
lifecycles and classes, and no centralized index. Format: the header block,
the per-lifecycle body skeleton, and a mandatory "Alternatives considered"
section (a pre-format note may carry the grandfather comment instead).

Prints one `structure:` or `format:` line per violation and exits 1 when any
were found.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

LIFECYCLES = ("proposed", "implemented", "rejected")
ARCHIVED = "archived"
CLASSES = ("feature", "bug-fix", "simplification", "architecture", "process", "testing")
ROOT_FILES = {"README.md"}
FOLDER_FILES = {"AGENTS.md", "CLAUDE.md"}

FORMAT_ADOPTED = "2026-09-03"
GRANDFATHER = "<!-- agent-note-format: alternatives-not-recorded (pre-format Agent Note) -->"
FILENAME = re.compile(r"^\d{4}-\d{2}-\d{2}-.+\.md$")
TITLE = re.compile(r"^# Agent Note: \S")
STATUS = {
    "proposed": re.compile(r"^Status: proposed$"),
    "implemented": re.compile(r"^Status: implemented$"),
    "rejected": re.compile(r"^Status: rejected — .+$"),
    ARCHIVED: re.compile(r"^Status: implemented$"),
}
ARCHIVED_LINE = re.compile(r"^Archived: \d{4}-\d{2}-\d{2}$")
REQUIRED = {
    "proposed": ("## Proposal", "## Acceptance criteria", "## Risks"),
    "implemented": ("## Decision", "## Consequences"),
    "rejected": ("## Proposal",),
}
BANNED_IMPLEMENTED = re.compile(r"^## (?:Proposal|Plan|Migration plan|Acceptance criteria)\b", re.IGNORECASE)
ALTERNATIVES = "## Alternatives considered"


def structure_violations(notes_dir: Path) -> tuple[list[str], list[tuple[str, Path]]]:
    """Return (violations, [(lifecycle, note path)]) for the tree."""
    violations: list[str] = []
    notes: list[tuple[str, Path]] = []
    for path in sorted(notes_dir.rglob("*")):
        if path.is_dir():
            continue
        rel = path.relative_to(notes_dir)
        parts = rel.parts
        if len(parts) == 1:
            if parts[0] not in ROOT_FILES:
                violations.append(f"structure: {rel} — only README.md may sit at the notes root (no INDEX.md, no loose notes)")
            continue
        lifecycle = parts[0]
        if lifecycle not in LIFECYCLES and lifecycle != ARCHIVED:
            violations.append(f"structure: {rel} — unknown lifecycle folder (allowed: {', '.join(LIFECYCLES)}, plus {ARCHIVED}/)")
            continue
        if len(parts) == 2:
            if parts[1] not in FOLDER_FILES:
                violations.append(f"structure: {rel} — expected {{lifecycle}}/{{class}}/file.md (got depth 2)")
            continue
        if len(parts) != 3:
            violations.append(f"structure: {rel} — expected {{lifecycle}}/{{class}}/file.md (got depth {len(parts)})")
            continue
        cls, name = parts[1], parts[2]
        if cls not in CLASSES:
            violations.append(f"structure: {rel} — unknown class folder \"{cls}\" (allowed: {', '.join(CLASSES)})")
            continue
        if not FILENAME.match(name):
            violations.append(f"structure: {rel} — filename must be yyyy-mm-dd-topic.md")
            continue
        notes.append((lifecycle, path))
    return violations, notes


def _strip_fences(text: str) -> list[str]:
    out: list[str] = []
    fenced = False
    for line in text.splitlines():
        if line.startswith("```"):
            fenced = not fenced
            out.append("")
            continue
        out.append("" if fenced else line)
    return out


def format_violations(lifecycle: str, path: Path, text: str, rel: str) -> list[str]:
    """Return the format violations of one note."""
    errors: list[str] = []
    lines = _strip_fences(text)
    lines += [""] * (5 - len(lines))
    if not TITLE.match(lines[0]):
        errors.append("line 1 must be `# Agent Note: <title>`")
    if lines[1] != "":
        errors.append("line 2 must be blank")
    if not STATUS[lifecycle].match(lines[2]):
        errors.append(f"line 3 must match the {lifecycle} status grammar ({STATUS[lifecycle].pattern})")
    if lifecycle == ARCHIVED:
        if not ARCHIVED_LINE.match(lines[3]):
            errors.append("line 4 must be `Archived: YYYY-MM-DD`")
        elif lines[4] != "":
            errors.append("line 5 must be blank")
    elif lines[3] != "":
        errors.append("line 4 must be blank")
    if sum(1 for line in lines if line.startswith("Status:")) != 1:
        errors.append("the line-3 `Status:` line must be the only one in the file")
    if lifecycle == ARCHIVED:
        return [f"format: {rel} — {e}" for e in errors]

    h2 = [line for line in lines if line.startswith("## ")]
    if not h2 or h2[0] != "## Problem":
        errors.append("the first section must be `## Problem`")
    for required in REQUIRED[lifecycle]:
        if required not in h2:
            errors.append(f"missing the required `{required}` section")
    if lifecycle == "implemented":
        for heading in h2:
            if BANNED_IMPLEMENTED.match(heading):
                errors.append(f"`{heading}` is a proposal-era heading; an implemented Agent Note states what is (fold it into Decision/Consequences/Testing)")
    has_alternatives = ALTERNATIVES in h2
    has_grandfather = any(line.strip() == GRANDFATHER for line in lines)
    if has_alternatives and has_grandfather:
        errors.append("carries both `## Alternatives considered` and the grandfather comment — drop the comment")
    elif not has_alternatives and not has_grandfather:
        errors.append("missing `## Alternatives considered` (a pre-format Agent Note whose alternatives are not reconstructible carries the grandfather comment instead — see .agents/notes/README.md § The file format)")
    elif has_grandfather and path.name[:10] >= FORMAT_ADOPTED:
        errors.append(f"the grandfather comment is only valid for Agent Notes dated before {FORMAT_ADOPTED}")
    return [f"format: {rel} — {e}" for e in errors]


def check(notes_dir: Path) -> tuple[list[str], int]:
    """Return (violations, notes checked)."""
    violations, notes = structure_violations(notes_dir)
    for lifecycle, path in notes:
        rel = path.relative_to(notes_dir).as_posix()
        violations.extend(format_violations(lifecycle, path, path.read_text(encoding="utf-8"), rel))
    return violations, len(notes)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--notes", type=Path, default=Path(__file__).resolve().parents[1] / ".agents" / "notes")
    args = parser.parse_args()
    violations, count = check(args.notes)
    if violations:
        print("verify_agent_notes: violations found:")
        for v in violations:
            print(f"  {v}")
        sys.exit(1)
    print(f"verify_agent_notes: {count} Agent Note(s) checked, all conform to .agents/notes/README.md.")


if __name__ == "__main__":
    main()
