# Agent Note: Decide the full e2e suite from the pull request's current labels

Status: implemented

## Problem

The full e2e suite ran when the triggering event's payload carried `ci-full`, `database` or `security`. Opening a pull request and applying its label fire `opened` and `labeled` within a second, and the per-ref concurrency group cancels one of the two runs. When the surviving run was the `opened` one, its payload had no label, change detection selected no full suite, and `CI (linux) required` passed. A database change reached a green required check without the suite its label demands; this happened twice in one day, including with `gh pr create --label`.

## Decision

`.github/scripts/check_ci_results.py` owns which labels require the full suite (`FULL_SUITE_LABELS`). Change detection calls `check_ci_results.py full`, which reads the pull request's labels through the API at job time. The required gate reads them again after every other job and fails, naming the label and the remedy, when a full-suite label is present but the run did not select `e2e-full`. A failed label lookup fails the job instead of skipping the suite. The workflow no longer reads labels from `github.event`.

## Alternatives considered

**Stop cancelling in-progress runs.** Both runs would finish and spend twice the CI time, and the `opened` run would still report a green required check without the suite.

**Run the full suite on every database or security path change.** Labels are the author's declaration of the change type; paths alone already select the other gates, and the policy keeps the full suite opt-in for other types.

## Consequences

A run that predates its label now fails the required check with a re-run instruction instead of passing silently. The `changes` and required jobs need `pull-requests: read` and one API call each. Tests cover the selection following current labels for each full-suite label and not for others, the gate rejecting a run that predates a label, and a missing label lookup failing closed; with the previous workflow and script they fail.
