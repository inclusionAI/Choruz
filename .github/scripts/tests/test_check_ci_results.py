import importlib.util
from pathlib import Path
import unittest


SCRIPT = Path(__file__).parents[1] / "check_ci_results.py"
SPEC = importlib.util.spec_from_file_location("check_ci_results", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class CheckCiResultsTests(unittest.TestCase):
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
