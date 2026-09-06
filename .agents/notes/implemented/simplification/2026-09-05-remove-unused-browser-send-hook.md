# Agent Note: Remove the unused browser send hook

Status: implemented

## Problem

The chat input registered `window.__choruz_sendMessage` with a separate send lifecycle. Repository references consisted only of its registration and cleanup; no repository test called it. Its delayed callback duplicated send state and error handling without the normal command's attachment and conversation-draft handling.

## Decision

The chat input exposes sending through its normal input and button controls. It does not register a global automation command. Existing messaging browser tests exercise Enter submission, failed-send draft restoration, conversation drafts and attachments through those controls.

## Alternatives considered

**Keep a test-only wrapper around the normal command.** No repository automation consumes the hook, so a wrapper adds an unsupported entry point without serving a current test.

## Consequences

The component has one send lifecycle. External scripts that used the undocumented global must operate the input controls. The repository tests do not depend on that global; no replacement hook or hook-specific test is needed.
