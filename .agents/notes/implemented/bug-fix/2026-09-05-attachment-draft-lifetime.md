# Agent Note: Attachment draft lifetime

Status: implemented

## Problem

Terminal DMs and task views unmount the composer. A file queue owned by that component disappears during ordinary navigation while its separately stored text survives.

## Decision

ChatApp owns the existing conversation-keyed file queue and passes it to ChatInput. Navigating away from the composer does not dispose the queue. Files remain in memory until removed or sent; a browser reload is not a file-draft persistence guarantee.

## Alternatives considered

**Keep hidden composers mounted.** Each composer adds effects and handlers despite only one being interactive. Moving the existing state preserves its ownership without extra mounted views.

**Persist files in browser storage.** This adds storage, quota and cleanup policy for a navigation-lifetime defect. The in-memory queue already has the required lifetime under ChatApp.

## Consequences

Each conversation retains its own files across group and terminal transitions. [Attachment send recovery](2026-09-05-attachment-send-recovery.md) still owns partial-send progress and retry; that callback and ordering are unchanged. The browser regression checks retained queues, removal, exact delivered bytes and an untouched second draft through the real upload/message routes without starting a Harness.
