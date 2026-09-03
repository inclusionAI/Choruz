import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from select_e2e_specs import (  # noqa: E402
    P0_SPECS,
    SMOKE,
    docs_only,
    shard_count,
    specs_for_change,
    vitest_selection,
)


class VitestSelectionTest(unittest.TestCase):
    def test_source_change_runs_related_tests(self):
        self.assertEqual(
            vitest_selection(["apps/web/lib/messages/messages.ts", "apps/web/components/chat/sidebar.tsx", "crates/x/lib.rs"]),
            ("related", ["lib/messages/messages.ts", "components/chat/sidebar.tsx"]),
        )

    def test_edited_unit_test_runs_itself(self):
        self.assertEqual(vitest_selection(["apps/web/lib/messages/messages.test.ts"]), ("related", ["lib/messages/messages.test.ts"]))

    def test_harness_change_runs_everything(self):
        self.assertEqual(vitest_selection(["apps/web/vitest.config.ts"]), ("all", []))
        self.assertEqual(vitest_selection(["pnpm-lock.yaml"]), ("all", []))

    def test_no_web_source_runs_nothing(self):
        self.assertEqual(vitest_selection(["apps/web/tests/e2e/auth.spec.ts", "apps/web/app/globals.css"]), ("none", []))


class DocsOnlyTest(unittest.TestCase):
    def test_prose_changes_are_docs_only(self):
        self.assertTrue(docs_only(["docs/onboarding.md", "README.md", "apps/web/README.md", "LICENSE"]))
        self.assertTrue(docs_only([".github/CODEOWNERS", ".github/PULL_REQUEST_TEMPLATE.md", ".editorconfig", ".nvmrc", ".gitignore"]))

    def test_any_code_file_is_not(self):
        self.assertFalse(docs_only(["docs/onboarding.md", "apps/web/app/docs/page.tsx"]))
        self.assertFalse(docs_only(["README.md", ".github/workflows/ci.yml"]))
        self.assertFalse(docs_only([".github/scripts/select_e2e_specs.py"]))
        # An Agent Note is Markdown, but the notes gate has to run on it.
        self.assertFalse(docs_only([".agents/notes/README.md"]))
        self.assertFalse(docs_only(["README.md", ".agents/notes/proposed/feature/2026-09-04-x.md"]))

    def test_empty_change_is_not_docs_only(self):
        self.assertFalse(docs_only([]))


class SpecsForChangeTest(unittest.TestCase):
    def test_backend_change_runs_everything(self):
        everything, specs = specs_for_change(["crates/choruz-domain/src/lib.rs"])
        self.assertTrue(everything)
        self.assertEqual(specs, [])

    def test_shared_web_file_runs_everything(self):
        everything, _ = specs_for_change(["apps/web/components/chat/chat-app.tsx"])
        self.assertTrue(everything)

    def test_fixture_change_runs_everything(self):
        everything, _ = specs_for_change(["apps/web/tests/fixtures/api.ts"])
        self.assertTrue(everything)

    def test_contained_feature_runs_its_specs_and_the_smoke(self):
        everything, specs = specs_for_change([
            "apps/web/components/workspace/git-graph.tsx",
            "apps/web/app/api/git-graph/route.ts",
        ])
        self.assertFalse(everything)
        self.assertEqual(specs, ["tests/e2e/git-graph.spec.ts", SMOKE])

    def test_mixed_change_falls_back_to_everything(self):
        everything, _ = specs_for_change([
            "apps/web/components/workspace/git-graph.tsx",
            "apps/web/components/chat/sidebar.tsx",
        ])
        self.assertTrue(everything)

    def test_agent_templates_are_not_documentation(self):
        self.assertFalse(docs_only(["agent-templates/core-protocol.md"]))
        self.assertEqual(
            vitest_selection(["agent-templates/extensions/file-sharing.md"]),
            ("related", ["lib/agents/agent-templates.test.ts"]),
        )

    def test_uncontained_change_keeps_the_specs_its_web_files_map_to(self):
        everything, specs = specs_for_change([
            "apps/web/components/pixel-world/pixel-sprites.ts",
            ".agents/notes/implemented/simplification/2026-09-03-pixel-world-release-assets.md",
            "apps/web/public/sprites/agents/agent_atlas.png",
        ])
        self.assertTrue(everything)
        self.assertIn("tests/e2e/pixel-world-e2e.spec.ts", specs)
        self.assertIn(SMOKE, specs)

    def test_edited_spec_runs_itself(self):
        everything, specs = specs_for_change(["apps/web/tests/e2e/theme.spec.ts"])
        self.assertFalse(everything)
        self.assertEqual(specs, ["tests/e2e/theme.spec.ts", SMOKE])

    def test_unit_tests_and_docs_select_nothing(self):
        everything, specs = specs_for_change([
            "apps/web/components/chat/message-list.test.ts",
            "apps/web/README.md",
        ])
        self.assertFalse(everything)
        self.assertEqual(specs, [])

    def test_features_union_without_duplicates(self):
        _, specs = specs_for_change([
            "apps/web/components/chat/chat-header.tsx",
            "apps/web/components/chat/detail-panel.tsx",
        ])
        self.assertEqual(len(specs), len(set(specs)))
        self.assertIn("tests/e2e/conversation.spec.ts", specs)
        self.assertIn("tests/e2e/detail-panel.spec.ts", specs)

    def test_p0_set_is_the_documented_eleven(self):
        self.assertEqual(len(P0_SPECS), 11)


class ShardCountTest(unittest.TestCase):
    def test_small_selection_uses_one_shard(self):
        self.assertEqual(shard_count(0), 1)
        self.assertEqual(shard_count(20), 1)

    def test_grows_with_tests_up_to_the_cap(self):
        self.assertEqual(shard_count(21), 2)
        self.assertEqual(shard_count(45), 3)
        self.assertEqual(shard_count(500), 3)


if __name__ == "__main__":
    unittest.main()
