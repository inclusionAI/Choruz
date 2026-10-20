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
        notes = checks.split("      - name: Agent Notes\n", 1)[1].split("      - name:", 1)[0]
        self.assertIn("python3 -m unittest discover scripts/tests -p test_verify_agent_notes.py", notes)
        self.assertNotIn("python3 -m unittest discover scripts/tests\n", notes)

    def test_bridge_mapping_uses_a_required_owned_database_gate(self):
        workflow = (Path(__file__).resolve().parents[3] / ".github/workflows/ci.yml").read_text()
        job = workflow.split("  db-smoke:\n", 1)[1].split("\n  e2e:\n", 1)[0]
        step = job.split("      - name: API smoke\n", 1)[1].split("      - name:", 1)[0]
        self.assertNotIn("if:", step)
        self.assertNotIn("continue-on-error", step)
        self.assertIn("run: pnpm api:smoke", step)
        runner = (Path(__file__).resolve().parents[3] / "infra/host/api_smoke.sh").read_text()
        self.assertIn('export PGUSER="${CHORUZ_PG_USER}" PGDATABASE="${CHORUZ_PG_DB}"', runner)
        self.assertIn("unset CHORUZ_TEST_DATABASE_URL", runner)
        self.assertIn('pnpm --dir "${ROOT_DIR}/services/choruz-bridge" exec tsx --test tests/mapping-store.pg.ts', runner)


if __name__ == "__main__":
    unittest.main()
