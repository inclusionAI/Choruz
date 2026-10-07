# Agent Note: Preserve workbench state through navigation and connection recovery

Status: implemented

## Problem

Editor unmounts and page reloads discard local work. Provisioning a task does not mean its first instruction reached the native process. A failed initial bootstrap cannot establish a valid user identity, and a pending Send must not prevent its own cancellation.

## Decision

`apps/web/lib/chat-drafts.ts` owns tab-local draft persistence. File drafts retain both edited content and the original save precondition, scoped to the authenticated principal, project and path. Each editor mount restores its buffer; discarding a dirty tab requires confirmation. A file tab retains its close control even when it is the only open tab. Successful saves clear the draft. Storage failures retain an in-memory buffer for navigation, without promising reload persistence.

Task provisioning stores the initial session draft and submission identity before navigation. `AgentSessionView` retains edits and retries until the session command accepts the instruction. A cancelled preparation retains the text for an explicit later submission instead of automatically restarting it after reload. The existing native-session submission owner still determines duplicate acceptance; browser persistence is not a durable native execution receipt.

`components/chat/dashboard.tsx` obtains the authenticated bootstrap through the shared API transport before mounting `ChatApp`. Failure renders a connection state with automatic and explicit retry. No empty principal or fabricated empty workspace enters the chat shell. An invalid session uses the existing recovery route.

Stop uses its own pending state and reaches the existing interrupt command while Send is awaiting team preparation. Expected cancellation conflicts do not turn a ready session into a disconnected display.

## Alternatives considered

**Keep every editor mounted.** This retains buffers only until reload and keeps a full CodeMirror instance for every file. Persisting content and its precondition supports both navigation and reload without another file-write owner.

**Clear the task draft after provisioning.** A binding can exist before its process accepts instructions. Clearing at command acceptance preserves recovery without creating a second task store.

**Render the chat shell with an empty identity after bootstrap failure.** Identity-dependent hooks cannot recover that shell. Connection recovery must precede its mount.

## Consequences

Drafts contain user content in tab-local session storage. Browser eviction or denied storage can prevent reload recovery. Restored file buffers retain the existing conflict detection from [file save preconditions](2026-09-05-editor-external-conflicts.md). The production browser scenarios verify retained edits, accepted task text after reload, bootstrap recovery and cancellation without native task submission. External model generation is replaced by the existing deterministic CLI fixture, so these checks do not establish provider availability or model quality.
