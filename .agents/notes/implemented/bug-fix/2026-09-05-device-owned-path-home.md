# Agent Note: Device-owned home lookup in path inputs

Status: implemented

## Problem

A path input can remain mounted while the selected execution device changes. A pending controller home lookup can then overwrite the remote device's empty path with the controller's directory. Mount lifetime does not establish ownership of an asynchronous filesystem response.

## Decision

`PathPicker` ties its existing home request cleanup to the selected device and session. A target change aborts the previous lookup and initializes an empty input from the new device. Typing still aborts the lookup; user edits do not restart it.

## Alternatives considered

**Wait longer or retry the browser test.** Rejected: response order must not determine which device's directory is selected.

**Add a second path cache or resolver.** Rejected: the existing target-aware home request already owns resolution; only its lifecycle must follow the target.

## Consequences

No input remount or separate local/remote path flow is needed. Existing nonempty user paths remain caller-owned. The Import Sessions regression holds the controller response across a device change and checks the target home, using controlled HTTP responses rather than a second physical device.
