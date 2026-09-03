# Real-Harness Platform Smoke

This opt-in acceptance runner exercises real installed Harness binaries through
Choruz. It supports Claude Code, Codex, Pi, Grok Build, and OpenCode. Selecting
an unknown adapter is a configuration error; no Harness has a placeholder flow.
The live path never substitutes a fake binary, mock server, or synthetic reply.

For every selected Harness the runner records all five verdicts before it can
exit:

1. The configured model is returned by the installed Harness's model discovery.
2. Choruz provisions an Agent, a direct message receives a persisted real Agent
   reply, and the native session ID is captured.
3. An operator-provided restart hook crosses a real process lifecycle boundary;
   the next turn must contain prior context and retain the same native ID.
4. A group request produces exactly two separate messages from the expected
   Agent through the bound `CHORUZ_SEND` helper.
5. A native CLI session is created under a nested workspace, its exact ID is
   recursively discovered and imported, the binding retains that exact ID, and
   the resumed reply contains context from before import.

`SKIP` and `BLOCKED` produce exit code 3 when no scenario is `FAIL`; a `FAIL`
takes precedence and produces exit code 1. Missing scenario execution is
initialized as `FAIL`, so a control-flow bug cannot silently omit coverage.

## Safety and cleanup

The runner creates one mode-0700 root under the configured parent. The Company
workspace and private runner artifacts are siblings: seed stdout and stderr are
never stored inside the workspace available to the imported Agent. The retained
report contains fixed verdict/reason codes only.

Platform credentials are held in Node memory and sent with `fetch`; they never
appear in curl/process arguments or artifact files. Child Harnesses and the
restart hook receive a scrubbed environment with every `CHORUZ_SMOKE_*` and
`CHORUZ_REAL_HARNESS_*` value removed. Claude and Codex seed prompts use stdin.
Pi, Grok, and OpenCode currently require a positional harmless generated marker
with their supported headless CLI syntax; no user content or credential is used.

On the normal path, cleanup is part of the verdict. Group conversations are
deleted as their own ledger batch, then Agents are soft-deleted without sending
their already-linked direct conversations through the endpoint a second time.
Both response counts must match exactly. Company deletion must succeed, then
companies, console, conversations, and runtime bindings must contain no tracked
ID, Company workspace, or unique smoke prefix. Cleanup failure is `FAIL`.
Only then is the verified disposable root recursively removed. Harness-native
records are not deleted because the CLIs do not expose a uniform safe deletion
API; they contain only generated acceptance markers and are reported explicitly.

Use a disposable Choruz deployment or dedicated test operator. Do not enable
shell tracing around credentials.

## Static check

```sh
pnpm smoke:real-harness:check
```

This checks the thin Shell wrapper and runs Node tests for the complete scenario
matrix, strict error classification, fail-closed exit behavior, and all five
native session event parsers. It invokes no model or service.

## Live run

```sh
CHORUZ_REAL_HARNESS_SMOKE=1 \
CHORUZ_SMOKE_API_BASE_URL=http://127.0.0.1:30292 \
CHORUZ_SMOKE_WEB_BASE_URL=http://127.0.0.1:3100 \
CHORUZ_SMOKE_SESSION_TOKEN="$CHORUZ_TEST_SESSION_TOKEN" \
CHORUZ_SMOKE_RESTART_HOOK=/absolute/path/to/restart-harness-process \
pnpm smoke:real-harness
```

The restart hook is an executable invoked without a shell. It receives
`{"harness":"...","agent_id":"..."}` on stdin, must perform and wait for a
real Harness/pipeline/connector lifecycle restart, and must print
`{"restarted":true,"before_identity":"...","after_identity":"..."}` only
after recovery. The two non-secret process identities (for example PID plus
start-time) must be non-empty and different. If the hook is absent, only
`restart-resume` is `BLOCKED`; group and import still run independently.

The runner accepts an existing short-lived session token or
`CHORUZ_SMOKE_OPERATOR_USER` plus `CHORUZ_SMOKE_OPERATOR_PASSWORD`.

| Variable | Default | Purpose |
| --- | --- | --- |
| `CHORUZ_REAL_HARNESS_DRIVERS` | `claude,codex,pi,grok,opencode` | Selected real adapters |
| `CHORUZ_{CLAUDE,CODEX,PI,GROK,OPENCODE}_BINARY` | Harness command name | Exact executable |
| `CHORUZ_SMOKE_CLAUDE_MODEL` | `haiku` | Claude model |
| `CHORUZ_SMOKE_CODEX_MODEL` | `gpt-5.4-mini` | Codex model |
| `CHORUZ_SMOKE_PI_MODEL` | `openrouter/openrouter/free` | Pi provider/model |
| `CHORUZ_SMOKE_GROK_MODEL` | `grok-4.6` | Grok model |
| `CHORUZ_SMOKE_OPENCODE_MODEL` | `opencode/mimo-v2.5-free` | OpenCode provider/model |
| `CHORUZ_SMOKE_RESTART_HOOK` | unset | Absolute real restart executable |
| `CHORUZ_REAL_HARNESS_SMOKE_ROOT_PARENT` | user home | Disposable root parent |
| `CHORUZ_REAL_HARNESS_SMOKE_REPORT` | unset | New verdict-only report path |

Pi requires Node 22.19 or newer. An older Node runtime, missing authentication,
or an explicitly empty model is `BLOCKED`; malformed protocol output, a CLI
crash, timeout, non-auth HTTP error, mismatch, or cleanup failure is `FAIL`.

Exit codes are 0 for every requested scenario passing, 1 for a functional
failure, 2 for invalid configuration/opt-in, and 3 for any blocked or skipped
precondition.
