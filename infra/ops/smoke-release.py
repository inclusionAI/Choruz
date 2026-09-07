"""Boot the actual archive away from the checkout with a disposable database."""

import contextlib
import importlib.util
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tarfile
import tempfile
import urllib.request

from release import healthy

spec = importlib.util.spec_from_file_location("verify_archive", Path(__file__).with_name("verify-archive.py"))
archive = importlib.util.module_from_spec(spec)
spec.loader.exec_module(archive)


def command(*args, **kwargs):
    return subprocess.run(args, check=True, stdout=subprocess.DEVNULL, **kwargs)


def stop(process):
    process.terminate()
    try:
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()


def smoke(bundle, revision):
    archive.verify_archive(bundle, revision)
    with tempfile.TemporaryDirectory(prefix="choruz-package-smoke-") as temp, contextlib.ExitStack() as cleanup:
        root = Path(temp).resolve()
        with tarfile.open(bundle) as source:
            source.extractall(root, filter="data")
        release = next(root.iterdir())
        state = root / "state"
        state.mkdir()
        # No live instance's database, connector files or runtime paths may enter
        # this acceptance run. Port leases prevent parallel tests choosing ours.
        env = {key: value for key, value in os.environ.items()
               if not key.startswith(("CHORUZ_", "NEXT_PUBLIC_", "PG"))}
        leases = [cleanup.enter_context(socket.socket()) for _ in range(4)]
        for lease in leases:
            lease.bind(("127.0.0.1", 0))
        pg_port, api_port, pipeline_port, web_port = [lease.getsockname()[1] for lease in leases]
        pgdata = state / "pgdata"
        pglog = state / "postgres.log"
        command("initdb", "-D", str(pgdata), "--auth=trust", "--username=choruz_smoke", env=env)
        leases[0].close()
        try:
            command("pg_ctl", "-D", str(pgdata), "-l", str(pglog), "-o",
                    f"-h 127.0.0.1 -p {pg_port} -k ''", "-w", "start", env=env)
        except subprocess.CalledProcessError:
            if pglog.exists():
                print(pglog.read_text(), file=sys.stderr)
            raise
        cleanup.callback(command, "pg_ctl", "-D", str(pgdata), "-m", "immediate", "-w", "stop", env=env)
        database = f"postgres://choruz_smoke@127.0.0.1:{pg_port}/postgres"
        for migration in sorted((release / "bin/migrations").glob("*.sql")):
            command("psql", "--no-psqlrc", "--dbname", database, "--set", "ON_ERROR_STOP=1", "--file", str(migration), env=env)
        api = f"http://127.0.0.1:{api_port}"
        env.update(CHORUZ_DATABASE_URL=database, CHORUZ_API_HOST="127.0.0.1", CHORUZ_API_PORT=str(api_port),
                   CHORUZ_API_BASE_URL=api, CHORUZ_PIPELINE_METRICS_HOST="127.0.0.1",
                   CHORUZ_PIPELINE_METRICS_PORT=str(pipeline_port), CHORUZ_RUNTIME_DIR=str(state / "runtime"),
                   CHORUZ_CONNECTOR_CONFIG_DIR=str(state / "connectors"), CHORUZ_ATTACHMENT_DIR=str(state / "attachments"),
                   CHORUZ_AGENT_TOKENS_FILE=str(state / "agent_tokens.json"),
                   PORT=str(web_port), HOSTNAME="127.0.0.1", NODE_ENV="production")
        processes = []
        for index, args in enumerate((
            [str(release / "bin/choruz-api-gateway")],
            [str(release / "bin/choruz-pipeline")],
            ["node", str(release / "web/apps/web/server.js")],
        ), start=1):
            leases[index].close()
            log = cleanup.enter_context((state / f"service-{index}.log").open("w+"))
            process = subprocess.Popen(args, cwd=state, env=env, stdout=log, stderr=log)
            cleanup.callback(stop, process)
            processes.append((process, log))
        try:
            endpoints = [api + "/readyz", f"http://127.0.0.1:{pipeline_port}/readyz", f"http://127.0.0.1:{web_port}/docs"]
            healthy(endpoints, 60)
            for endpoint, name in zip(endpoints, ("choruz-api-gateway", "choruz-pipeline")):
                with urllib.request.urlopen(endpoint, timeout=5) as response:
                    payload = json.load(response)
                if payload.get("service") != name or payload.get("database") is not True:
                    raise ValueError(f"packaged {name} readiness contract failed")
            for process, _ in processes:
                if process.poll() is not None:
                    raise ValueError("packaged service exited during acceptance")
            command(str(release / "bin/choruz"), "--help", env=env, cwd=state)
            print(f"Verified extracted release {revision}: API + PostgreSQL, pipeline and standalone web are ready")
        except Exception:
            for _, log in processes:
                log.flush()
                log.seek(0)
                print(log.read()[-8000:], file=sys.stderr)
            raise


if __name__ == "__main__":
    smoke(sys.argv[1], sys.argv[2])
