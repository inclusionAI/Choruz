"""Exercise acquired-database cleanup when migration application fails."""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest


class DatabaseCleanupTest(unittest.TestCase):
    def test_migration_failure_drops_only_the_acquired_database(self):
        script = Path(__file__).resolve().parents[1] / "setup_test_database.sh"
        with tempfile.TemporaryDirectory(prefix="choruz-db-cleanup-") as scratch:
            root = Path(scratch)
            (root / "migrations").mkdir()
            (root / "migrations" / "V001.sql").write_text("invalid SQL")
            psql = root / "psql"
            psql.write_text(
                "#!/bin/bash\n"
                'printf "%s\\n" "$*" >> "$TEST_SQL_LOG"\n'
                'for arg in "$@"; do [[ "$arg" == "-f" ]] && exit 17; done\n'
                "exit 0\n"
            )
            psql.chmod(0o700)
            log = root / "queries"
            env = {key: value for key, value in os.environ.items()
                   if not key.startswith("CHORUZ_")}
            env.update(PATH=f"{root}:{env['PATH']}", TEST_SQL_LOG=str(log),
                       CHORUZ_REQUIRE_TEST_DATABASE="1")
            run = subprocess.run(["bash", "-ec", 'source "$1"', "test", str(script)],
                                 cwd=root, env=env, capture_output=True, text=True, timeout=10)
            self.assertEqual(run.returncode, 17, run.stderr)
            queries = log.read_text().splitlines()
            created = next(line.split("CREATE DATABASE ")[1] for line in queries if "CREATE DATABASE " in line)
            self.assertTrue(any(f"DROP DATABASE IF EXISTS {created}" in line for line in queries), queries)


if __name__ == "__main__":
    unittest.main()
