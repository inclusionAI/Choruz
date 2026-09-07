# choruz-host-runtime

Everything Choruz does on the machine where a Harness runs, written once so the API gateway can do it for its own device and `choruz-connector` can do it for a paired one: spawn and pool interactive terminals, provision a binding-owned Codex home and attribute the session file it writes, find the newest native session for a workspace, scan session catalogs and list directories inside the configured browse roots.

`HostRequest` is the request vocabulary; `execute` runs one request on the current device. The gateway's `RuntimeHost` sends the same requests in-process for its own device and over the host link for a remote one, so a device feature is written against this crate exactly once. See [docs/subsystems/agent-runtime.md](../../docs/subsystems/agent-runtime.md).

Managed Codex home preparation receives the binding's account metadata. The executing device resolves its isolated profile under its own account root; the controller never supplies an absolute account-home path.

Directory listing also serves the gateway's filesystem route. Symlinks use their target's type only when the canonical target lies within the configured browse roots; broken and out-of-root links are omitted. Entry paths retain the alias, and navigating into it revalidates the target.

Terminal attachment atomically subscribes to live PTY bytes and captures replay. The first attachment receives up to 64 KiB of raw startup output; later attachments, or startup overflow, receive a bounded snapshot of the visible screen, cursor and input modes. Reconnection does not restart the CLI or retain an unbounded terminal transcript. Live output remains the CLI's original bytes.

Terminal creation and resize accept 1–512 columns and 1–256 rows. Invalid dimensions return a validation error before allocating a screen or changing the PTY.

`DriverCatalog` inspects harness executables and default-profile models on this device. The web-facing `drivers.inspect` operation uses the existing authenticated host link; explicit harness-account models remain owned by the account probe. Custom workspace paths are validated by the device that provisions them, not against the controller's home directory.

## Structured conversations

`session::execute` owns Claude Code stream-JSON and Codex app-server processes.
`Prepare` reserves a Claude session in the journal without spawning; `Ensure`
starts or reuses the binding-owned process. `Read` returns revision-cursor pages.
Commands require both the binding owner and the current process instance.
The same requests travel through `LinkRequest::Session` on remote devices.

The browser may reconnect without restarting the process. Closing the session
stops its process before a raw terminal can open; native history restores the
conversation when switching back. Gateway and connector graceful shutdown close
their structured processes. Missing native history is an explicit error, not a
silently replaced session. Claude permission prompts use the SDK's stdio host
channel, so a requested approval is presented rather than automatically denied.
Codex starts paginated native history and resumes with the most recent 20 turns
using `initialTurnsPage` with `itemsView: full`. The returned native projection
replaces the saved preview: reconstructed item IDs need not equal live event
IDs. An older-history cursor triggers the same visible truncation notice.

Journals under the workspace's `.choruz/sessions/` contain transcript content,
submission IDs and native identity. Writes use owner-only files and atomic
replacement, with an 8 MiB write ceiling. They are session data, not telemetry. The selected account's native
store remains the resume source; credentials are not copied into the journal.

Protocol lines are limited to 8 MiB and pending interactions to 64 KiB. Each
transport page has a 192 KiB budget, with bounded text and detail previews.
Full native tool output remains in the Harness's session store. The live
transcript and journal retain a rolling preview of at most 512 items and 4 MiB
of serialized items. The browser discards evicted positions and displays a
truncation notice instead of presenting the preview as complete history.

Claude history reads retain at most 2,048 source entries and 32 MiB of source
JSON before projecting the active chain. Each session accepts at most 4,096
distinct submission IDs; only message fingerprints are stored for deduplication.
Reaching that limit rejects further Conversation submissions explicitly; Terminal
can continue the native session without dropping the duplicate-delivery guard.
