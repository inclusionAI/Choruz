# Choruz evaluation

Use fixed tasks to evaluate and optimize instructions and team configurations without starting Choruz, PostgreSQL or an agent CLI. This crate depends only on Serde and serde_json. It does not collect traces, run models, schedule work or install the selected configuration.

## Run a search

The executable example uses a deterministic local responder, not a language model:

```sh
cargo run -p choruz-evaluation --example optimize
```

Create an `EvaluationSuite` with disjoint training, validation and test cases, then initialize `Optimization` with two seed candidates and bounded budgets. Dispatch each `SearchAction` to your own model or runner. Grade deterministic tasks with `OutputCheck::score`; for rubric-based tasks, obtain an independent `JudgeResult`. Submit conclusive scores through `complete_scored_rollout`. Inconclusive judgments must fail or defer the caller's run rather than become a zero score.

For proposals, pass only `proposal_input` to the proposer and return its result to `complete_proposal`. Do not pass the complete suite: validation and test answers must remain outside proposal construction. `evolve_team` allows bounded team proposals as well as instructions; the caller executes the team.

Serialize `Optimization` together with the unchanged suite to checkpoint a search. Persist an action reservation before dispatch and the result after completion. A pending action after a crash has an uncertain external outcome: the caller must reconcile or fail it, not silently repeat a paid call. Winner selection precedes held-out testing; `can_apply` checks the comparison but grants no authority to install anything.

## Use outside the platform

Add `choruz-evaluation` as a path dependency pointing to this directory, or to the extracted crate archive. There are no parent-directory assets or dependencies on other workspace crates. The Cargo package contains the source, this contract and the runnable example. No registry publication is implied by the repository version.
