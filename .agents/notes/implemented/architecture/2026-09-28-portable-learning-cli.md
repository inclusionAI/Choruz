# Agent Note: Share bounded native learning execution

Status: implemented

## Problem

The learning procedures accept an injected runner, but using the platform's actual Claude Code or Codex runner requires the whole device host and an interactive terminal specification. A consumer needs PTY dependencies and irrelevant session fields for an isolated background call.

## Decision

`choruz-learning` owns the existing runner behind the permanent optional `native-cli` dependency boundary. Its configuration contains only executable, driver, model and account selection. The host enables this capability and converts its device request into that configuration; there is no second implementation of process execution or result validation.

Device executable resolution belongs to `choruz-agent-runtime::executable`, shared by terminal, session and learning callers. Explicit paths and environment override precedence remain unchanged.

## Alternatives considered

**Implement another lightweight runner.** Rejected because tool restrictions, refusal handling, budgets and process cleanup would diverge.

**Depend on the whole host runtime.** Rejected because pure learning consumers do not need PTYs or remote device dispatch. Consumers with their own runner need neither native dependency.

## Consequences

Native execution keeps its scratch conversation, account environment, bounded output and cleanup behavior. It does not install or authenticate a CLI, collect traces, schedule analysis, save outcomes or activate guidance. Platform authorization and durable dispatch remain at their existing owners. Subprocess fixtures verify adapter behavior without claiming live model quality.
