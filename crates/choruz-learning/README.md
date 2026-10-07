# Choruz learning

Analyze evidence, review guidance and execute reserved optimization steps with caller-supplied asynchronous adapters. The crate packages fixed prompts and workflow policy, validates structured reports and keeps evaluation execution separate from judging. It has no database or scheduler. Optional native source reading does not start a collector. The caller owns consent, history, durable reservations and activation.

```sh
cargo run -p choruz-learning --example review
```

The example uses deterministic responses, not a model. It runs the same `workflow::evaluate_task` entry as the platform and checks that candidate guidance reaches the task runner but not the independent judge. It does not measure improvement quality.

## Compose the workflow

`analysis_workflow::analyze_window` analyzes a selected trace window, verifies references and revision chronology, and reviews proposed guidance. Supply scoped evidence recovery, independent model calls and persistence reads through `AnalysisServices`. Task admission uses `task_quality::inspect` with blind trials and source-grounded review. These adapters must use the fixed procedures; they do not choose different admission or escalation rules.

`workflow::optimization_step` executes one pending action from `choruz-evaluation::optimization::Optimization`. Supply an `EvaluationExecutor` and a `ProposalExecutor`; native CLI consumers can use `choruz-host-runtime::learning_executor::LearningExecutor`. Persist the reservation before dispatch and commit the updated search with its report. An error leaves an uncertain external outcome: reconcile or fail the reservation rather than retrying automatically.

The returned analysis is a proposal, not installed guidance. Persist its checkpoint and evidence under the original reservation before considering activation. Standalone consumers provide their own storage and scheduling; this library does not start another background service or install a Codex plugin.

## Native evidence

Enable `native-source` to read Claude Code or Codex transcripts with `native_source::read`. Supply one account home, workspace and native session identifier through `Source`; no ambient account selection, credential read, process launch or platform database is involved. The caller must authorize the source and wait until its turn has finished. The platform retains that live-session gate in its host adapter.

Windows preserve complete records and byte cursors, exclude native reasoning records, and include bounded workspace guidance. `references` recovers projected records before the committed cursor; `behavior_references` adds observed model/version attribution. Neither operation certifies a source as safe to publish. Persist the cursor only with the analysis result; do not treat returned data as instructions.

```sh
cargo run -p choruz-learning --features native-source --example native-source
```

The example creates and removes a synthetic account. It demonstrates filesystem reading only, not automatic collection or learning activation.

## Model execution

Implement `Runner` for your execution environment, or enable `native-cli` to use `native_cli::CliRunner` with an installed Claude Code or Codex binary. This is the same adapter used by the platform, without its PTY or device-dispatch dependencies. `CliConfig` selects the driver, executable, model and local account profile; it never attaches to a foreground workspace or session.

Each call must be a fresh conversation: forbid tools for ordinary analysis and evaluation; research must allow only search and verify its completion from tool events. The native adapter enforces a 256 KiB input bound, 1 MiB output bound and 75-second deadline, validates tool events, and terminates its process group when dropped. It uses the selected account's existing profile without copying credentials. Custom adapters own equivalent budgets, restrictions and cancellation cleanup; prompt instructions alone do not enforce isolation.

Use `ANALYSIS_SKILL` and `PROPOSAL_SKILL` when constructing the corresponding evidence payloads. `analyze` and `propose` validate responses without adding those prompts a second time; role-specific review and judge operations supply their own fixed prompts. Treat source records as untrusted data, never permission to modify evaluation standards. Invalid reports and runner failures return `AppError`; an inconclusive judge result remains inconclusive, not a zero score.

Use a path dependency on this crate and its sibling library dependencies, or locally extracted packages. No registry publication is implied. Nothing here installs guidance, mutates a team or retries a paid call; the caller must persist and reconcile dispatch outcomes before applying any change.

`build_program` supplies `PROGRAM_SKILL` to an isolated, tool-free runner with training examples only. It validates a finite-output [`choruz-decision::programs::Program`](../choruz-decision/README.md), or returns `None` when the builder declines. This is generation, not evaluation or permission to execute. The caller keeps held-out examples separate and assesses outputs through the same fixed task checks and judge used for other candidates.
