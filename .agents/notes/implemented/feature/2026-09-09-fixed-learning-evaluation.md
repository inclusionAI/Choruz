# Agent Note: Fixed learning evaluation

Status: implemented

## Problem

Content review cannot determine whether a guidance revision improves task execution. Comparing two changing configurations or letting a proposer supply its own grade also produces evidence that cannot support selection.

## Decision

An explicit, owner-scoped comparison freezes a task suite and snapshots of the active and proposed guidance. Expected outputs are withheld from the execution prompt and checked by deterministic code. Training, validation and test partitions have distinct inputs. Evaluation uses the same device abstraction as foreground work, but runs in fresh tool-free conversations rather than replaying production tasks. A reviewed execution team is captured with its guidance and executed through the [shared team runner](2026-09-10-evolving-execution-teams.md).

One durable pending comparison per binding bounds concurrency. A lease reserves each old/new case before model execution. Settings changes fence publication; an uncertain call after lease expiry fails the run rather than repeating spend. History retains partial results. The background comparison does not activate guidance or change the existing [trace-review decision](2026-09-08-background-experience-learning.md).

## Alternatives considered

**Use the content review as the score.** Agreement with a proposal does not test its effect on a task.

**Replay recent user tasks in the real workspace.** Repeating tool use can resend messages or mutate files. Self-contained evaluation inputs make the permitted effects explicit.

**Let the candidate edit evaluation criteria.** An improved grade could then reflect a weaker test rather than better behavior. Criteria remain immutable within a comparison.

**Retry every interrupted call automatically.** A missing result does not establish that the model call was uncharged. Failed runs preserve evidence without unbounded retries.

## Consequences

The execution adapter measures text and JSON response behavior, not arbitrary tool-using workflows. A passing suite is scoped evidence, not a universal improvement claim. Calls consume the target account's quota, including preflight calls. Explicit model selection and a configuration fingerprint prevent mixing ordinary binding changes, but do not pin provider model weights, an executable changed in place or externally replaced profile credentials. The owner can inspect every case without receiving internal lease tokens.
