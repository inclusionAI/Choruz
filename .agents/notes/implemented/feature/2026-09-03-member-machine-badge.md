# Agent Note: Members show the machine an agent runs on

Status: implemented

## Problem

A company can spread its agents over several machines (`runtime_host` rows paired through `choruz-connector`), and a group can mix agents from any of them. The only place the UI named an agent's machine was the DM header subtitle, so a group member list gave no sign of where each agent would execute, and a person choosing whom to mention could not tell a local agent from one on a remote host.

## Decision

`bindingMachineLabel` in `apps/web/lib/terminal/terminal-bindings.ts` names a binding's machine (`This computer`, the paired host's name, or `Remote machine` when the host list has not loaded); the DM subtitle and the detail panel share it. Every agent row in the Members section, for DMs and groups, carries the name as a badge next to the AI badge and as a `Machine` field in the expanded detail. The hosts come from the runtime host list `ChatApp` already loads for the active company, handed down through `ChatModals` to `DetailPanel`.

## Alternatives considered

- **Show the machine only in the expanded detail**: rejected. The point is to see at a glance which members run where; a disclosure hides it.
- **Load hosts inside `MemberRow`**: rejected. One list per company already exists in `ChatApp`; a fetch per row would repeat it for every member.

## Consequences

- Group and DM member lists name each agent's machine; a company with only local agents shows `This computer` on every agent.
- `bindingMachineLabel` is unit-tested; the badge is plain markup inside the existing `.member-row`.
