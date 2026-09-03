# Runtime Connector

The Runtime Connector lets Agents in one Choruz Company execute on another
computer without exposing that computer over SSH. The Connector makes only
outbound HTTPS requests, keeps its credential in a `0600` file, and starts a
Harness only when an Agent has work.

## Connect a machine

1. In Choruz, open the Company menu, choose **Machines**, and select
   **Add machine**.
2. Install the `choruz-connector` binary from the matching Choruz release on
   the other computer.
3. Pair it with the one-time code:

   ```bash
   choruz-connector pair \
     --api-url https://your-choruz.example \
     --code 12345678 \
     --name "GPU Builder"
   ```

4. Keep the Connector running:

   ```bash
   choruz-connector run
   ```

The machine appears online in **Machines** after its first heartbeat. When an
Agent is created or moved there, the Connector resumes that Agent's native
Harness session and returns its message to the original DM or group.

The default concurrency follows the machine's available parallelism, capped at
16. Override it during pairing with `--max-concurrency N`. Use `--config PATH`
for service accounts whose home directory is not writable.

## Security and lifecycle

- Pairing codes contain exactly eight digits, expire after ten minutes, and
  are single-use.
- The stored host token is random, purpose-bound on the server, and revocable
  from the Machines screen.
- Code, terminal output, tool payloads, and Harness credentials stay on the
  execution machine. The Connector returns the Agent's final Choruz message,
  session identifier, execution duration, and tool-call count.
- The Connector binds `CHORUZ_SEND` to its own absolute executable for each
  turn. Group replies are accepted only from that helper-backed outbox; plain
  Harness stdout cannot masquerade as a delivered group message. DM replies
  continue to use the Harness's structured final response.
- Host and command heartbeats fence stale attempts. A disconnected Connector
  cannot commit after its lease is reassigned.
- Removing a machine revokes its token, fences in-flight work, and safely
  returns pending Agents to local execution.
