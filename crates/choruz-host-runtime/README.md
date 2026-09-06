# choruz-host-runtime

Everything Choruz does on the machine where a Harness runs, written once so the API gateway can do it for its own device and `choruz-connector` can do it for a paired one: spawn and pool interactive terminals, provision a binding-owned Codex home and attribute the session file it writes, find the newest native session for a workspace, scan session catalogs and list directories inside the configured browse roots.

`HostRequest` is the request vocabulary; `execute` runs one request on the current device. The gateway's `RuntimeHost` sends the same requests in-process for its own device and over the host link for a remote one, so a device feature is written against this crate exactly once. See [docs/subsystems/agent-runtime.md](../../docs/subsystems/agent-runtime.md).

Managed Codex home preparation receives the binding's account metadata. The executing device resolves its isolated profile under its own account root; the controller never supplies an absolute account-home path.

Directory listing also serves the gateway's filesystem route. Symlinks use their target's type only when the canonical target lies within the configured browse roots; broken and out-of-root links are omitted. Entry paths retain the alias, and navigating into it revalidates the target.

Terminal attachment atomically subscribes to live PTY bytes and captures replay. The first attachment receives up to 64 KiB of raw startup output; later attachments, or startup overflow, receive a bounded snapshot of the visible screen, cursor and input modes. Reconnection does not restart the CLI or retain an unbounded terminal transcript. Live output remains the CLI's original bytes.

Terminal creation and resize accept 1–512 columns and 1–256 rows. Invalid dimensions return a validation error before allocating a screen or changing the PTY.

`DriverCatalog` inspects harness executables and default-profile models on this device. The web-facing `drivers.inspect` operation uses the existing authenticated host link; explicit harness-account models remain owned by the account probe. Custom workspace paths are validated by the device that provisions them, not against the controller's home directory.
