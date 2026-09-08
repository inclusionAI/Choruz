# Agent Note: Retain device computer-use skills in isolated accounts

Status: implemented

## Problem

An operator installs BrowserSkill and Cua for a device user, but an isolated Claude or Codex profile does not necessarily discover those skills. A service-started Harness can also lack the user-local binary directory on PATH.

## Decision

The shared Agent runtime prepares the two installed skill directories when it prepares an isolated account for execution. Device-local launch paths use this owner, including the host link and headless connector. The read-only account resolver stays separate so scans and authentication probes do not create files. The child PATH appends the existing user-local binary directory without overriding inherited binaries.

## Alternatives considered

**Copy the default Harness home.** This mixes credentials and settings across accounts. Only named skill directories are shared, and account-specific entries win.

**Download tools during every launch.** Installation requires operator trust and sometimes native consent or a desktop. Agent startup neither installs software nor promises a browser on a headless host.

## Consequences

Tool installations remain device-local and opt-in through the operator's installation. Missing skills do not prevent normal Agent work; filesystem errors preparing an installed skill are reported as launch errors. Symlinks follow upstream skill updates, and existing account overrides are preserved. Remote devices need their own installations; pairing does not expose the controller's browser or credentials. Tests exercise the shared host-link dispatcher with distinct process homes, verify the selected account's unchanged credentials and run a child shell through the augmented PATH.
