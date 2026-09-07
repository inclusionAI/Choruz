# Agent Note: Reveal local sends without interrupting history reading

Status: implemented

## Problem

A reader scrolled into history cannot see a message they send. Treating every message from that principal as a local send would also move the reader when another client sends or history is fetched. Ordinary message bubbles additionally expose terminal CSI escape sequences.

## Decision

`chat-app.tsx` supplies an explicit conversation-scoped local-send identity to `message-list.tsx` when it appends a text or attachment message. That action resumes following the bottom; incoming messages preserve a reader's position. Opening a conversation and scrolling back to its bottom also start following. Following continues as virtualized row measurements settle; a reading gesture interrupts it. A pending history fetch cannot restore an old position after a local send resumes following.

`message-bubble.tsx` removes ANSI CSI sequences at presentation time, including Markdown fallback, quote previews and system text. Stored content and raw-copy behavior remain unchanged. The existing terminal text helper owns sequence removal; native PTY rendering is unaffected.

## Alternatives considered

**Follow every new message.** This interrupts history reading, including messages received from another client using the same account.

**Use the latest sender as a scroll signal.** Sender identity cannot distinguish local intent from fetched history or another client.

**Rewrite stored text.** Presentation cleanup does not justify changing original message content or native terminal bytes.

## Consequences

The browser regression sends while reading history and asserts the new message is visible before reload. An external-client message first proves that arrival alone does not move the reader. ANSI coverage is an active browser test rather than a skipped known defect.
