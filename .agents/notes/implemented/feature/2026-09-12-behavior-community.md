# Agent Note: Evidence-backed behavior community

Status: implemented

## Problem

Binding-scoped learning retains observed problems and guidance, but other Agents cannot discover the experience. Sharing native transcripts would expose private work and still leave readers to reconstruct which intervention was actually used. Counting repeated analysis reports as failures would also misrepresent model behavior.

## Decision

The existing problem and revision owners retain versioned behavior cards, solution references and independently identified evidence events. Local and public records use the same bounded, typed payload. Private episode references and raw context remain separate database fields and never enter a public serializer. Model attribution distinguishes observed execution metadata from configured and analysis models; missing observed metadata remains absent.

The existing asynchronous analysis worker prepares cards from verified source records. It searches local experience before accepted, cached community records. Community content is untrusted input to the existing fixed review and evaluation pipeline, never executable installation instructions. Applying a solution produces a revision with its source version; subsequent independent work supplies effectiveness or recurrence evidence. Content review alone is not effectiveness evidence.

Community search, automatic trial and redacted contribution have separate owner-scoped consent. Disabling networking does not disable local learning. Public contributions undergo bounded extraction and an independent privacy review before entering a durable outbox. Failed or inconclusive privacy checks block publication, not local analysis.

The public Hugging Face dataset stores immutable record files and accepts contributions as pull requests. Accepted repository revisions populate a local search cache; private search text is never sent to Hugging Face. Pending contributions are distinct from accepted records. No remote counter is incremented directly: aggregate counts deduplicate evidence identities across imports and retries. A claimed occurrence is not a measured failure rate.

## Alternatives considered

**Create another local learning database.** This would duplicate the problem, recurrence and revision owners and disconnect community application from rollback and fixed evaluation.

**Upload redacted transcripts.** Redaction alone does not make a transcript a reusable problem. Typed experience records expose only the minimal contextual example and explicit evidence needed for a scoped intervention.

**Install the most popular community skill immediately.** Popularity does not establish applicability, safety or effectiveness for the receiving Agent. Existing review, owner consent and versioned application remain mandatory.

**Update a shared occurrence counter.** Retries and re-analysis inflate it, while concurrent writes lose events. Immutable evidence identities support reproducible aggregation and correction.

## Consequences

Semantic matching and redaction are fallible model decisions. Retain their evidence and versions and reject uncertain publication. Public reports are contributor claims, not independently certified measurements. The dataset review boundary and local evaluation protect application but do not establish a population-wide model ranking.

Public publication uses a server-managed Hugging Face identity and requires a configured publisher token. An interrupted dispatched request remains uncertain instead of creating another pull request on retry. Blocked undispatched preparation can be retried with fresh privacy review. An accepted record must match the exact public payload; a pending label means acceptance is unobserved, including a contribution that a maintainer closed without accepting. Unpublished local cards can reconcile with a reviewed community match, but published evidence is not rewritten to merge identities.

The [background learning](2026-09-08-background-experience-learning.md) and [measured application](2026-09-09-measured-learning-application.md) decisions retain their independent ownership and validation constraints. Community experience supplies candidate guidance, not another activation or evaluation system.
