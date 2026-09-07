import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("verified_release", Path(__file__).resolve().parents[1] / "verified_release.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class VerifiedReleaseTests(unittest.TestCase):
    def test_workflow_binds_publication_and_deployment_to_verified_artifacts(self):
        workflows = Path(__file__).resolve().parents[2] / "workflows"
        delivery = (workflows / "cd.yml").read_text()
        ci = (workflows / "ci.yml").read_text()
        self.assertIn("cancel-in-progress: false", delivery)
        self.assertIn("python3 .github/scripts/verified_release.py", delivery)
        self.assertIn('ref: ${{ needs.authorize.outputs.sha }}', delivery)
        self.assertIn('gh run download "$CI_RUN_ID"', delivery)
        self.assertIn('--name "release-$REVISION"', delivery)
        self.assertIn("sha256sum --check", delivery)
        self.assertIn("verify-archive.py", delivery)
        self.assertNotIn("release:package", delivery)
        gateway = delivery.split("  gateway:", 1)[1]
        self.assertIn("needs: [authorize, publish]", gateway)
        self.assertIn("environment: cloud-production", gateway)
        self.assertIn("vars.CHORUZ_CLOUD_DEPLOY_ENABLED == 'true'", gateway)
        self.assertNotIn("CLOUDFLARE_API_TOKEN", delivery.split("  gateway:", 1)[0])
        self.assertIn('OPS: ${{ steps.filter.outputs.ops }}', ci)
        self.assertIn('&& add release-package', ci)
        package = ci.split("  release-package:", 1)[1].split("  required:", 1)[0]
        self.assertIn("needs.changes.outputs.ops == 'true'", package)
        self.assertLess(package.index("smoke-release.py"), package.index("actions/upload-artifact"))
        self.assertIn("release-${{ github.sha }}", package)

    def test_only_successful_same_repository_main_ci_with_packaging_is_authorized(self):
        repo = "owner/Choruz"
        run = {"name": "ci", "event": "push", "head_branch": "main", "conclusion": "success",
               "head_repository": {"full_name": repo}, "repository": {"full_name": repo}, "head_sha": "a" * 40}
        jobs = [{"name": name, "conclusion": "success"} for name in ("CI (linux) required", "Release packaging")]
        self.assertEqual(module.authorize(run, repo, jobs), "a" * 40)
        for key, value in (("event", "pull_request"), ("head_branch", "feature"), ("conclusion", "failure"),
                           ("conclusion", "cancelled"), ("name", "untrusted"), ("head_sha", "main"),
                           ("head_repository", {"full_name": "fork/Choruz"}), ("repository", {"full_name": "fork/Choruz"})):
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                module.authorize({**run, key: value}, repo, jobs)
        for result in ("failure", "skipped", "cancelled", None):
            for index in (0, 1):
                modified = copy.deepcopy(jobs)
                modified[index]["conclusion"] = result
                with self.subTest(result=result, index=index), self.assertRaises(ValueError):
                    module.authorize(run, repo, modified)
        with self.assertRaises(ValueError):
            module.authorize(run, repo, [])


if __name__ == "__main__":
    unittest.main()
