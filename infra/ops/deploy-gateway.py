"""Promote one Worker version after storage preflight; restore the prior version on failure."""

import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time
import tomllib
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[2]
GATEWAY = ROOT / "services/remote-control-gateway"


def wrangler(*args, capture=False):
    result = subprocess.run(["pnpm", "exec", "wrangler", *args], cwd=GATEWAY,
                            check=True, text=True, stdout=subprocess.PIPE if capture else None)
    return json.loads(result.stdout) if capture else None


def current_version():
    deployments = wrangler("deployments", "list", "--json", capture=True)
    current = max(deployments, key=lambda deployment: deployment["created_on"])
    versions = current["versions"]
    if len(versions) != 1 or versions[0]["percentage"] != 100:
        raise ValueError("automatic delivery requires one fully deployed baseline, not a traffic split")
    return versions[0]["version_id"]


def storage_preflight(config, version):
    runtime = version["resources"]["script_runtime"]
    if runtime.get("migration_tag") != config["migrations"][-1]["tag"]:
        raise ValueError("Durable Object migration requires an operator-controlled rollout before automatic delivery")
    database = config["d1_databases"][0]
    applied = wrangler("d1", "execute", database["database_name"], "--remote", "--json",
                       "--command", "SELECT name FROM d1_migrations ORDER BY name", capture=True)
    names = {row["name"] for result in applied for row in result["results"]}
    required = {path.name for path in (GATEWAY / database["migrations_dir"]).glob("*.sql")}
    if names != required:
        raise ValueError("D1 migration set differs from this release; verify storage compatibility before deployment")


def probe(origin, version, timeout=60, require_metadata=True):
    deadline = time.monotonic() + timeout
    while True:
        try:
            headers = {"User-Agent": "Choruz-CD/1.0"}
            with urllib.request.urlopen(urllib.request.Request(origin + "/healthz", headers=headers), timeout=5) as response:
                health = json.load(response)
            if health.get("ok") is not True:
                raise ValueError("gateway health is not ready")
            if require_metadata and (health.get("version") or {}).get("id") != version:
                raise ValueError("gateway is serving a different release")
            with urllib.request.urlopen(urllib.request.Request(origin + "/v1/online/auth/get-session", headers=headers), timeout=5) as response:
                if json.load(response) is not None:
                    raise ValueError("anonymous Online session response is invalid")
            return
        except (OSError, ValueError) as error:
            if time.monotonic() >= deadline:
                raise RuntimeError("gateway readiness did not match the deployed version") from error
            time.sleep(min(2, max(0, deadline - time.monotonic())))


def deploy(revision, origin, record):
    config = tomllib.loads((GATEWAY / "wrangler.toml").read_text())
    previous = current_version()
    baseline = wrangler("versions", "view", previous, "--json", capture=True)
    storage_preflight(config, baseline)
    has_metadata = any(binding["type"] == "version_metadata" for binding in baseline["resources"]["bindings"])
    probe(origin, previous, require_metadata=has_metadata)
    record.update(previous_version=previous, phase="upload")
    tag = f"ci-{revision[:12]}-{uuid.uuid4().hex[:8]}"
    wrangler("versions", "upload", "--tag", tag, "--message", f"Verified CI revision {revision}",
             "--var", f"CHORUZ_RELEASE_SHA:{revision}", "--keep-vars")
    versions = wrangler("versions", "list", "--json", capture=True)
    candidates = [version for version in versions if version.get("annotations", {}).get("workers/tag") == tag]
    if len(candidates) != 1:
        raise ValueError("uploaded version could not be identified; no deployment was requested")
    target = candidates[0]["id"]
    record.update(target_version=target, phase="activation")
    if current_version() != previous:
        raise ValueError("gateway changed during preflight; refusing to overwrite another deployment")
    try:
        wrangler("versions", "deploy", f"{target}@100", "--yes")
        probe(origin, target)
        if current_version() != target:
            raise ValueError("gateway changed during verification")
    except Exception:
        active = current_version()
        if active not in (target, previous):
            record["phase"] = "conflicting_deployment"
            raise RuntimeError("another deployment is active; automatic rollback is unsafe")
        record["phase"] = "restoring"
        wrangler("versions", "deploy", f"{previous}@100", "--yes")
        probe(origin, previous, require_metadata=has_metadata)
        if current_version() != previous:
            raise RuntimeError("previous gateway version was not restored")
        record["phase"] = "restored_after_failure"
        raise
    record["phase"] = "healthy"


def main():
    revision = sys.argv[1]
    origin = os.environ["CHORUZ_GATEWAY_URL"].rstrip("/")
    if not re.fullmatch(r"[0-9a-f]{40}", revision) or not origin.startswith("https://"):
        raise ValueError("delivery requires a full commit SHA and an HTTPS gateway origin")
    if not os.environ.get("CLOUDFLARE_API_TOKEN") or not os.environ.get("CLOUDFLARE_ACCOUNT_ID"):
        raise ValueError("delivery requires the production Cloudflare API token and account ID")
    record = {"revision": revision, "phase": "preflight", "ci_run": os.environ.get("GITHUB_RUN_ID")}
    try:
        deploy(revision, origin, record)
    finally:
        # The record contains version IDs and lifecycle state, never credentials or request bodies.
        (ROOT / "gateway-delivery.json").write_text(json.dumps(record, indent=2) + "\n")


if __name__ == "__main__":
    main()
