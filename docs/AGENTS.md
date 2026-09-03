# Documentation rules

How documentation is organised and written in this repository. `AGENTS.md` at the root holds the standing engineering rules; this file holds the ones about prose. The procedure for writing or auditing a page is [choruz-doc](../.agents/skills/choruz-doc/SKILL.md); sentence-level judgement is [choruz-prose-standard](../.agents/skills/choruz-prose-standard/SKILL.md).

## The tiers: one home per fact

Every fact has exactly one home. Everything else links to it.

| Tier | Home | Holds | Never holds |
|---|---|---|---|
| Standing orders | `AGENTS.md` | One to three lines per rule, linking its home | Rationale, procedures |
| Platform protocol | `CLAUDE.md` | How an agent on Choruz talks to humans and agents | Engineering rules |
| Architecture | [architecture.md](architecture.md) | The ordered map of how the system works today, read before changing `crates/`, `services/` or the pipeline | Per-crate detail, history, status annotations |
| Subsystems | [subsystems/](subsystems/README.md) | One reference page per subsystem: what it owns, the data it moves, its entry points and invariants | Cross-subsystem narrative (that is architecture) |
| Feature checklist | [adding-a-feature.md](adding-a-feature.md) | The extension point each horizontal concern of a new feature plugs into, one example each, what breaks when skipped | The seams' own contracts (those stay with their subsystem, the data model or the test policy) |
| Data model | [data-model.md](data-model.md) | Tables, ownership, `workspace_id`, sequences | Migration narration |
| Operations | [operations/](operations/) | Install, run, deploy, back up, SLO, runbook | Design rationale |
| Testing | [testing/](testing/) | PR types and the tests they require, what CI runs, e2e conventions | Test walkthroughs |
| Defensive patterns | [defensive-patterns.md](defensive-patterns.md) | Bug classes that shipped or nearly shipped here, as the rule that prevents each | War stories |
| Decisions | [.agents/notes/](../.agents/notes/README.md) | Problem, decision, alternatives, consequences | Current-state reference |
| Skills | [.agents/skills/](../.agents/skills/) | Repeatable procedures for agents and people | Product or runtime contracts |
| Contributor entry | `CONTRIBUTING.md`, `README.md` | Setup, commands, layout, links into the tiers | Duplicated rules |
| User docs | `apps/web/app/docs/` | The in-app documentation site | Engineering process |
| Package contracts | a package's `README.md`, doc comments | Config, semantics, failures, limitations | Cross-package narrative |

Placement: bugs and incidents go to a note or a runbook, rationale to notes, procedures to operations or a skill, definitions to the data model or a subsystem page, package contracts to the package README, standing orders to `AGENTS.md` with a link.

## Writing rules

- **Document current state, not change history.** No "previously", "now", "no longer", pull request numbers, or migration narration outside `archive/` and a note's `Consequences`.
- **Classify every page as a tutorial or a reference.** A tutorial orders concepts by prerequisite and ends in an observable outcome; a reference is exhaustive and skimmable. Split a page that tries to be both.
- **Test, do not assume.** A command, flag, default, error or path appears in a page only after being run or read in the current checkout; delete what did not reproduce.
- **Link by relative path**, never by bare filename or note number; a link to a heading uses its real anchor.
- **One physical line per paragraph.** Use editor soft-wrap; hard-wrapped prose makes diffs and greps unreadable.
- **Each section opens with a short orienting paragraph** before subsections or exhaustive detail.
- **Prefer the real identifier**: the crate, the table, the endpoint, the env var, the script, so the page can be checked against the tree.
- Chinese is welcome where the existing page is Chinese (`README.md`, user-facing pages); keep one page in one language.

## The slop checklist

Hunt these with the cheapest probe first (`git grep` a distinctive phrase, word counts, heading counts):

- **Duplicated rules**: the same instruction in two homes. Keep one, link from the other.
- **Narrated history**: "we used to", "this was changed", "since PR …".
- **Status annotations**: "(WIP)", "(deprecated)", "(new)" inside reference prose; status lives in the tree or the note lifecycle.
- **Hand-restated inventories**: a list the tree, a script or a manifest already owns.
- **Reasoning transcripts**: "(decision 3)", "as discussed", "the reviewer asked", hedged planning residue.
- **Repeated rationale**: the why written again where only the what belongs, instead of a link to the note.
- **Paragraph walls**: a section with no orienting sentence and no structure.
- **Emphasis inflation**: bold on whole sentences, exclamation marks, superlatives.
- **Spec-speak in implemented material**: "will", "should", "plan to", acceptance criteria, migration steps in a page or note that describes shipped reality.

## Validation

Every non-trivial change updates the document that names what it changed, in the same pull request. `python3 scripts/verify_agent_notes.py` gates the notes; links are checked by hand or in the rendered preview; a command in a page is re-run before a claim about it merges. A documentation-only pull request runs no test job and merges in seconds, so keep configuration and code out of it.
