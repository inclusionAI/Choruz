# Agent Note: Structured Claude and Codex direct conversations

Status: implemented

## Problem

A CLI's terminal theme and screen geometry belong to that CLI. Rendering its
screen in a light conversation pane can produce dark backgrounds, clipped
content and controls that do not match the surrounding application. Changing
ANSI colors cannot provide accessible tool approvals or a readable transcript.

## Decision

Claude Code and Codex direct conversations use the Harness's structured protocol
on the binding's execution device. Claude uses bidirectional stream JSON; Codex
uses app-server. The gateway authorizes the same human, direct conversation and
binding as the terminal route. Local and linked devices execute the same session
requests. Other Harnesses retain the terminal interface.

Conversation presents native text and tool events, not a second model-generated
summary and not synthetic group messages. A process instance fences commands;
submission IDs prevent automatic duplicate turns after an uncertain delivery.
Pending approvals remain attached to the live process across a browser reload.
The optional Terminal view closes the structured process before opening a PTY.
Native session identity, rather than the browser buffer, provides continuity.
Binding refreshes from native capture preserve the selected Details tab; only
a different conversation or binding identity resets that selection.
Claude's completed envelopes can contain separate blocks of one message, so
their offsets span envelopes as they do in the stream. Codex's reconstructed
history can use different item IDs from live events; full native history
replaces the preview on resume instead of being appended to it.

Direct Claude history has a binding-owned terminal anchor, separate from the
headless session used by groups. Preparing a session journals its selected ID
without starting a process. The gateway reserves that ID before execution.
Seeding either namespace from the other requires a native fork; subsequent
turns never share a mutable native transcript. An anchor write tolerates a
concurrent headless completion but rejects changes to account, host, generation
or the existing direct anchor. Authorization-derived workspace fields are not
stored configuration and are excluded from that comparison.

Host changes use the terminal launch guard. They stop the old device's process
before clearing direct anchors and advancing the generation. A disconnected
device must reconnect before its bindings move or are revoked: missing presence
does not prove that its terminal children have stopped.
Assignment and revocation lock the target device row before binding launch
guards, preventing a late assignment from attaching to a revoked device.

The package [session contract](../../../../crates/choruz-host-runtime/README.md#structured-conversations)
owns transport limits and persistence. The
[runtime subsystem](../../../../docs/subsystems/agent-runtime.md) owns API routing.

## Alternatives considered

- Recolor terminal bytes: cannot change a CLI's layout or expose semantic approvals.
- Parse the rendered screen: loses native event IDs and cannot safely correlate
  approval responses with pending requests.
- Reuse headless group messages as direct history: mixes independently executing
  turns and makes native resume ownership ambiguous.
- Build separate remote adapters: duplicates parsing and lifecycle rules that
  already belong on the execution device.
- Restart a CLI on every browser reconnect: loses pending interactions and can
  duplicate a paid turn after ambiguous delivery.

## Consequences

This supersedes the Claude/Codex presentation decision in
[native terminal history](2026-05-27-native-terminal-dm-history.md), not its exact
identity and account-isolation requirements. Group delivery remains headless.
The structured journal contains conversation content and must be protected and
retained as session data, not treated as a disposable diagnostic log. Explicit
CLI protocol adapters require compatibility tests when Harness versions change.

## Verification

`apps/web/tests/e2e/terminal.spec.ts` exercises both Harness protocols through the
browser, API and real connector with deterministic external CLI fixtures. The
`structured_session_probe` example exercises installed Harness binaries in a
disposable workspace. Session unit tests cover stale instances, approvals,
duplicate delivery and bounded transport pages. Runtime-store tests fence
concurrent direct and headless identity updates.
