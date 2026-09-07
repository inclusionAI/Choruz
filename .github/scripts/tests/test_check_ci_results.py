import importlib.util
import json
import os
import re
from pathlib import Path
import subprocess
import sys
import unittest


SCRIPT = Path(__file__).parents[1] / "check_ci_results.py"
SPEC = importlib.util.spec_from_file_location("check_ci_results", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class CheckCiResultsTests(unittest.TestCase):
    def test_browser_failure_artifacts_survive_a_passing_retry(self) -> None:
        root = SCRIPT.parents[2]
        config = (root / "apps/web/playwright.config.ts").read_text()
        self.assertIn('"retain-on-first-failure"', config)
        workflow = (root / ".github/workflows/ci.yml").read_text()
        uploads = workflow.split("      - name: Upload Playwright report\n")[1:]
        self.assertEqual(len(uploads), 2)
        for upload in uploads:
            condition = upload.splitlines()[0]
            self.assertIn("!cancelled()", condition)
            self.assertIn("failure() || hashFiles('apps/web/test-results/**/trace.zip') != ''", condition)

    def test_workflow_exports_every_consumed_change_output(self) -> None:
        workflow = (SCRIPT.parents[2] / ".github/workflows/ci.yml").read_text()
        outputs = workflow.split("    outputs:\n", 1)[1].split("    steps:\n", 1)[0]
        exported = set(re.findall(r"^      (\w+):", outputs, re.MULTILINE))
        consumed = set(re.findall(r"needs\.changes\.outputs\.(\w+)", workflow))
        self.assertEqual(consumed - exported, set())

    def test_bridge_gate_runs_the_existing_test_command(self) -> None:
        workflow = (SCRIPT.parents[2] / ".github/workflows/ci.yml").read_text()
        bridge = workflow.split("      - name: Bridge build", 1)[1].split("      - name:", 1)[0]
        self.assertIn("pnpm --dir services/choruz-bridge build", bridge)
        self.assertIn("pnpm --dir services/choruz-bridge test", bridge)

    def test_cli_requires_successful_change_detection_even_without_selected_jobs(self) -> None:
        for result in ("failure", "cancelled", "skipped", None, "success"):
            with self.subTest(result=result):
                needs = {} if result is None else {"changes": {"result": result}}
                run = subprocess.run(
                    [sys.executable, str(SCRIPT)],
                    env={**os.environ, "NEEDS": json.dumps(needs), "REQUIRED_NEEDS": ""},
                    capture_output=True,
                    text=True,
                    check=False,
                )
                self.assertEqual(run.returncode == 0, result == "success", run.stdout + run.stderr)

    def test_accepts_only_successful_dependencies(self) -> None:
        self.assertEqual(MODULE.failures({"validate": {"result": "success"}}), [])

    def test_rejects_failed_skipped_cancelled_and_missing_results(self) -> None:
        needs = {
            "cancelled": {"result": "cancelled"},
            "failed": {"result": "failure"},
            "missing": {},
            "skipped": {"result": "skipped"},
        }
        self.assertEqual(
            MODULE.failures(needs),
            [
                ("cancelled", "cancelled"),
                ("failed", "failure"),
                ("missing", "missing"),
                ("skipped", "skipped"),
            ],
        )

    def test_parses_the_required_list(self) -> None:
        self.assertEqual(
            MODULE.applicable_needs("rust-lint, e2e,,perf"),
            ("rust-lint", "e2e", "perf"),
        )

    def test_empty_list_requires_nothing(self) -> None:
        # A documentation-only change lists no job; the gate passes.
        self.assertEqual(MODULE.applicable_needs(""), ())
        self.assertEqual(MODULE.failures({}), [])

    def test_only_listed_jobs_count(self) -> None:
        needs = {"e2e": {"result": "success"}, "perf": {"result": "skipped"}}
        required = MODULE.applicable_needs("e2e")
        self.assertEqual(MODULE.failures({name: needs.get(name, {}) for name in required}), [])


if __name__ == "__main__":
    unittest.main()
