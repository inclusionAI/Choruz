# Agent Note: Replace prose "Assignments" with a "Board tasks created" receipt

Status: implemented

Formerly ADR-005.

## Problem

The project-operator role template (`apps/web/lib/groups/team-templates.ts`) defined `outputContract.requiredSections = ["Plan", "Assignments", "Blockers", "Next Steps"]`. In local testing the operator satisfied "Assignments" with a chat prose section and never issued `task_create` commands, so no board tasks appeared. A prose "Assignments" section is a structural duplicate of what `task_create` produces: as long as an agent can satisfy the output contract by writing prose, the task command is optional and intermittent.

## Decision

`Assignments` is replaced in the project-operator (and every other coordinator-style) role's `requiredSections` by `Board tasks created`. The renderer in `apps/web/lib/groups/team-template-renderer.ts` explains that this section is a receipt of the `task_create` calls just made: a numbered list of `task_id — title — assignee` lines. When the agent has no board-worthy work this turn, the section reads `Board tasks created: none — <reason>`, which keeps the contract satisfiable without prose-only assignment.

## Alternatives considered

- **Redefine `Assignments` as a one-line human-readable summary that complements the commands**: rejected because it keeps a prose section an agent can fill without issuing any command.
- **Keep the `Assignments` prose and additionally require `task_create` commands in the contract**: rejected because two representations of the same fact drift, and the prose one is the easier to satisfy.

## Consequences

- A prose-only response cannot pass the output contract: the agent runs `task_create` or states why no task was warranted.
- The chat thread keeps a human-readable audit trail of board changes without duplicating their meaning.
- Role templates that copy the project-operator contract (`software-development-team` and future coordinator templates) inherit the change.
- `apps/web/lib/groups/team-templates.test.ts` and `team-template-renderer.test.ts` assert that prose "Assignments" alone does not satisfy the receipt and that "Board tasks created" appears in every visible coordinator role's rendered required sections.
