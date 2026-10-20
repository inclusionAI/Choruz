# Agent Note: Share learning evidence resolution and execution identity

Status: implemented

## Problem

Analysis and behavior-card preparation independently resolved private feedback and native sources. Shared-task analysis followed the learning profile and retained source ledger, but card preparation selected only the root Agent and its foreground native home. Separate execution fingerprints also disagreed about profile-directory changes.

## Decision

The application's `learning_feedback` owner supplies current shared-conversation scopes and committed-reference recovery to analysis and behavior preparation. Both include completed finite decisions. The gateway's `learning_sources` owner resolves authorized source bindings and retained session homes for both consumers. A retained cursor identifies progress, not authorization: membership, effective learning binding and current account/device/workspace scope are checked again.

`choruz-host-runtime::execution_identity` defines credential-free driver, executable, workspace, model, device, account, profile kind and directory fields. Analysis continuation, program reservations and evaluation use this identity while retaining their own procedure digests and reservation scope. Terminal attachment metadata and credentials are excluded.

Publication remains a separate consent and privacy-review path. Sharing local learning history does not authorize publishing it. Existing generation, claim, cursor and uncertain-call checkpoints remain their owners' responsibility; a common identity is not an authorization token.

## Alternatives considered

**Keep a narrower community evidence reader.** Card extraction still needs the same verified source underlying the observation; silently losing inherited-task evidence does not strengthen publication consent or independent privacy review.

**Recover from saved analysis prose.** A previous analysis cannot establish current access to the original evidence or its native account attribution.

**Hash the entire TerminalSpec.** Terminal geometry and attachment changes would invalidate unrelated learning, while arbitrary account configuration could introduce credential material. A bounded identity shares only stable execution inputs.

**Use one fingerprint for all stages.** Each stage has different fixed procedures and durable reservation owners. Shared fields do not make those scopes interchangeable.

## Consequences

Behavior cards can recover source evidence from shared tasks and retained native profiles without importing foreground sessions. A changed execution identity fences pending work; callers retain their actionable restart behavior. Source resolution adds no model calls and creates no new persistent source copy. Native-source containment and projection still belong to the existing host and learning libraries.
