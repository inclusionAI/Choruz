import importlib.util
import json
import os
import re
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import urllib.error
from unittest import mock


SCRIPT = Path(__file__).parents[1] / "check_ci_results.py"
SPEC = importlib.util.spec_from_file_location("check_ci_results", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class CheckCiResultsTests(unittest.TestCase):
    def test_embedded_postgres_download_has_a_step_scoped_read_only_token(self):
        workflow = (SCRIPT.parents[2] / ".github/workflows/ci.yml").read_text()
        rust = workflow.split("  rust-test:\n", 1)[1].split("\n  web:\n", 1)[0]
        step = rust.split("      - name: Rust tests\n", 1)[1].split("      - name:", 1)[0]
        self.assertIn("GITHUB_TOKEN: ${{ github.token }}", step)
        self.assertIn("permissions:\n  contents: read", workflow)
        self.assertNotIn("contents: write", rust)
        self.assertNotIn("GITHUB_TOKEN", workflow.split("\njobs:", 1)[0])

    def test_routing_portability_is_checked_separately_from_default_features(self) -> None:
        workflow = (SCRIPT.parents[2] / ".github/workflows/ci.yml").read_text()
        rust_job = workflow.split("  rust-test:\n", 1)[1].split("\n  web:\n", 1)[0]
        self.assertIn("cargo test ${{ needs.changes.outputs.cargo_args }}", rust_job)
        self.assertIn("cargo test -p choruz-router -p choruz-session -p choruz-store --no-default-features", rust_job)
        self.assertIn("cargo run -p choruz-router --no-default-features --example route_message", rust_job)
        self.assertIn("cargo tree -p choruz-router --no-default-features --edges normal", rust_job)
        self.assertIn("exit 1", rust_job)

    def test_server_and_schema_pull_requests_require_the_load_smoke(self) -> None:
        workflow = (SCRIPT.parents[2] / ".github/workflows/ci.yml").read_text()
        step = workflow.split("        id: required\n", 1)[1].split("\n\n", 1)[0]
        script = step.split("        run: |\n", 1)[1]
        perf_job = workflow.split("  perf:\n", 1)[1].split("\n\n", 1)[0]
        self.assertIn("needs.changes.outputs.db == 'true'", perf_job)

        def required(**changes: str) -> list[str]:
            env = {
                "PATH": os.environ["PATH"],
                "GITHUB_OUTPUT": os.devnull,
                "EVENT": "pull_request",
                "RUST_COUNT": "1",
                "E2E_SHARDS": "1",
                "PR_LABELS": "[]",
                **changes,
            }
            result = subprocess.run(
                ["bash", "-c", "\n".join(line[10:] for line in script.splitlines())],
                env=env,
                capture_output=True,
                text=True,
                check=True,
            )
            return result.stdout.strip().removeprefix("required: ").split(",")

        self.assertIn("perf", required(DB="true", RUST="true"))
        self.assertIn("perf", required(DB="true"))
        self.assertNotIn("perf", required(WEB="true"))
        self.assertNotIn("perf", required(DOCS_ONLY="true"))

    def test_every_job_runs_on_a_pinned_runner_image(self) -> None:
        # A moving `ubuntu-latest` changed under the embedded PostgreSQL and
        # Playwright, which do not support every new image on release day.
        workflow = (SCRIPT.parents[2] / ".github/workflows/ci.yml").read_text()
        runners = re.findall(r"^\s+runs-on:\s*(\S+)", workflow, re.MULTILINE)
        self.assertTrue(runners)
        for runner in runners:
            self.assertRegex(runner, r"^ubuntu-\d{2}\.\d{2}$")

    def _required_script(self) -> str:
        workflow = (SCRIPT.parents[2] / ".github/workflows/ci.yml").read_text()
        step = workflow.split("        id: required\n", 1)[1].split("\n\n", 1)[0]
        return "\n".join(line[10:] for line in step.split("        run: |\n", 1)[1].splitlines())

    def _run_required(self, **env: str) -> tuple[list[str], dict[str, str]]:
        """Run the real step as Actions does (`bash -e`) and return its job
        list and the outputs it wrote."""
        with tempfile.NamedTemporaryFile("r") as outputs:
            result = subprocess.run(
                ["bash", "-e", "-c", self._required_script()],
                env={"PATH": os.environ["PATH"], "GITHUB_OUTPUT": outputs.name, "RUST_COUNT": "1",
                     "E2E_SHARDS": "1", "EVENT": "pull_request", **env},
                cwd=SCRIPT.parents[2], capture_output=True, text=True, check=True,
            )
            written = dict(line.split("=", 1) for line in outputs.read().splitlines())
        return result.stdout.strip().rsplit("required: ", 1)[-1].split(","), written

    def _selected(self, **env: str) -> list[str]:
        return self._run_required(**env)[0]

    def test_push_runs_the_full_suite_without_a_label_lookup(self) -> None:
        # No PR number, token or PR_LABELS: a lookup would fail the step.
        selected, outputs = self._run_required(EVENT="push")
        self.assertIn("e2e-full", selected)
        self.assertEqual(outputs["full"], "true", "the e2e-full job runs only when full=true")
        printed = subprocess.run(
            [sys.executable, str(SCRIPT), "full"], env={"PATH": os.environ["PATH"], "EVENT": "push"},
            capture_output=True, text=True, check=True,
        )
        self.assertEqual(printed.stdout.strip(), "true")

    def test_full_suite_follows_the_pull_requests_current_labels(self) -> None:
        workflow = (SCRIPT.parents[2] / ".github/workflows/ci.yml").read_text()
        # The event payload of an `opened` run predates a label applied right after.
        self.assertNotIn("github.event.pull_request.labels", workflow)
        for label in sorted(MODULE.FULL_SUITE_LABELS):
            self.assertIn("e2e-full", self._selected(E2E="true", PR_LABELS=json.dumps([label])))
        self.assertNotIn("e2e-full", self._selected(E2E="true", PR_LABELS='["documentation"]'))
        self.assertIn("e2e-full", self._selected(E2E="true", CI_FILES="true", PR_LABELS="[]"))

    def _gate(self, required: str, labels: str | None) -> subprocess.CompletedProcess:
        needs = {name: {"result": "success"} for name in ("changes", *MODULE.applicable_needs(required))}
        env = {"PATH": os.environ["PATH"], "EVENT_NAME": "pull_request",
               "NEEDS": json.dumps(needs), "REQUIRED_NEEDS": required}
        if labels is not None:
            env["PR_LABELS"] = labels
        return subprocess.run([sys.executable, str(SCRIPT)], env=env, capture_output=True, text=True)

    def test_gate_rejects_a_run_that_predates_a_full_suite_label(self) -> None:
        stale = self._gate("checks,e2e", '["database"]')
        self.assertEqual(stale.returncode, 1, stale.stdout)
        self.assertIn("database", stale.stdout)
        self.assertIn("Re-run the run triggered by the label", stale.stdout)
        self.assertEqual(self._gate("checks,e2e", "[]").returncode, 0)
        self.assertEqual(self._gate("checks,e2e,e2e-full", '["database","security"]').returncode, 0)

    def test_gate_fails_closed_when_labels_cannot_be_read(self) -> None:
        unreadable = self._gate("checks,e2e", None)
        self.assertNotEqual(unreadable.returncode, 0, "a missing label lookup must not pass the gate")
        # A reachable configuration whose request fails must fail too, for both
        # the selection and the gate, rather than fall back to "no labels".
        api = {"REPOSITORY": "owner/repo", "PR_NUMBER": "1", "GITHUB_TOKEN": "token"}
        def unavailable(*_args, **_kwargs):
            raise urllib.error.URLError("network unavailable")
        selection = {"EVENT": "pull_request", "CI_FILES": "false", **api}
        gate = {"EVENT_NAME": "pull_request", "REQUIRED_NEEDS": "checks",
                "NEEDS": json.dumps({"changes": {"result": "success"}, "checks": {"result": "success"}}), **api}
        for entry, env in ((MODULE.print_full_suite, selection), (MODULE.main, gate)):
            with mock.patch.dict(os.environ, env, clear=True), \
                    mock.patch.object(MODULE.urllib.request, "urlopen", unavailable):
                with self.assertRaises(urllib.error.URLError):
                    entry()

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
