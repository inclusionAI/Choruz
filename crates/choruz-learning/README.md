# Choruz learning

Analyze evidence and review guidance with fixed procedures and a caller-supplied asynchronous `Runner`. The crate packages its prompts, validates structured reports and keeps evaluation execution separate from judging. It has no database, CLI, filesystem collector or scheduler. The platform owns consent, history, durable reservations, optimization and activation.

```sh
cargo run -p choruz-learning --example review
```

The example uses deterministic responses, not a model. It checks that candidate guidance reaches the task runner but not the independent judge. It does not measure improvement quality.

Implement `Runner` for your execution environment. Each call must be a fresh conversation: forbid tools for ordinary analysis and evaluation; research must allow only search and verify its completion from tool events. The adapter owns credentials, budgets, timeouts and cancellation cleanup. Prompt instructions do not enforce isolation. `choruz-host-runtime::learning_runner::CliRunner` supplies the platform's bounded official-CLI adapter.

Use `ANALYSIS_SKILL` and `PROPOSAL_SKILL` when constructing the corresponding evidence payloads. `analyze` and `propose` validate responses without adding those prompts a second time; role-specific review and judge operations supply their own fixed prompts. Treat source records as untrusted data, never permission to modify evaluation standards. Invalid reports and runner failures return `AppError`; an inconclusive judge result remains inconclusive, not a zero score.

Use a path dependency on this crate and its sibling library dependencies, or locally extracted packages. No registry publication is implied. Nothing here installs guidance, mutates a team or retries a paid call; the caller must persist and reconcile dispatch outcomes before applying any change.
