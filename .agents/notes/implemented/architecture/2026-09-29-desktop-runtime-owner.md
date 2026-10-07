# Agent Note: Desktop runtime owner

Status: implemented

## Problem

A browser workbench still requires users to start a separate stack and manage its lifetime. The requested downloadable application must keep background work alive when its window closes without duplicating account, task or device behavior.

## Decision

`apps/desktop` is an Electron host for the shared production workbench. It packages the existing Rust binaries and Next.js standalone output, owns their lifecycle and stores desktop data separately. Window close hides the workbench; explicit quit stops its services. Native folder selection is a narrow, local-only preload capability.

This extends [task-first workbench](../feature/2026-09-29-task-first-workbench.md): its shared task and collaboration projection remains authoritative. The desktop shell supersedes only the rejection of a separately packaged application, not the rejection of duplicated business behavior. Background collaboration is an explicit Actions-menu entry.

## Alternatives considered

**A second desktop task implementation.** Separate provisioning, message and account owners would diverge from browser and remote behavior. The shell loads the same application instead.

**A shortcut to the development server.** It cannot start independently or own reliable shutdown, and relies on an external checkout and toolchain.

**Reusing a running local backend.** It risks attaching to unrelated data or stopping another installation. Dedicated ports and private credentials make ownership explicit; conflicts fail rather than killing a listener.

## Consequences

Instrument Sans and Azeret Mono come from pinned Fontsource packages, including
their Latin and Latin Extended subsets. The shared layout bundles their CSS and
font binaries without a build-time Google Fonts request or generated Google
font module. Public font notices travel with browser and desktop distributions.

Electron adds distribution size but preserves the existing renderer and native runtime contracts. Standalone packaging must retain pnpm-relative symlinks and exclude workspace-local files. macOS arm64 is the packaging target; signing and notarization require release credentials. Embedded PostgreSQL is downloaded on first launch, so the installer is not offline-complete.
