# Agent Note: Separate browser drafts from execution authority

Status: implemented
Archived: 2026-09-21

## Problem

A successful browser trace contains observation-specific targets and private inputs. Replaying it as a reusable automation can click a different control, repeat a mutation after a lost response, or mistake existing page text for success.

## Decision

Extend the existing background program builder with declarative browser drafts. Only referenced training evidence reaches generation; records shared with held-out objectives are excluded. Drafts use exact pages and semantic role/name targets, never recorded refs, scripts or coordinates. The independent check is supplied at execution, not invented by the builder.

A human authorizes each bounded manifest and provider disclosure. [Bounded reuse](2026-09-21-bounded-browser-reuse.md) defines the separate decision for delegating subsequent runs. The existing runtime-host transport dispatches to the selected device. That device owns an isolated BrowserSkill session, fresh target verification, a deadline and cancellation cleanup. The database admits each run ID once and retains its result; a server-keyed HMAC binds private inputs without storing their plaintext. UI receipts survive panel reopening without resubmitting actions. Cancellation prevents late success from replacing the cancelled receipt.

## Alternatives considered

**Activate generated browser programs like finite answers.** Their mutations require additional authority and live outcome evidence. The [finite-program decision](2026-09-20-structured-decision-programs.md) remains authoritative for answer-only evaluation and selection; browser drafts do not enter that active pointer.

**Repeat on a lost response.** A tool may have acted before transport failed. Keep a durable receipt and require inspection instead of automatic replay.

**Accept the model's completion claim.** Confidence is not evidence of a state transition. Require fresh observations and independently specified expected text absent at baseline, while exposing the limited strength of that check.

## Consequences

The browser subset supports clicks, text fields and fixed-value native option selection, not arbitrary generated scripts or authentication. Optional native assistance can generate only explicitly named text fields from caller instructions; it cannot generate selectors, option values, additional actions or success checks. This separates content generation from execution authority. Invalid generated fields stop before a browser session opens. Receipts expose field names, elapsed time and provider-reported tokens without retaining generated text or pretending to measure native Harness usage.

Exact page and label constraints can hand tasks back when a site changes. URL checks are not a network sandbox. Draft creation does not establish generalization or authorize automatic reuse. A cancelled in-flight mutation can have effects; an unavailable device acknowledges cancellation only after reconnection or reaches the execution deadline.
