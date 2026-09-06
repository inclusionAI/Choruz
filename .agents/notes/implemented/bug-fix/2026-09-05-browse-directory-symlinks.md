# Agent Note: Browse directory symlinks through one filesystem owner

Status: implemented

## Problem

Directory listings classified symlinks from the link's file type, so directory links disappeared from folder pickers and appeared as files in the explorer. The gateway and device runtime maintained separate listing implementations.

## Decision

The gateway delegates authenticated directory listings to the host-runtime filesystem owner on a blocking worker. That owner canonicalizes a link target, checks it against canonical browse roots, and uses the target's file type. Broken and out-of-root links are omitted. Returned entry paths retain the alias; navigation validates the target again.

The shared listing retains the gateway's 500-entry cap, hidden/file filtering, Unicode lowercase sorting, parent-root boundary and not-found classification for unreadable directories. Device requests use the same owner and result type.

## Alternatives considered

**Patch both listing loops.** This leaves filtering and browse-root rules with two owners and invites the next divergence.

**Follow every symlink.** Directory links can point outside the permitted roots; their targets must pass the same boundary as direct navigation.

## Consequences

Allowed directory aliases can be browsed through either entry path. Symlink file types are resolved only for allowed targets. Shared-owner tests use explicit roots without mutating process environment, and an authenticated HTTP regression verifies visible aliases, navigation, file filtering and refusal of forbidden targets using owned directories.
