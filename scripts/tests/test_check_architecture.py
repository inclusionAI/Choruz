import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from check_architecture import BOUNDARIES, RETIRED, check  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/check_architecture.py"


class ArchitectureTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        # Use the real owner/consumer wiring, but mutate only a disposable tree.
        self.files = sorted({rule.owner for rule in BOUNDARIES} | {
            path for rule in BOUNDARIES for path, _ in rule.consumers
        })
        for path in self.files:
            target = self.root / path
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / path, target)

    def append(self, path, code):
        target = self.root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        with target.open("a") as stream:
            stream.write("\n" + code + "\n")
        if path not in self.files:
            self.files.append(path)

    def test_current_owner_wiring_passes(self):
        self.assertEqual(check(self.root, self.files), [])

    def test_bypasses_are_rejected_at_each_boundary(self):
        cases = (
            ("browser-api-transport", "apps/web/lib/api/new-client.ts", "await globalThis.fetch('/api/v1/messages');"),
            ("headless-turn", "services/choruz-connector/src/main.rs", "fn bypass() { Command::new(binary); }"),
            ("native-learning-evidence", "services/choruz-api-gateway/src/behavior_worker.rs", "runtime.retained_native_sources(binding).await?;"),
            ("learning-feedback", "crates/choruz-application/src/db_service/behavior.rs", "self.list_learning_feedback(conversation).await?;"),
            ("execution-identity", "services/choruz-api-gateway/src/decision_worker.rs", "struct ExecutionIdentity { model: String }"),
            ("database-webhook-outbox", "services/choruz-api-gateway/src/webhook.rs", "fn memory_queue(app: &ChatApp) {}"),
            ("driver-metadata", "apps/web/lib/drivers/driver-availability.ts", "const DRIVER_LABELS = new Map();"),
        )
        for name, path, code in cases:
            with self.subTest(contract=name):
                target = self.root / path
                original = target.read_text() if target.exists() else ""
                # Rust inline test modules are excluded; inject into production.
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(code + "\n" + original)
                errors = check(self.root, list(set(self.files + [path])))
                self.assertTrue(any(name + " bypass" in error for error in errors), errors)
                target.write_text(original)

    def test_removing_shared_calls_or_owners_is_not_a_green_exit(self):
        for rule in BOUNDARIES:
            with self.subTest(contract=rule.name):
                path, _ = rule.consumers[0]
                target = self.root / path
                original = target.read_text()
                target.write_text("// shared call removed\n")
                errors = check(self.root, self.files)
                self.assertTrue(any(rule.name + " shared-owner wiring missing" in error for error in errors))
                target.write_text(original)
                errors = check(self.root, [file for file in self.files if file != rule.owner])
                self.assertTrue(any(rule.name + ": missing owner" in error for error in errors))

    def test_retired_implementations_cannot_be_reintroduced(self):
        for path in RETIRED:
            self.append(path, "pub struct RetiredOwner;")
        errors = check(self.root, self.files)
        for path in RETIRED:
            self.assertTrue(any(error.startswith(path + ": retired") for error in errors))

    def test_comments_strings_tests_and_server_auth_are_not_browser_bypasses(self):
        self.append("apps/web/lib/api/new-client.ts", """
// fetch('/api/v1/messages')
const documentation = "globalThis.fetch('/api/v1/messages')";
await transport.fetch('/api/v1/messages');
""")
        self.append("apps/web/lib/api/api-auth.ts", "await fetch('/v1/me');")
        self.append("apps/web/lib/api/new-client.test.ts", "await fetch('/fixture');")
        self.append("apps/web/lib/api/clients/README.md", "Use fetch(url) only through the transport.")
        path = "services/choruz-connector/src/main.rs"
        self.append(path, '#[cfg(test)]\nmod tests { fn fixture() { Command::new("fixture"); } }')
        self.assertEqual(check(self.root, self.files), [])

    def test_multiline_strings_do_not_hide_later_production_calls(self):
        path = "services/choruz-connector/src/main.rs"
        text = (self.root / path).read_text()
        (self.root / path).write_text('const HELP: &str = "long \\\nhelp";\nfn bypass() { Command::new(binary); }\n' + text)
        self.assertTrue(any("headless-turn bypass" in error for error in check(self.root, self.files)))

    def test_executable_template_interpolation_is_checked_but_literal_text_is_not(self):
        path = "apps/web/lib/api/template-client.ts"
        self.append(path, "const docs = `Use fetch(url), not raw HTTP`; ")
        self.assertEqual(check(self.root, self.files), [])
        self.append(path, "const result = `Result: ${ {value: `Nested ${await fetch(url)}`} }`; ")
        self.assertTrue(any("browser-api-transport bypass" in error for error in check(self.root, self.files)))

    def test_inline_test_module_does_not_hide_following_production(self):
        path = "services/choruz-connector/src/main.rs"
        text = (self.root / path).read_text()
        (self.root / path).write_text('#[cfg(test)]\nmod fixtures { fn run() { Command::new("test"); } }\n'
                                      'fn bypass() { Command::new(binary); }\n' + text)
        errors = check(self.root, self.files)
        self.assertEqual(sum("headless-turn bypass" in error for error in errors), 1)

    def test_cli_checks_new_staged_files_and_exits_nonzero(self):
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        subprocess.run(["git", "-C", str(self.root), "add", "."], check=True)
        command = [sys.executable, str(SCRIPT), "--root", str(self.root)]
        before = subprocess.run(command, capture_output=True, text=True)
        self.assertEqual(before.returncode, 0, before.stdout + before.stderr)
        for path in ("apps/web/lib/api/new-client.ts", "apps/web/lib/api/clients/nested/new-client.ts"):
            self.append(path, "await window.fetch('/api/v1/messages');")
            subprocess.run(["git", "-C", str(self.root), "add", "."], check=True)
            rejected = subprocess.run(command, capture_output=True, text=True)
            self.assertEqual(rejected.returncode, 1, rejected.stdout + rejected.stderr)
            self.assertIn(path + ":2: browser-api-transport bypass", rejected.stdout)


if __name__ == "__main__":
    unittest.main()
