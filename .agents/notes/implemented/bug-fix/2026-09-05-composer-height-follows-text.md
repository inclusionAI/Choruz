# Agent Note: Composer height follows text

Status: implemented

## Problem

Input events do not cover restored drafts or send-driven clearing. Resizing only in the input handler leaves the main composer at another conversation's height.

## Decision

`apps/web/components/chat/chat-input.tsx` derives textarea height from committed `inputText` in one effect. The existing 120px cap remains. Typing, mention insertion, clearing and restoration share this calculation rather than storing another height state.

## Alternatives considered

**Resize at each text mutation.** Rejected because draft restoration and asynchronous send paths would each need the same DOM measurement and could diverge again.

## Consequences

Text changes trigger one DOM measurement. The browser retains ordinary textarea scrolling above the cap. `apps/web/tests/e2e/messaging.spec.ts` checks multiline growth, empty-composer shrink after sending, and height restoration when switching drafts through the real dashboard.
