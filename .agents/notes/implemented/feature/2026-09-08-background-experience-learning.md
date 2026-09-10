# Agent Note: Background experience learning

Status: implemented

## Problem

An Agent can repeat a mistake or ignore a durable user preference across turns. Asking the foreground Agent to summarize itself adds latency and mixes retrospective evidence with the task it is executing. A generated recommendation is also not evidence that a later task improved.

## Decision

An opt-in, binding-scoped policy selects a separate analysis Agent. A gateway worker reads the target's native transcript and shared conversation feedback through existing device operations. Fresh analysis processes use the selected account without resuming either Agent's active session. The independently written [analysis prompt](../../../../agent-templates/experience-analysis.md) groups work into meaningful attempts and preserves later corrections without assigning blame to missing user input.

The worker consumes bounded, resumable windows instead of retaining only the transcript tail. Checkpoints, summaries and reviewed revisions commit together under a generation-fenced lease. Disabling learning or selecting a revision invalidates an in-flight result. Existing project instructions remain untouched; the current active revision supplies labeled context to subsequent structured and headless submissions, after the original routing envelope.

A no-change report can cite no evidence while an objective remains open. Its committed cursor preserves the boundary for recovering an opener or other historical citation from the original scoped source. The worker revalidates only identifiers requested by the bounded analysis report: native record boundaries in the selected device, account, workspace and session, or messages in currently shared owner/Agent conversations. Native recovery returns projected visible records for validation without copying historical content into the prompt. A summary mentioning an identifier does not establish provenance. A problem still requires failure evidence in the current window; recovery cannot turn repeated historical feedback into a new occurrence.

A retained marker identifier does not prove which revision was used or when. Cross-window recurrence validation re-reads that marker's native user record and compares numeric offsets within the same session: marker before the committed window boundary, cited failure at or after it. A reset cannot supply this ordering. Recovered records are not merged into the current evidence window.

Content review and observed execution outcomes are separate fields. A successful content review permits a scoped prompt change, not a claim of improved execution. The analysis process cannot edit the project. [Evidence-driven execution roles](2026-09-09-evidence-driven-execution-role.md) define the additional recurrence evidence required for a separate working-team role.

## Alternatives considered

**Analyze synchronously before every reply.** This puts model latency and failures on the user's critical path. Durable background work isolates both.

**Resume the selected analysis Agent's live session.** This contaminates its conversation and can collide with active work. An ephemeral process retains account selection without sharing conversational state.

**Rewrite project instruction files automatically.** Shared files can affect unrelated Agents and overwrite user edits. Binding-scoped revisions have an explicit owner and can be cleared or restored without editing those files.

**Treat a self-review as behavioral validation.** The same model can repeat its assumptions. The stored outcome describes later trace evidence separately; content review alone cannot justify execution-loop escalation.

**Accumulate every consumed reference in each prompt.** This grows with transcript length and repeats history the model does not need. Revalidating requested identifiers uses the existing source cursor and bounded report without a second source archive.

**Trust references mentioned in the rolling summary.** Summaries are model output, including in no-change reports. Only an original record inside the committed source boundary can recover a missing citation.

## Consequences

Analysis consumes the selected account's quota and sends source content to its model provider. The settings panel discloses this before opt-in. Raw source windows are not stored in revision history; summaries, evidence references and guidance are. Native reasoning records are excluded.

The implementation supports Claude Code and Codex. Byte-level PTY input is not rewritten; learned context applies to Choruz-owned structured and headless submissions. A missing native source, disconnected device or oversized record leaves the active revision unchanged and exposes a retryable error. Window summaries are lossy model output, so references and subsequent user feedback remain necessary when judging a recommendation.

Historical recovery also covers persisted no-change reports with empty reference lists. It requires the original source to remain available and scoped to the binding; native sources retain the reader's append-only transcript assumption. Missing or truncated history cannot be reconstructed from a summary. Recovery adds source reads for missing citations and historical revision-use markers; it does not change the evaluator prompt, content review or recurrence thresholds. Projected bodies increase the transport cost over identifiers, so the source reader bounds both individual envelopes and the aggregate serialized response. Exceeding the bound rejects the whole recovery instead of silently dropping requested evidence.
