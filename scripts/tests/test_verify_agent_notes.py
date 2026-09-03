import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from verify_agent_notes import GRANDFATHER, check  # noqa: E402

IMPLEMENTED = """# Agent Note: Pick a thing

Status: implemented

## Problem

Why.

## Decision

What.

## Alternatives considered

- **Other thing** — rejected because.

## Consequences

Cost and gain.
"""

PROPOSED = """# Agent Note: Do a thing

Status: proposed

## Problem

Why.

## Proposal

How.

## Alternatives considered

- **Nothing** — rejected.

## Acceptance criteria

- Done when.

## Risks

- Might.
"""


class NotesTree:
    def __init__(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        (self.root / "README.md").write_text("# Agent Notes\n")

    def write(self, rel: str, text: str) -> None:
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def violations(self) -> list[str]:
        return check(self.root)[0]


class StructureTest(unittest.TestCase):
    def setUp(self):
        self.tree = NotesTree()

    def test_conforming_notes_pass(self):
        self.tree.write("implemented/architecture/2026-08-18-thing.md", IMPLEMENTED)
        self.tree.write("proposed/feature/2026-09-04-thing.md", PROPOSED)
        self.tree.write("implemented/AGENTS.md", "# Implemented\n")
        self.assertEqual(self.tree.violations(), [])
        self.assertEqual(check(self.tree.root)[1], 2)

    def test_rejects_index_unknown_lifecycle_class_and_filename(self):
        self.tree.write("INDEX.md", "# no\n")
        self.tree.write("accepted/feature/2026-08-18-x.md", IMPLEMENTED)
        self.tree.write("implemented/misc/2026-08-18-x.md", IMPLEMENTED)
        self.tree.write("implemented/feature/thing.md", IMPLEMENTED)
        self.tree.write("implemented/feature/deeper/2026-08-18-x.md", IMPLEMENTED)
        messages = "\n".join(self.tree.violations())
        self.assertIn("INDEX.md — only README.md", messages)
        self.assertIn("unknown lifecycle folder", messages)
        self.assertIn('unknown class folder "misc"', messages)
        self.assertIn("filename must be yyyy-mm-dd-topic.md", messages)
        self.assertIn("got depth 4", messages)


class FormatTest(unittest.TestCase):
    def setUp(self):
        self.tree = NotesTree()

    def one(self, rel: str, text: str) -> str:
        """Violations of a single note checked in a fresh tree."""
        tree = NotesTree()
        tree.write(rel, text)
        return "\n".join(tree.violations())

    def test_status_must_agree_with_folder(self):
        messages = self.one("proposed/feature/2026-09-04-x.md", IMPLEMENTED)
        self.assertIn("line 3 must match the proposed status grammar", messages)

    def test_rejected_needs_a_reason(self):
        text = PROPOSED.replace("Status: proposed", "Status: rejected")
        self.assertIn("rejected status grammar", self.one("rejected/feature/2026-09-04-x.md", text))
        ok = PROPOSED.replace("Status: proposed", "Status: rejected — costs more than it saves")
        self.assertEqual(self.one("rejected/feature/2026-09-05-y.md", ok), "")

    def test_problem_first_and_required_sections(self):
        text = IMPLEMENTED.replace("## Problem\n\nWhy.\n\n", "")
        messages = self.one("implemented/feature/2026-09-04-x.md", text)
        self.assertIn("first section must be `## Problem`", messages)
        text = IMPLEMENTED.replace("## Consequences", "## Outcome")
        self.assertIn("missing the required `## Consequences`", self.one("implemented/feature/2026-09-04-y.md", text))

    def test_implemented_rejects_proposal_era_headings(self):
        text = IMPLEMENTED.replace("## Consequences", "## Acceptance criteria\n\nx\n\n## Consequences")
        self.assertIn("proposal-era heading", self.one("implemented/feature/2026-09-04-x.md", text))

    def test_alternatives_are_mandatory_with_a_dated_grandfather(self):
        without = IMPLEMENTED.replace("## Alternatives considered\n\n- **Other thing** — rejected because.\n\n", "")
        self.assertIn("missing `## Alternatives considered`", self.one("implemented/feature/2026-09-04-x.md", without))
        grand = without.replace("## Consequences", GRANDFATHER + "\n\n## Consequences")
        self.assertEqual(self.one("implemented/feature/2026-08-18-old.md", grand), "")
        self.assertIn("only valid for Agent Notes dated before", self.one("implemented/feature/2026-09-04-new.md", grand))
        both = IMPLEMENTED.replace("## Consequences", GRANDFATHER + "\n\n## Consequences")
        self.assertIn("drop the comment", self.one("implemented/feature/2026-08-18-both.md", both))

    def test_headings_inside_code_fences_do_not_count(self):
        text = IMPLEMENTED.replace("## Decision", "## Decision\n\n```markdown\n## Proposal\nStatus: proposed\n```")
        self.assertEqual(self.one("implemented/feature/2026-09-04-x.md", text), "")

    def test_archived_checks_only_the_header(self):
        body = "# Agent Note: Old\n\nStatus: implemented\nArchived: 2026-09-03\n\nAnything at all.\n"
        self.assertEqual(self.one("archived/feature/2026-05-29-old.md", body), "")
        no_date = "# Agent Note: Old\n\nStatus: implemented\n\n## Problem\n"
        self.assertIn("line 4 must be `Archived: YYYY-MM-DD`", self.one("archived/feature/2026-05-30-x.md", no_date))


if __name__ == "__main__":
    unittest.main()
