# Agent Note: Pin the CI runner image

Status: implemented

## Problem

Ten CI jobs ran on `ubuntu-latest`. When GitHub moved that label to Ubuntu 26.04, the same commit that passed on 24.04 failed there: the embedded PostgreSQL used by Rust tests could not load `libxml2.so.2`, and Playwright refused to install Chromium on the new image. The move rolls out per repository, so the private and public repositories ran the same tree on different images within an hour.

## Decision

Every job in `.github/workflows/ci.yml` names an explicit image, currently `ubuntu-24.04`. A CI policy test in `.github/scripts/tests/test_check_ci_results.py` rejects `ubuntu-latest` or any unpinned label. Moving to a newer image is a deliberate change that updates the pin once the embedded PostgreSQL and Playwright support it.

## Alternatives considered

**Install the missing libraries on 26.04.** That fixes PostgreSQL but not Playwright's platform check, and it ties the workflow to whatever the next image removes.

**Keep `ubuntu-latest` and react to breakage.** A label move then turns `main` red with no code change, and the private and public repositories disagree until both move.

## Consequences

Runner upgrades no longer arrive silently; someone has to bump the pin and run the full suite on the new image. The pinned image eventually reaches end of support, at which point GitHub's deprecation notices make the bump due.
