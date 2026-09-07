"""Authorize delivery from a completed main-branch CI run, never from a PR artifact."""

import json
import os
import re
import subprocess


def authorize(run, repository, jobs):
    if (run.get("name"), run.get("event"), run.get("head_branch"), run.get("conclusion")) != (
        "ci", "push", "main", "success"
    ):
        raise ValueError("delivery requires a successful push-to-main CI run")
    if run.get("head_repository", {}).get("full_name") != repository:
        raise ValueError("delivery cannot consume another repository's artifact")
    if run.get("repository", {}).get("full_name") != repository:
        raise ValueError("CI run repository does not match delivery repository")
    sha = run.get("head_sha", "")
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise ValueError("CI run has no valid revision")
    for name in ("CI (linux) required", "Release packaging"):
        if not any(job.get("name") == name and job.get("conclusion") == "success" for job in jobs):
            raise ValueError(f"CI run has no successful {name} job")
    return sha


def api(path):
    return json.loads(subprocess.check_output(["gh", "api", path]))


def main():
    repository = os.environ["GITHUB_REPOSITORY"]
    run_id = os.environ["CI_RUN_ID"]
    if not run_id.isdecimal():
        raise ValueError("CI run id must be numeric")
    run = api(f"repos/{repository}/actions/runs/{run_id}")
    jobs = []
    page = 1
    while True:
        batch = api(f"repos/{repository}/actions/runs/{run_id}/jobs?per_page=100&page={page}")["jobs"]
        jobs.extend(batch)
        if len(batch) < 100:
            break
        page += 1
    sha = authorize(run, repository, jobs)
    with open(os.environ["GITHUB_OUTPUT"], "a") as output:
        output.write(f"sha={sha}\nrun_id={run_id}\n")


if __name__ == "__main__":
    main()
