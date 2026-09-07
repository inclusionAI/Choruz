"""Build immutable host releases and activate them with verified recovery."""

import argparse
import contextlib
import fcntl
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
SERVICES = ("api-gateway", "pipeline", "web-app")
BINARIES = ("choruz", "choruz-server", "choruz-api-gateway", "choruz-pipeline", "choruz-connector")


def run(*args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def digest(path):
    result = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            result.update(chunk)
    return result.hexdigest()


def inventory(directory):
    result = {}
    for path in sorted(directory.rglob("*")):
        name = path.relative_to(directory).as_posix()
        if name == "manifest.json":
            continue
        if path.name == ".env" or path.name.startswith(".env."):
            raise ValueError(f"release must not contain environment files: {name}")
        if path.is_symlink():
            if not path.resolve(strict=True).is_relative_to(directory.resolve()):
                raise ValueError(f"release symlink escapes its directory: {name}")
            result[name] = {"symlink": os.readlink(path)}
        elif path.is_file():
            result[name] = {"sha256": digest(path), "mode": path.stat().st_mode & 0o777}
    return result


def verify(directory):
    directory = directory.resolve(strict=True)
    manifest = json.loads((directory / "manifest.json").read_text())
    if manifest.get("format") != 1 or manifest.get("files") != inventory(directory):
        raise ValueError("release manifest does not match the files; do not activate this package")
    for name in BINARIES:
        if not os.access(directory / "bin" / name, os.X_OK):
            raise ValueError(f"release binary is missing or not executable: {name}")
    for name in ("web/apps/web/server.js", "web/apps/web/.next/static", "bin/migrations"):
        if not (directory / name).exists():
            raise ValueError(f"release artifact is missing: {name}")
    return manifest


def package(releases):
    run("git", "diff", "--quiet", "HEAD", cwd=ROOT)
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    name = f"choruz-{revision}-{platform.system().lower()}-{platform.machine()}"
    target = releases / name
    if target.exists():
        raise ValueError(f"immutable release already exists: {target}")
    releases.mkdir(parents=True, exist_ok=True)
    # Failed builds never change current/previous or leave an activatable release.
    with tempfile.TemporaryDirectory(prefix=".build-", dir=releases) as staging:
        stage = Path(staging) / name
        (stage / "bin").mkdir(parents=True)
        run("cargo", "build", "--locked", "--release", "-p", "choruz-cli", "-p", "choruz-server",
            "-p", "choruz-api-gateway", "-p", "choruz-pipeline", "-p", "choruz-connector", cwd=ROOT)
        run("pnpm", "install", "--frozen-lockfile", cwd=ROOT)
        run("pnpm", "web:build", cwd=ROOT)
        metadata = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=ROOT))
        for binary in BINARIES:
            shutil.copy2(Path(metadata["target_directory"]) / "release" / binary, stage / "bin")
        shutil.copytree(ROOT / "migrations", stage / "bin/migrations")
        shutil.copytree(ROOT / "apps/web/.next/standalone", stage / "web", symlinks=True,
                        ignore=shutil.ignore_patterns(".env", ".env.*"))
        shutil.copytree(ROOT / "apps/web/.next/static", stage / "web/apps/web/.next/static", dirs_exist_ok=True)
        shutil.copytree(ROOT / "apps/web/public", stage / "web/apps/web/public", dirs_exist_ok=True)
        shutil.copytree(ROOT / "infra/ops", stage / "infra/ops", ignore=shutil.ignore_patterns("__pycache__"))
        # Tar's safe extraction strips group/world writes and restores owner
        # read/write. Canonicalize before hashing so extracted modes still match.
        for path in stage.rglob("*"):
            if not path.is_symlink():
                path.chmod(0o755 if path.is_dir() or path.stat().st_mode & 0o100 else 0o644)
        manifest = {"format": 1, "revision": revision, "platform": platform.system(),
                    "architecture": platform.machine(), "libc": platform.libc_ver(),
                    "ci_run": os.environ.get("GITHUB_RUN_ID"), "files": inventory(stage)}
        (stage / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        verify(stage)
        stage.rename(target)
    dist = releases / "dist"
    dist.mkdir(exist_ok=True)
    archive = dist / f"{name}.tar.gz"
    with tarfile.open(archive, "w:gz") as output:
        output.add(target, arcname=name)
    (dist / f"{archive.name}.sha256").write_text(f"{digest(archive)}  {archive.name}\n")
    print(archive)
    return target


def replace_link(link, target):
    pending = link.with_name(link.name + ".next")
    pending.unlink(missing_ok=True)
    pending.symlink_to(target)
    os.replace(pending, link)


@contextlib.contextmanager
def activation_lock(releases):
    releases.mkdir(parents=True, exist_ok=True)
    with (releases / ".activation.lock").open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise ValueError("another release activation is in progress") from None
        yield


def control_services(action):
    if platform.system() == "Linux":
        run("sudo", "systemctl", "daemon-reload")
        run("sudo", "systemctl", action, *(f"choruz-{name}.service" for name in SERVICES))
    elif platform.system() == "Darwin":
        for name in SERVICES:
            label = f"gui/{os.getuid()}/com.choruz.{name}"
            if action == "restart":
                run("launchctl", "kickstart", "-k", label)
            else:
                run("launchctl", "bootout", label)
    else:
        raise ValueError("managed activation supports Linux and macOS only")


def healthy(urls, timeout):
    deadline = time.monotonic() + timeout
    while True:
        try:
            for url in urls:
                with urllib.request.urlopen(url, timeout=3) as response:
                    if response.status != 200:
                        raise ValueError(f"health endpoint returned {response.status}")
            return
        except (OSError, ValueError) as error:
            if time.monotonic() >= deadline:
                raise RuntimeError("release health checks did not become ready") from error
            time.sleep(min(1, max(0, deadline - time.monotonic())))


def activate(releases, target, urls, timeout):
    with activation_lock(releases):
        manifest = verify(target)
        if (manifest["platform"], manifest["architecture"]) != (platform.system(), platform.machine()):
            raise ValueError("release platform does not match this device")
        built_libc, built_version = manifest.get("libc", ("", ""))
        local_libc, local_version = platform.libc_ver()
        if built_libc and (built_libc != local_libc or tuple(map(int, built_version.split("."))) > tuple(map(int, local_version.split(".")))):
            raise ValueError("release C library baseline exceeds this device; use a compatible build")
        current, previous = releases / "current", releases / "previous"
        known_good = current.resolve(strict=True) if current.is_symlink() else None
        if current.exists() and not current.is_symlink():
            raise ValueError("current must be a release symlink, not a directory")
        if known_good:
            verify(known_good)
        replace_link(current, target.resolve())
        try:
            control_services("restart")
            healthy(urls, timeout)
        except Exception:
            if known_good:
                replace_link(current, known_good)
                try:
                    control_services("restart")
                    healthy(urls, timeout)
                    print("Activation failed; previous release restored and healthy", file=sys.stderr)
                except Exception as recovery:
                    raise RuntimeError("CRITICAL: activation and restoration failed; inspect managed services") from recovery
            else:
                control_services("stop")
                current.unlink()
                print("First activation failed; no previous release exists", file=sys.stderr)
            raise
        if known_good and known_good != target.resolve():
            replace_link(previous, known_good)
        print(f"Active release: {target.resolve()}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("package", "verify", "deploy", "rollback", "restart"))
    parser.add_argument("target", nargs="?", type=Path)
    parser.add_argument("--releases", type=Path, default=ROOT / "releases")
    parser.add_argument("--health-url", action="append", help="repeat for API, pipeline and web readiness endpoints")
    parser.add_argument("--timeout", type=float, default=60)
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error("--timeout must be positive")
    releases = args.releases.resolve()
    if args.action == "package":
        package(releases)
    elif args.action == "verify":
        if not args.target:
            parser.error("verify requires an extracted release directory")
        print(json.dumps({key: value for key, value in verify(args.target).items() if key != "files"}))
    elif args.action == "restart":
        control_services("restart")
    else:
        if not args.health_url or len(args.health_url) < 3:
            parser.error("activation requires explicit --health-url values for API, pipeline and web")
        target = releases / "previous" if args.action == "rollback" else args.target
        if target is None:
            parser.error("deploy requires an extracted, verified release directory; it does not rebuild")
        activate(releases, target.resolve(strict=True), args.health_url, args.timeout)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"Release failed: {error}", file=sys.stderr)
        sys.exit(1)
