# Agent Note: Five instruction sections with inline help

Status: implemented

## Problem

The agent instruction editor asked for thirteen sections (Identity & Role, Goals & Responsibilities, Project Context, Communication & Triggers, Allowed Operations, Forbidden Operations, SOP / Workflow, Work Style & Output, Collaboration & Reporting, Escalation Protocol, Definition of Done, Error Handling, Scheduled Tasks). Half of them split one thought across two or three boxes, so users repeated themselves or left boxes empty; Communication, Collaboration and Escalation restated the `[DONE]`/`[BLOCKED]` and `@mention` mechanics that the core protocol already gives every agent; Scheduled Tasks was prose no code reads while Cron is a real per-agent feature. Nothing in the form said what a box was for beyond a one-line placeholder, and the AI Manager's prompt asked it to fill all thirteen with "N/A" where nothing applied.

## Decision

`AgentInstructionFields` has five keys: `role` (identity plus goals), `projectContext`, `boundaries` (allowed, forbidden, output style), `workflow` (steps, done criteria, failure handling) and `collaboration` (triggers, reporting targets, escalation). Each field carries `help` text that the form shows behind an info icon on hover and keyboard focus. `markdownToFields` still recognises the thirteen old headings and folds each into its new field with the old heading kept as a bold line, so an agent written under the old layout opens in the editor with nothing lost and is saved in the new layout. The AI Manager prompt and the team template renderer produce the same five sections.

## Alternatives considered

- **Keep thirteen sections and only add help text**: rejected. The help would have had to explain the difference between Communication, Collaboration and Escalation, which is the problem, not a documentation gap.
- **Keep Forbidden Operations as its own section**: rejected. It is the half of Boundaries people actually fill; the help text and the placeholder put the forbidden part first so it stays prominent without a separate box.
- **Keep Scheduled Tasks as a link to the Cron settings**: rejected. A section that holds no instruction text is UI chrome; the Cron editor is reachable from the agent detail panel already.
- **Migrate existing instruction files on disk**: rejected. Files are rewritten on the next save from the editor; a background rewrite of every workspace for a heading rename is not worth the blast radius.

## Consequences

- One editor, one AI Manager prompt and one template renderer share the five headings; docs pages list the same five.
- A legacy file round-trips through the editor into the new layout with its old headings preserved as bold lines inside the merged sections.
- The AI Manager instruction text itself is composed from the same prose as before, regrouped; its tests assert the five headings.
