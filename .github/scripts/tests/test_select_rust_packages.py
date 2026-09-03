import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from select_rust_packages import load_workspace, packages_for_change  # noqa: E402

ROOT = Path(__file__).resolve().parents[3]

MEMBERS = {
    "choruz-common": {"dir": "crates/choruz-common", "deps": []},
    "choruz-store": {"dir": "crates/choruz-store", "deps": ["choruz-common"]},
    "choruz-api-gateway": {"dir": "services/choruz-api-gateway", "deps": ["choruz-common", "choruz-store"]},
    "choruz-cli": {"dir": "apps/choruz-cli", "deps": []},
    "choruz-pipeline": {"dir": "services/choruz-pipeline", "deps": []},
}


class PackagesForChangeTest(unittest.TestCase):
    def test_agent_templates_test_the_pipeline_that_embeds_them(self):
        self.assertEqual(packages_for_change(["agent-templates/core-protocol.md"], MEMBERS), (False, ["choruz-pipeline"]))

    def test_leaf_change_selects_only_that_crate(self):
        self.assertEqual(packages_for_change(["services/choruz-api-gateway/src/main.rs"], MEMBERS), (False, ["choruz-api-gateway"]))

    def test_shared_crate_selects_its_dependents(self):
        everything, packages = packages_for_change(["crates/choruz-common/src/lib.rs"], MEMBERS)
        self.assertFalse(everything)
        self.assertEqual(packages, ["choruz-common", "choruz-store", "choruz-api-gateway"])

    def test_workspace_wide_files_select_everything(self):
        for path in ("Cargo.lock", ".cargo/config.toml", "rustfmt.toml", "migrations/V040__x.sql", "infra/host/setup_test_database.sh", ".github/workflows/ci.yml"):
            with self.subTest(path=path):
                self.assertEqual(packages_for_change([path], MEMBERS), (True, []))

    def test_files_outside_the_workspace_select_nothing(self):
        self.assertEqual(packages_for_change(["apps/web/lib/messages/messages.ts", "docs/x.md", ".github/CODEOWNERS"], MEMBERS), (False, []))

    def test_member_manifest_belongs_to_its_crate(self):
        self.assertEqual(packages_for_change(["apps/choruz-cli/Cargo.toml"], MEMBERS), (False, ["choruz-cli"]))


class LoadWorkspaceTest(unittest.TestCase):
    def test_reads_the_real_workspace(self):
        members = load_workspace(ROOT)
        self.assertIn("choruz-api-gateway", members)
        self.assertEqual(members["choruz-api-gateway"]["dir"], "services/choruz-api-gateway")
        self.assertIn("choruz-common", members["choruz-api-gateway"]["deps"])
        self.assertIn("choruz-supervisor", members["choruz-server"]["deps"])


if __name__ == "__main__":
    unittest.main()
