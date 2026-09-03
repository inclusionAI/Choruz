#!/usr/bin/env python3

"""Fail the required CI job unless every dependency the run asked for succeeded.

The workflow's `changes` job lists the jobs a run must pass in REQUIRED_NEEDS
(comma-separated; empty for a documentation-only change, which needs none).
A job on that list that was skipped, cancelled, failed or never reported
fails the gate; a job the list leaves out may do anything.
"""

import json
import os


def failures(needs: dict[str, dict[str, str]]) -> list[tuple[str, str]]:
    """Return every dependency that did not explicitly succeed."""
    return sorted(
        (name, dependency.get("result", "missing"))
        for name, dependency in needs.items()
        if dependency.get("result") != "success"
    )


def applicable_needs(required_needs: str) -> tuple[str, ...]:
    """The jobs that must succeed, from the workflow's comma-separated list."""
    return tuple(name.strip() for name in required_needs.split(",") if name.strip())


def main() -> None:
    needs = json.loads(os.environ["NEEDS"])
    required = applicable_needs(os.environ["REQUIRED_NEEDS"])
    failed = failures({name: needs.get(name, {}) for name in required})
    if failed:
        print("CI dependencies did not succeed:")
        for name, result in failed:
            print(f"{name}: {result}")
        raise SystemExit(1)
    print(f"All CI dependencies succeeded: {', '.join(required) or 'none required'}.")


if __name__ == "__main__":
    main()
