#!/usr/bin/env python3

"""Fail the required CI job unless every dependency the run asked for succeeded.

The workflow's `changes` job lists the jobs a run must pass in REQUIRED_NEEDS
(comma-separated; empty for a documentation-only change, which needs none).
Change detection must succeed even when its selection is empty. A selected
job that was skipped, cancelled, failed or never reported fails the gate.

This script also owns which pull requests need the full e2e suite. Labels are
read from the pull request when the job runs, not from the triggering event:
an `opened` and a `labeled` run start together and the concurrency group
cancels one of them, so the surviving run may predate the label. `full`
prints the selection for the `changes` job; the gate re-reads the labels and
fails a run that selected less than its labels now require.
"""

import json
import os
import sys
import urllib.request

# Labels that ask for the full e2e suite (docs/testing/pr-test-policy.md).
FULL_SUITE_LABELS = frozenset({"ci-full", "database", "security"})


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


def full_suite_requested(event: str, ci_files_changed: bool, labels: set[str]) -> bool:
    """The full e2e suite runs on main, for a workflow change and on request."""
    return event == "push" or ci_files_changed or bool(FULL_SUITE_LABELS & labels)


def unmet_full_suite_labels(labels: set[str], required: tuple[str, ...]) -> list[str]:
    """Labels that ask for the full suite when this run did not select it."""
    return [] if "e2e-full" in required else sorted(FULL_SUITE_LABELS & labels)


def pull_request_labels() -> set[str]:
    """The pull request's labels now. PR_LABELS (a JSON list) replaces the
    API call in tests; any lookup failure fails the job rather than skipping."""
    if "PR_LABELS" in os.environ:
        return set(json.loads(os.environ["PR_LABELS"]))
    request = urllib.request.Request(
        f"https://api.github.com/repos/{os.environ['REPOSITORY']}/issues/"
        f"{os.environ['PR_NUMBER']}/labels?per_page=100",
        headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {os.environ['GITHUB_TOKEN']}",
        },
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        return {label["name"] for label in json.load(response)}


def print_full_suite() -> None:
    event = os.environ["EVENT"]
    labels = pull_request_labels() if event == "pull_request" else set()
    print("true" if full_suite_requested(event, os.environ.get("CI_FILES") == "true", labels) else "false")


def main() -> None:
    needs = json.loads(os.environ["NEEDS"])
    required = applicable_needs(os.environ["REQUIRED_NEEDS"])
    failed = failures({name: needs.get(name, {}) for name in ("changes", *required)})
    if failed:
        print("CI dependencies did not succeed:")
        for name, result in failed:
            print(f"{name}: {result}")
        raise SystemExit(1)
    if os.environ.get("EVENT_NAME") == "pull_request":
        unmet = unmet_full_suite_labels(pull_request_labels(), required)
        if unmet:
            print(
                f"The pull request now carries {', '.join(unmet)}, which requires the full e2e "
                "suite, but this run did not select it because it started before the label was "
                "applied. Re-run the run triggered by the label."
            )
            raise SystemExit(1)
    print(f"All CI dependencies succeeded: {', '.join(required) or 'none required'}.")


if __name__ == "__main__":
    if sys.argv[1:] == ["full"]:
        print_full_suite()
    else:
        main()
