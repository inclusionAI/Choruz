---
name: choruz-record-demo
description: Record and verify a truthful Choruz product demo or browser workflow video, including real local and remote Agent interactions. Use for requested recordings, not as an automatic media requirement for every GUI pull request.
---

# Record a Choruz demo

Produce a local, watchable demonstration of verified product behaviour. The [evidence-chain decision](../../notes/implemented/process/2026-09-06-demo-recording-evidence-chain.md) owns the provenance and publication boundaries.

## Establish the demonstrated build

Record the exact commit, checkout, serving origin, runtime mode and device revisions before capture. Use the repository's existing startup workflow after inspecting its configuration; do not improvise a second stack or overwrite a user's running workspace. A remote demonstration identifies both ends and the actual transport.

Use dedicated demonstration groups, workspaces and benign prompts. Prefer isolated browser state; when the user requests an existing authenticated browser, preserve it and record that exception. Never clear unrelated cookies, sessions or data to stage a demo.

Complete functional acceptance before recording a polished walkthrough. Real-product claims require real servers, transports and model rounds, not fixture agents, injected events or mocked responses. Keep failures visible in acceptance evidence even when the final showcase uses a subsequent successful run.

## Plan and capture

Choose a compact story at the user's requested duration. For a two-minute overview, give each feature enough time to show an action and its observable result; omit features that cannot be demonstrated honestly. Keep a fixed, readable viewport and capture only the intended application region.

Discover the available browser or native recording capability and read its instructions before using it. Use the supported CUA surface for user interaction. Prefer continuous capture for terminal output and animation. State-based screenshot encoding is suitable for a short edited walkthrough, but label it as such rather than claiming real-time video.

Each scenario's frames must come from one causal run. Do not splice an old successful response into a newer failed interaction. Separate scenarios may become explicitly separated chapters, with their provenance retained. Wait for concrete, uniquely identified UI results rather than using elapsed time or an echoed prompt as proof. To demonstrate an Agent tool action, show its identity, status and result; a spinner or final chat sentence alone is insufficient.

Keep credentials, pairing codes, private conversations, unrelated browser tabs and notifications outside the recording. Stage safe demo content instead of editing the product's rendered content. Stop capture before entering authentication material. Retakes, omitted waits, speed changes and chapter transitions are editorial changes, not evidence that the original operation was faster.

## Encode and verify

Store raw captures and output outside tracked source paths. Use the available media encoder for the requested format; MP4 is appropriate for a two-minute video, while GIF suits short loops. Preserve readable text and enough final-state hold time. Do not replace an existing artifact without resolving its exact path.

Inspect the encoded artifact itself: verify duration, dimensions, frame count and file size with media probing, then play it or decode representative frames from its beginning, every chapter transition and its ending. Check crop, ordering, text legibility and sensitive content. Source screenshots alone do not verify the encoded result.

Deliver the absolute local artifact path with a concise provenance summary: demonstrated commits, local/remote setup, actual model execution, and any cuts or state-based capture. A demo supports only the scenarios it shows; it does not replace automated tests or the separate acceptance report.

## Publication is separate

Recording does not authorize uploading media or editing a PR. Publish only when requested, verify the uploaded bytes and access path, and ensure the destination still refers to the demonstrated revision. Never commit recordings to main or a feature branch that will merge into main. Keep source-controlled skill instructions separate from generated media.
