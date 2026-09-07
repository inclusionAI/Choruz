import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("gateway_delivery", Path(__file__).resolve().parents[1] / "deploy-gateway.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class GatewayDeliveryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="choruz-cd-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "migrations").mkdir()
        (self.root / "migrations/0001.sql").write_text("SELECT 1;")
        (self.root / "wrangler.toml").write_text('[[migrations]]\ntag="v1"\n[[d1_databases]]\ndatabase_name="test"\nmigrations_dir="migrations"\n')
        self.state = {"active": "old", "tag": None, "migration": "v1", "applied": "0001.sql"}
        self.commands = []
        self.probes = []
        self.root_patch = patch.object(module, "GATEWAY", self.root)
        self.root_patch.start()
        self.addCleanup(self.root_patch.stop)

    def wrangler(self, *args, capture=False):
        self.commands.append(args)
        if args[:2] == ("deployments", "list"):
            return [{"created_on": "2026-09-06", "versions": [{"version_id": self.state["active"], "percentage": 100}]}]
        if args[:2] == ("versions", "view"):
            return {"resources": {"script_runtime": {"migration_tag": self.state["migration"]}, "bindings": [{"type": "version_metadata"}]}}
        if args[:2] == ("d1", "execute"):
            return [{"results": [{"name": self.state["applied"]}]}]
        if args[:2] == ("versions", "upload"):
            self.state["tag"] = args[args.index("--tag") + 1]
        elif args[:2] == ("versions", "list"):
            return [{"id": "candidate", "annotations": {"workers/tag": self.state["tag"]}}]
        elif args[:2] == ("versions", "deploy"):
            self.state["active"] = args[2].split("@")[0]
        else:
            raise AssertionError(args)

    def test_verified_promotion_and_failed_probe_restore_exact_previous_version(self):
        for fail in (False, True):
            self.state["active"] = "old"
            record = {}

            def probe(origin, version, **kwargs):
                self.probes.append(version)
                if fail and version == "candidate":
                    raise RuntimeError("wrong runtime version")

            with patch.object(module, "wrangler", side_effect=self.wrangler), patch.object(module, "probe", side_effect=probe):
                if fail:
                    with self.assertRaisesRegex(RuntimeError, "wrong runtime version"):
                        module.deploy("a" * 40, "https://test.invalid", record)
                    self.assertEqual(self.state["active"], "old")
                    self.assertEqual(record["phase"], "restored_after_failure")
                    self.assertEqual(self.probes[-1], "old")
                else:
                    module.deploy("a" * 40, "https://test.invalid", record)
                    self.assertEqual(self.state["active"], "candidate")
                    self.assertEqual(record["phase"], "healthy")

    def test_storage_migration_gates_run_before_upload_or_activation(self):
        for key, value in (("migration", "older"), ("applied", "unknown.sql")):
            self.commands.clear()
            before = self.state[key]
            self.state[key] = value
            with patch.object(module, "wrangler", side_effect=self.wrangler), self.assertRaisesRegex(ValueError, "migration"):
                module.deploy("a" * 40, "https://test.invalid", {})
            self.assertFalse(any(command[:2] in (("versions", "upload"), ("versions", "deploy")) for command in self.commands))
            self.state[key] = before

    def test_intervening_deployment_is_not_overwritten(self):
        def probe(origin, version, **kwargs):
            if version == "old":
                self.state["active"] = "someone-else"

        with patch.object(module, "wrangler", side_effect=self.wrangler), patch.object(module, "probe", side_effect=probe):
            with self.assertRaisesRegex(ValueError, "another deployment"):
                module.deploy("a" * 40, "https://test.invalid", {})
        self.assertEqual(self.state["active"], "someone-else")
        self.assertFalse(any(command[:2] == ("versions", "deploy") for command in self.commands))

    def test_probe_rejects_wrong_version_and_broken_online_route(self):
        import io
        for health, session in (({"ok": True, "version": {"id": "wrong"}}, None),
                                ({"ok": True, "version": {"id": "right"}}, {"user": "unexpected"})):
            responses = [io.BytesIO(json.dumps(item).encode()) for item in (health, session)]
            with patch.object(module.urllib.request, "urlopen", side_effect=responses):
                with self.assertRaisesRegex(RuntimeError, "readiness"):
                    module.probe("https://test.invalid", "right", timeout=0)


if __name__ == "__main__":
    unittest.main()
