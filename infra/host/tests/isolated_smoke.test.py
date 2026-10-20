import concurrent.futures
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


SOURCE = Path(__file__).resolve().parents[1]


class IsolatedSmokeTests(unittest.TestCase):
    def test_remote_listener_starts_before_controller_ports_are_allocated(self):
        # Exercise the real entry guards and allocator with a live wrapper listener.
        with tempfile.TemporaryDirectory() as temporary:
            host = Path(temporary) / "infra/host"
            host.mkdir(parents=True)
            for name in ("isolated-smoke.mjs", "env.example"):
                shutil.copyfile(SOURCE / name, host / name)
            guards = (SOURCE / "web_e2e.sh").read_text().split("# shellcheck disable=SC1091", 1)[0]
            entry = host / "web_e2e.sh"
            entry.write_text(guards + '''
node --input-type=module -e '
import {createServer} from "node:net";
const server = createServer();
server.on("error", error => { console.error(error.code); process.exit(1); });
server.listen(Number(process.env.CHORUZ_API_PORT), "127.0.0.1", () => server.close());
'
''')
            (host / "remote_web_e2e.mjs").write_text('''
import {createServer} from "node:net";
import {spawn} from "node:child_process";
import {fileURLToPath} from "node:url";
const listener = createServer();
await new Promise((resolve, reject) => {
  listener.once("error", reject);
  listener.listen(Number(process.env.CHORUZ_API_PORT || 0), "127.0.0.1", resolve);
});
const child = spawn("bash", [fileURLToPath(new URL("web_e2e.sh", import.meta.url)), ...process.argv.slice(2)], {
  stdio: "inherit", env: {...process.env, CHORUZ_REMOTE_E2E_RUNNING: "1"}
});
child.once("exit", code => { listener.close(); process.exitCode = code ?? 1; });
''')
            env = dict(os.environ, CHORUZ_WEB_E2E_FULL="1")
            for key in ("CHORUZ_SMOKE_ENTRY", "CHORUZ_REMOTE_E2E_RUNNING", "CHORUZ_API_PORT"):
                env.pop(key, None)
            result = subprocess.run(["bash", str(entry)], env=env, text=True,
                                    capture_output=True, timeout=20)
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_concurrent_runs_ignore_development_state_and_clean_failed_run(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            host = root / "infra/host"
            host.mkdir(parents=True)
            for name in ("isolated-smoke.mjs", "env.example", "common.sh"):
                shutil.copyfile(SOURCE / name, host / name)
            development = host / ".env"
            development.write_text("exit 93\n")
            entry = host / "api_smoke.sh"
            entry.write_text('''#!/usr/bin/env bash
set -eu
source "$(dirname "$0")/common.sh"
export OBSERVED_RUNTIME_DIR="$RUNTIME_DIR"
cd "$(dirname "$0")"
node -e 'console.log(JSON.stringify(process.env))'
exit "${TEST_EXIT_CODE}"
''')
            recovery = host / "recovery_smoke.sh"
            shutil.copyfile(entry, recovery)
            inherited = dict(os.environ, CHORUZ_DATABASE_URL="never-connect",
                             DATABASE_URL="never-connect", CHORUZ_PG_PORT="1",
                             CHORUZ_AGENT_TOKENS_FILE="/never-read",
                             CHORUZ_RUNTIME_DIR="/never-remove", CHORUZ_HOST_ENV_FILE=str(development))

            def invoke(code):
                selected = entry if code == 0 else recovery
                return subprocess.run(["node", str(host / "isolated-smoke.mjs"), str(selected)],
                                      env=dict(inherited, TEST_EXIT_CODE=str(code)),
                                      text=True, capture_output=True, timeout=20)

            with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
                results = list(pool.map(invoke, (0, 17)))
            self.assertEqual([result.returncode for result in results], [0, 17], results)
            snapshots = [json.loads(result.stdout) for result in results]
            for env in snapshots:
                self.assertNotIn("DATABASE_URL", env)
                self.assertNotIn("CHORUZ_DATABASE_URL", env)
                self.assertNotEqual(env["CHORUZ_PG_PORT"], "1")
                self.assertTrue(Path(env["CHORUZ_RUNTIME_DIR"]).is_absolute())
                self.assertTrue(Path(env["CHORUZ_RUNTIME_DIR"]).name.startswith("choruz-smoke-"))
                self.assertEqual(env["OBSERVED_RUNTIME_DIR"], env["CHORUZ_RUNTIME_DIR"])
                self.assertEqual(env["CHORUZ_AGENT_TOKENS_FILE"], str(Path(env["CHORUZ_RUNTIME_DIR"]) / "agent_tokens.json"))
                self.assertFalse((root / env["CHORUZ_RUNTIME_DIR"]).exists())
                self.assertFalse(Path(env["CHORUZ_HOST_ENV_FILE"]).exists())
            self.assertNotEqual(snapshots[0]["CHORUZ_RUNTIME_DIR"], snapshots[1]["CHORUZ_RUNTIME_DIR"])
            self.assertEqual(development.read_text(), "exit 93\n")

    def test_rejects_unowned_entry(self):
        result = subprocess.run(["node", str(SOURCE / "isolated-smoke.mjs"), "/tmp/not-a-smoke.sh"],
                                text=True, capture_output=True, timeout=10)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Only the owned smoke entry points", result.stderr)


if __name__ == "__main__":
    unittest.main()
