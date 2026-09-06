# Agent Note: Interaction reads reuse committed records

Status: implemented

## Problem

Behaviour analysis needs to join a user's submitted message to execution and
reply records. Copying those records into browser telemetry creates competing
content stores and exposes text through a lower-trust ingestion surface.
Terminal bytes do not distinguish a submitted prompt from a password or draft.

## Decision

`DbService::list_interactions` projects `conversation_events`, `agent_commands`
and `agent_results` through a human-only, conversation-membership-gated read.
It returns an exclusive sequence cursor and explicit sender role, trace, turn
and attempt identifiers. Content is omitted unless requested. Execution prompts,
raw errors and tool payloads are excluded even from content-enabled reads.
Every successful read writes `interaction.read` through the existing audit owner.

Terminal activity uses the same gateway bridge for local and linked devices.
Attachment records share an identifier; detach records include transferred byte
counts, successful resizes and duration. REST submissions have separate started
and finished records. Those outcomes describe terminal writes, not model success.
The two bridge tasks are stopped and joined before final counters are recorded.

## Alternatives considered

**Copy message text into telemetry.** The committed event already owns the text
and retry identity. A second copy complicates deletion and privacy controls.

**Reconstruct prompts from PTY keystrokes.** Terminal applications handle editing,
bracketed paste and authentication differently. Guessing submission would capture
drafts or credentials. Native CLI transcripts remain owned by their Harness;
this projection does not claim to collect their text.

**Make terminal use depend on analytics storage.** A recording failure must not
turn a successful write into a retry that sends duplicate input. The gateway logs
terminal persistence failures and preserves the actual operation result.

## Consequences

Message metadata and execution outcomes can be analysed without reading content.
Content-enabled reads expose the already-authorized conversation text and are
audited. Historical sender roles reflect the remaining principal row; absent
principals return null, not an inferred role. A missing command or attempt is an
empty list, not proof that an agent never acted.

Terminal records contain no text and do not feed the conversation pipeline.
Transferred output counts can include screen replay; they are not token usage.
A gateway crash can leave an attachment or submission without a finish record.
Terminal telemetry failures are observable but not transactionally coupled to the
remote process. The committed-message read does not backfill raw native CLI DMs.

## Testing

Gateway integration tests cover pagination, denied membership, default content
exclusion, opt-in content, execution retry projection and read auditing.
Browser tests join a sent message's request trace to the projected record.
A real connector and PTY on distinct device roots verify terminal failure then
success, byte counters and absence of content. Only the external Harness binary
is replaced with a deterministic echo process; no model execution is claimed.
