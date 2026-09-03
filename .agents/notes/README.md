# Agent Notes

An Agent Note records one decision: the problem it answers, what was chosen, what it beat, and what it costs. Notes are the reason a maintainer can answer "why is it like this?" a year later without re-deriving the design. `docs/` describes how the system works today; Agent Notes describe why.

## Layout and naming

Every Agent Note has two axes, both encoded in its **path**:

```
.agents/notes/{lifecycle}/{class}/yyyy-mm-dd-topic-title.md
```

- **Lifecycle** is one of `proposed/`, `implemented/`, `rejected/`. A fourth folder, `archived/`, holds frozen implemented notes (see [Archiving and deletion](#archiving-and-deletion)); its path omits the lifecycle segment: `archived/{class}/yyyy-mm-dd-topic-title.md`.
- **Class** is the kind of decision (see [Classification](#classification)).
- The date in the filename is when the topic was **first proposed** (per git history). It does not change when the note moves between lifecycle folders.

Cross-references between Agent Notes use relative Markdown links (`[topic](../../implemented/architecture/2026-08-18-modular-monolith.md)`), never bare prose or numbers, so they are mechanically checkable and survive moves between folders. Do not add a centralized `INDEX.md`: the tree is the index, and `git log` is the history.

## Classification

| Class | What it covers |
|---|---|
| `feature` | A new user-, agent- or model-facing capability. |
| `bug-fix` | Corrects a defect or closes a gap an incident surfaced. |
| `simplification` | Removes code, behaviour, or surface area without adding a capability. |
| `architecture` | A structural decision about the **shipped source**: how crates and services relate, what the runtime vocabulary is, what the data model carries. |
| `process` | Tooling, policy, or workflow **around** the code: CI gates, the test policy, release mechanics, this system itself. Not runtime behaviour. |
| `testing` | Test infrastructure and strategy. |

Pick the class that matches the decision, not the files touched: a CI change that alters how the pipeline behaves is `feature` or `architecture`, a runtime change made purely to keep a gate honest is `process`.

## Lifecycle folders

- **`proposed/`**: proposals reviewed before implementation; not yet built, or only partly.
- **`implemented/`**: the decision shipped. The file records what was decided and what was rejected, and is **kept current with what actually shipped**: when the code later moves a file, renames a crate, or changes a key or default, the note is updated in the same change to match (facts only: paths, names, structure; never the decision itself). See [implemented/AGENTS.md](implemented/AGENTS.md).
- **`rejected/`**: the proposal was considered and declined. Keep it only while its rationale prevents a tempting, meaningful mistake; otherwise delete it.

## Archiving and deletion

An implemented note whose decision no longer describes anything in the tree, or whose content has been consolidated into a newer owning note, moves to `archived/{class}/`. Archival keeps `Status: implemented`, inserts an `Archived: YYYY-MM-DD` line directly below it, and repairs or removes inbound links. Those are the only permitted content changes during archival.

Once archived, a note is frozen: do not edit, reformat, update, move, or delete it, and do not treat it as authority for current behaviour. Never archive a proposed note; reject an obsolete proposal instead.

An implemented note that is fully superseded may be consolidated into the current owning note and deleted. Before deletion, the owner must preserve every unique rationale, alternative, consequence, and named coverage gap, and repair every inbound link. Partial supersession does not qualify: keep both notes cross-linked and update every fact that remains current.

## When to write one

Every non-trivial change MUST add or update at least one Agent Note in the same pull request. A change is non-trivial when it alters behaviour, architecture, a contract shared across crates, services or packages, process or tooling, testing strategy, an on-disk, wire, database or configuration format, or another decision a maintainer may reasonably revisit. A proposal for substantial future work starts in `proposed/`; a decision already made starts in `implemented/`.

Updating the note that already owns the decision satisfies the rule; do not create a duplicate. Only a purely mechanical or local edit with no change to behaviour, contracts, structure, process, or rationale is exempt: a rename, a comment, a formatting pass, a dependency bump with no behaviour change, a test made deterministic.

A note is never edited into a *different decision*: supersede it with a new one and keep both cross-linked. Editing an `implemented/` note to track where its existing decision now lives is required, not forbidden.

## The file format

`scripts/verify_agent_notes.py` enforces every mechanical clause below; CI runs it on any change under `.agents/`.

### The header block

The first three lines of every Agent Note are exactly:

```markdown
# Agent Note: <title>

Status: <status>
```

followed by a blank line. `Status:` is one of three forms and must agree with the lifecycle folder the file sits in; the gate cross-checks them:

- `Status: proposed`
- `Status: implemented`
- `Status: rejected — <why, in one line>`

The status carries no dates and no parentheticals: the filename holds the first-proposed date, git holds everything else, and an "accepted in amended form" remark is body content. The rejection reason is the one status with content, because a rejected note's verdict is the fact readers come for.

An archived note has `Status: implemented` on line 3 and `Archived: YYYY-MM-DD` on line 4, then the blank line.

### The body skeleton

Every Agent Note opens its body with `## Problem`: the motivation, written to stand without the solution. What follows depends on the lifecycle. Recurring sections use these canonical names and nothing else; genuinely bespoke technical sections (data model, wire contract, migration table, UI flow) stay free-form between the required ones.

#### `proposed/`

```markdown
## Problem
## Proposal
…bespoke sections…
## Alternatives considered
## Acceptance criteria
## Risks
```

`## Proposal` is the intended change and may speak in the future tense: plans, migration steps and open questions belong here while the work is unbuilt. `## Acceptance criteria` says what observable state means done. `## Risks` covers both what could go wrong and what the change knowingly gives up.

#### `implemented/`

```markdown
## Problem
## Decision
…bespoke sections…
## Alternatives considered
## Consequences
```

`## Decision` describes shipped reality in the present tense, and the whole file is kept current with it. `## Consequences` records what the trade-off cost **and** bought. Proposal-era headings are spec-speak here and the gate rejects them: `## Proposal`, `## Plan`, `## Migration plan` and `## Acceptance criteria` may not appear in an implemented note. A `## Testing`, `## Deferred` or `## Related` section is fine where it states present-tense fact.

#### `rejected/`

A rejected note is the proposal, frozen: it keeps whatever proposal-time sections it had, and the verdict lives on the `Status:` line. Only the header block, the `## Problem` opener, a `## Proposal` section and the alternatives mandate apply.

#### `archived/`

Only the header block is checked. The body is whatever the note contained when it was sealed.

### Alternatives considered: mandatory

Every proposed, implemented and rejected note carries an `## Alternatives considered` section: each genuine alternative and why it lost, one bold-led paragraph per alternative or a `### Why not <X>?` subsection per contested one. A decision recorded without what it beat invites re-litigation, the failure Agent Notes exist to prevent.

Alternatives are recorded, never invented. A note dated before 2026-09-03 (the day this format was adopted) whose alternatives are not reconstructible from the record carries this exact comment in place of the section, which the gate accepts for pre-format files only:

```markdown
<!-- agent-note-format: alternatives-not-recorded (pre-format Agent Note) -->
```

### Moving between lifecycles

Moving a file between lifecycle folders means updating the `Status:` line and re-satisfying that folder's skeleton in the same change; the gate fails the move otherwise. `proposed/` → `implemented/` rewrites `## Proposal` into a present-tense `## Decision`, folds `## Acceptance criteria` and `## Risks` into `## Consequences` (or a present-tense `## Testing` section for what now pins the behaviour), and drops plans in favour of what shipped. `proposed/` → `rejected/` only adds the reason to the `Status:` line and freezes the file.

## Writing style

- Present tense for what exists, future tense only inside `## Proposal`.
- Name files, crates, tables and endpoints by their real paths and identifiers so the note can be checked against the tree.
- Link pull requests inline where they matter (`[PR #162](https://github.com/jcguo123/Choruz/pull/162)`); the header has no PR field.
- No change history inside a note. "Previously X, now Y" belongs in git; the note states Y.
