#!/usr/bin/env python3

"""Pick the cargo packages a change has to build and test.

Reads the changed file paths (stdin, one per line, or arguments), maps each to
the workspace member that owns it, adds every member that depends on it
(transitively, through `path` dependencies), and prints the cargo package
selection as a GitHub Actions output:

    cargo_args=-p choruz-api-gateway -p choruz-pipeline
    all=false

A change to something every crate sees (the workspace manifest or lock file,
cargo config, the toolchain pin, migrations and the test-database script the
integration tests use, the CI definition) selects the whole workspace.
EXCLUDED names crates CI never builds on Linux (none today).
"""

from __future__ import annotations

import argparse
import fnmatch
import sys
import tomllib
from pathlib import Path

EXCLUDED: tuple[str, ...] = ()
WORKSPACE_ARGS = " ".join(["--workspace", *(f"--exclude {name}" for name in EXCLUDED)])

# Anything here touches every crate.
# Files outside any package directory that one package embeds with include_str!.
EXTRA_OWNERS = (("agent-templates/**", "choruz-pipeline"),)

EVERYTHING_PATTERNS = (
    "Cargo.toml",
    "Cargo.lock",
    ".cargo/**",
    "rust-toolchain*",
    "rustfmt.toml",
    "migrations/**",
    "infra/host/setup_test_database.sh",
    ".github/workflows/**",
    ".github/actions/**",
    ".github/scripts/**",
)


def _matches(path: str, pattern: str) -> bool:
    if pattern.endswith("/**"):
        return path.startswith(pattern[:-2])
    return fnmatch.fnmatchcase(path, pattern)


def load_workspace(root: Path) -> dict[str, dict]:
    """name -> {"dir": relative dir, "deps": [workspace dep names]}"""
    manifest = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    members: dict[str, dict] = {}
    by_dir: dict[str, str] = {}
    for member in manifest["workspace"]["members"]:
        member_dir = (root / member).resolve()
        package = tomllib.loads((member_dir / "Cargo.toml").read_text(encoding="utf-8"))
        name = package["package"]["name"]
        rel = member_dir.relative_to(root.resolve()).as_posix()
        members[name] = {"dir": rel, "raw": package, "member_dir": member_dir}
        by_dir[rel] = name
    for name, info in members.items():
        deps: list[str] = []
        for section in ("dependencies", "dev-dependencies", "build-dependencies"):
            for spec in info["raw"].get(section, {}).values():
                if isinstance(spec, dict) and "path" in spec:
                    dep_dir = (info["member_dir"] / spec["path"]).resolve().relative_to(root.resolve()).as_posix()
                    dep = by_dir.get(dep_dir)
                    if dep and dep not in deps:
                        deps.append(dep)
        info["deps"] = deps
        del info["raw"], info["member_dir"]
    return members


def owner_of(path: str, members: dict[str, dict]) -> str | None:
    best: str | None = None
    for name, info in members.items():
        prefix = info["dir"] + "/"
        if path.startswith(prefix) and (best is None or len(prefix) > len(members[best]["dir"]) + 1):
            best = name
    return best


def dependents(members: dict[str, dict]) -> dict[str, set[str]]:
    reverse: dict[str, set[str]] = {name: set() for name in members}
    for name, info in members.items():
        for dep in info["deps"]:
            reverse[dep].add(name)
    return reverse


def packages_for_change(changed: list[str], members: dict[str, dict]) -> tuple[bool, list[str]]:
    """Return (everything, packages in workspace order)."""
    reverse = dependents(members)
    selected: set[str] = set()
    for raw in changed:
        path = raw.strip()
        if not path:
            continue
        if any(_matches(path, pattern) for pattern in EVERYTHING_PATTERNS):
            return True, []
        owner = owner_of(path, members)
        if owner is None:
            owner = next((name for pattern, name in EXTRA_OWNERS if _matches(path, pattern) and name in members), None)
        if owner is None:
            continue  # not a Rust file the workspace owns
        stack = [owner]
        while stack:
            name = stack.pop()
            if name in selected:
                continue
            selected.add(name)
            stack.extend(reverse[name])
    return False, [name for name in members if name in selected and name not in EXCLUDED]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("files", nargs="*", help="changed paths (default: stdin, one per line)")
    parser.add_argument("--all", action="store_true", help="select the whole workspace")
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    args = parser.parse_args()

    changed = args.files if args.files else sys.stdin.read().splitlines()
    if args.all:
        everything, packages = True, []
    else:
        everything, packages = packages_for_change(changed, load_workspace(args.root))
    if everything:
        cargo_args = WORKSPACE_ARGS
    else:
        cargo_args = " ".join(f"-p {name}" for name in packages)
    print(f"cargo_args={cargo_args}")
    print(f"all={'true' if everything else 'false'}")
    print(f"count={'all' if everything else len(packages)}")


if __name__ == "__main__":
    main()
