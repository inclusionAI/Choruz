from pathlib import Path
import unittest


class ArchitectureGateTests(unittest.TestCase):
    def test_every_non_documentation_change_runs_boundaries_in_required_checks(self):
        workflow = (Path(__file__).resolve().parents[3] / ".github/workflows/ci.yml").read_text()
        checks = workflow.split("  checks:\n", 1)[1].split("\n  db-smoke:\n", 1)[0]
        self.assertIn("needs.changes.outputs.docs_only != 'true'", checks)
        step = checks.split("      - name: Architecture boundaries\n", 1)[1].split("      - name:", 1)[0]
        self.assertNotIn("if:", step)
        self.assertNotIn("continue-on-error", step)
        self.assertIn("python3 -m unittest discover scripts/tests -p test_check_architecture.py", step)
        self.assertIn("python3 scripts/check_architecture.py", step)
        self.assertIn('[ "$DOCS_ONLY" != true ] && add checks', workflow)


if __name__ == "__main__":
    unittest.main()
