# Agent Note: Remote access disclosure

Status: implemented

## Problem

Claiming that terminal output and files stay local misrepresents a remote dashboard that can receive those bytes. Keeping execution on a device does not keep every displayed datum there.

## Decision

Remote Control distinguishes a browser opening this computer's dashboard from another computer becoming a Company device. The connection description identifies messages, terminal/tool output and files as possible remote data while stating that Agent processes stay on their computer.

## Alternatives considered

**Describe only encrypted messages.** The relay transport also multiplexes HTTP bodies and terminal streams, so that description understates the access granted.

**List every permitted API path in the dialog.** The executor owns path authorization. The UI explains the data categories rather than duplicating its routing policy.

## Consequences

The copy does not change encryption or access. `relay-session.ts` encrypts relay envelopes; `relay-transport.ts` carries HTTP and stream bytes, and `remote_control_executor.rs` validates their destinations. A browser test checks both connection purposes and the disclosure without creating credentials or making an external connection.
