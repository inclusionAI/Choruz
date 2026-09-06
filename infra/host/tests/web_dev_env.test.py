"""Exercise the launcher's child-process environment without binding ports."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


class WebLaunchEnvironmentTest(unittest.TestCase):
    def test_configured_web_port_reaches_next_bootstrap(self):
        host = Path(__file__).resolve().parents[1]
        with tempfile.TemporaryDirectory(prefix="choruz-web-launch-") as scratch:
            root = Path(scratch)
            fixture_host = root / "infra" / "host"
            fixture_host.mkdir(parents=True)
            for name in ("common.sh", "web_dev.sh", "env.example"):
                shutil.copyfile(host / name, fixture_host / name)
            config = (host / "env.example").read_text()
            config = config.replace("CHORUZ_API_PORT=3000", "CHORUZ_API_PORT=38220")
            config = config.replace("CHORUZ_WEB_PORT=3100", "CHORUZ_WEB_PORT=48220")
            (fixture_host / ".env").write_text(config)
            bin_dir = root / "bin"
            bin_dir.mkdir()
            pnpm = bin_dir / "pnpm"
            # The only replaced boundary is Next's process: inspect what the
            # shipped launcher actually passes, not the script's spelling.
            pnpm.write_text(
                "#!/usr/bin/env python3\n"
                "import json, os, sys\n"
                "print('CHILD=' + json.dumps({"
                "'web_port': os.environ.get('CHORUZ_WEB_PORT'),"
                "'api_port': os.environ.get('CHORUZ_API_PORT'),"
                "'args': sys.argv[1:]}))\n"
            )
            pnpm.chmod(0o700)
            env = os.environ.copy()
            for key in ("CHORUZ_WEB_PORT", "CHORUZ_API_PORT", "CHORUZ_PG_PORT", "CHORUZ_PIPELINE_METRICS_PORT"):
                env.pop(key, None)
            env.update(PATH=f"{bin_dir}:{env['PATH']}", CHORUZ_ENV="production")
            result = subprocess.run(
                ["bash", str(fixture_host / "web_dev.sh")],
                env=env, text=True, capture_output=True, check=True, timeout=30,
            )
            child = json.loads(next(line[6:] for line in result.stdout.splitlines() if line.startswith("CHILD=")))
            self.assertEqual(child["web_port"], "48220")
            self.assertEqual(child["api_port"], "38220")
            self.assertEqual(child["args"], ["--dir", "apps/web", "dev", "--port", "48220"])


if __name__ == "__main__":
    unittest.main()
