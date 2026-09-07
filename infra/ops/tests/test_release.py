import http.server
import importlib.util
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import tarfile
import tempfile
import threading
import unittest
from unittest.mock import patch

OPS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(OPS))
import release

spec = importlib.util.spec_from_file_location("archive", OPS / "verify-archive.py")
archive = importlib.util.module_from_spec(spec)
spec.loader.exec_module(archive)
SHA = "a" * 40


def fixture(path):
    (path / "bin/migrations").mkdir(parents=True)
    for binary in release.BINARIES:
        target = path / "bin" / binary
        target.write_text("#!/bin/sh\nexit 0\n")
        target.chmod(0o755)
    (path / "bin/migrations/schema.sql").write_text("SELECT 1;\n")
    (path / "web/apps/web/.next/static").mkdir(parents=True)
    (path / "web/apps/web/server.js").write_text("// fixture\n")
    manifest = {"format": 1, "revision": SHA, "platform": platform.system(),
                "architecture": platform.machine(), "files": release.inventory(path)}
    (path / "manifest.json").write_text(json.dumps(manifest))
    return path


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="choruz-release-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.old = fixture(self.root / "good")
        self.new = fixture(self.root / "candidate")
        self.current = self.root / "current"
        self.current.symlink_to(self.old)

    def test_manifest_rejects_corruption_extra_files_missing_binary_and_escaping_links(self):
        self.assertEqual(release.verify(self.new)["revision"], SHA)
        binary = self.new / "bin/choruz"
        binary.write_text("corrupt")
        with self.assertRaisesRegex(ValueError, "manifest"):
            release.verify(self.new)
        extra = self.old / "unexpected"
        extra.write_text("untracked payload")
        with self.assertRaisesRegex(ValueError, "manifest"):
            release.verify(self.old)
        extra.unlink()
        (self.old / "outside").symlink_to("/tmp")
        with self.assertRaisesRegex(ValueError, "escapes"):
            release.verify(self.old)

    def test_archive_roundtrip_and_wrong_revision(self):
        output = self.root / "release.tar.gz"
        with tarfile.open(output, "w:gz") as bundle:
            bundle.add(self.new, arcname="choruz")
        self.assertEqual(archive.verify_archive(output, SHA)["revision"], SHA)
        with self.assertRaisesRegex(ValueError, "revision"):
            archive.verify_archive(output, "b" * 40)
        with tarfile.open(output, "w:gz") as bundle:
            bundle.add(self.new / "bin/choruz", arcname="../outside")
        with self.assertRaisesRegex(ValueError, "escapes"):
            archive.verify_archive(output, SHA)

    def test_environment_files_and_incompatible_libc_cannot_be_activated(self):
        (self.new / ".env.production").write_text("TEST_CONFIGURATION=not-a-real-secret")
        with self.assertRaisesRegex(ValueError, "environment files"):
            release.verify(self.new)
        (self.new / ".env.production").unlink()
        manifest = json.loads((self.new / "manifest.json").read_text())
        manifest["libc"] = ["glibc", "2.39"]
        (self.new / "manifest.json").write_text(json.dumps(manifest))
        with patch.object(release.platform, "libc_ver", return_value=("glibc", "2.32")):
            with self.assertRaisesRegex(ValueError, "baseline"):
                release.activate(self.root, self.new, [], 1)
        self.assertEqual(self.current.resolve(), self.old)

    def test_exact_service_controls_and_atomic_links(self):
        for system, expected in (
            ("Linux", ("sudo", "systemctl", "restart", "choruz-api-gateway.service", "choruz-pipeline.service", "choruz-web-app.service")),
            ("Darwin", ("launchctl", "kickstart", "-k", f"gui/{os.getuid()}/com.choruz.api-gateway")),
        ):
            with patch.object(release.platform, "system", return_value=system), patch.object(release, "run") as calls:
                release.control_services("restart")
                self.assertIn(unittest.mock.call(*expected), calls.call_args_list)
        release.replace_link(self.current, self.new)
        self.assertEqual(self.current.resolve(), self.new)
        self.assertFalse((self.old / "current").exists())
        with release.activation_lock(self.root):
            with self.assertRaisesRegex(ValueError, "in progress"):
                with release.activation_lock(self.root):
                    self.fail("overlapping activation was allowed")

    def test_real_deploy_entry_restores_previous_after_http_failure_and_supports_rollback(self):
        # Only the OS service manager is replaced. The shipped CLI, manifest,
        # symlinks, readiness HTTP calls and recovery all run unchanged.
        commands = self.root / "commands"
        commands.mkdir()
        log = self.root / "services.log"
        manager = commands / ("sudo" if platform.system() == "Linux" else "launchctl")
        manager.write_text(f"#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{log}'\n")
        manager.chmod(0o755)
        failing = {"enabled": True}
        current = self.current

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_response(503 if failing["enabled"] and current.resolve().name == "candidate" else 200)
                self.end_headers()
                self.wfile.write(b"ready")

            def log_message(self, *args):
                pass

        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever)
        thread.start()

        def close():
            server.shutdown()
            server.server_close()
            thread.join()

        self.addCleanup(close)
        origin = f"http://127.0.0.1:{server.server_port}"
        options = ["--releases", str(self.root), "--timeout", "0.02"]
        for endpoint in ("/api/readyz", "/pipeline/readyz", "/web"):
            options += ["--health-url", origin + endpoint]
        env = {**os.environ, "PATH": str(commands) + os.pathsep + os.environ["PATH"]}

        def invoke(action, *args):
            return subprocess.run([sys.executable, str(OPS / "release.py"), action, *map(str, args), *options],
                                  env=env, capture_output=True, text=True)

        failed = invoke("deploy", self.new)
        self.assertNotEqual(failed.returncode, 0, failed.stdout)
        self.assertIn("previous release restored and healthy", failed.stderr)
        self.assertEqual(self.current.resolve(), self.old)
        self.assertFalse((self.root / "previous").exists())
        failing["enabled"] = False
        success = invoke("deploy", self.new)
        self.assertEqual(success.returncode, 0, success.stderr)
        self.assertEqual(self.current.resolve(), self.new)
        self.assertEqual((self.root / "previous").resolve(), self.old)
        restored = invoke("rollback")
        self.assertEqual(restored.returncode, 0, restored.stderr)
        self.assertEqual(self.current.resolve(), self.old)
        self.assertEqual((self.root / "previous").resolve(), self.new)
        self.assertIn("com.choruz.api-gateway" if platform.system() == "Darwin" else "choruz-api-gateway.service", log.read_text())
        self.assertNotIn("choruz-choruz-", log.read_text())

    def test_package_does_not_activate_and_contains_migrations_and_nested_public_assets(self):
        source = self.root / "source"
        source.mkdir()
        (source / "migrations").mkdir()
        (source / "migrations/schema.sql").write_text("SELECT 1;")
        for binary in release.BINARIES:
            destination = source / "target/release" / binary
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes((self.new / "bin" / binary).read_bytes())
            destination.chmod(0o755)
        for part in ("standalone/apps/web", "static"):
            (source / "apps/web/.next" / part).mkdir(parents=True)
        (source / "apps/web/.next/standalone/apps/web/server.js").write_text("// server")
        (source / "apps/web/.next/standalone/apps/web/server.js").chmod(0o664)
        (source / "apps/web/.next/standalone/apps/web/.env.production").write_text("TEST_CONFIGURATION=fixture")
        (source / "apps/web/.next/standalone/apps/web/public").mkdir()
        (source / "apps/web/public").mkdir()
        (source / "apps/web/public/logo.svg").write_text("<svg/>")
        (source / "infra/ops").mkdir(parents=True)
        # Compilers are outside this packaging contract; actual release binaries
        # and the standalone Next server are also packaged in CI.
        def output(args, **kwargs):
            return SHA if args[0] == "git" else json.dumps({"target_directory": str(source / "target")}).encode()

        with patch.object(release, "ROOT", source), patch.object(release, "run"), patch.object(release.subprocess, "check_output", side_effect=output):
            target = release.package(self.root)
        self.assertEqual(self.current.resolve(), self.old)
        self.assertFalse((self.root / "previous").exists())
        self.assertTrue((target / "web/apps/web/public/logo.svg").is_file())
        self.assertTrue((target / "bin/migrations/schema.sql").is_file())
        self.assertFalse((target / "web/apps/web/.env.production").exists())
        self.assertEqual(release.verify(target)["revision"], SHA)
        bundle = self.root / "dist" / f"{target.name}.tar.gz"
        self.assertEqual(archive.verify_archive(bundle, SHA)["revision"], SHA)


if __name__ == "__main__":
    unittest.main()
