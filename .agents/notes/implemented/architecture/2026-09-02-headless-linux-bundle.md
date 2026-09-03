# Agent Note: Ship the headless host as a complete Linux bundle

Status: implemented

## Problem

`choruz` starts its adjacent `choruz-server`, but `choruz-server` also starts
the adjacent `choruz-api-gateway` and `choruz-pipeline` and requires migrations. A
pair of binaries could therefore pass a superficial CLI check yet fail during
host startup. The embedded PostgreSQL dependency also selected native TLS by
default, preventing a self-contained musl cross-build.

## Decision

The headless Linux bundle keeps `choruz`, `choruz-server`, `choruz-api-gateway`,
`choruz-pipeline`, and `migrations/` in one binary directory. The supervisor
uses the source workspace while developing and uses that complete binary
directory when launched from a bundle. `choruz-supervisor` selects the Rustls TLS
backend for embedded PostgreSQL so the headless binaries can target musl
without a target-system OpenSSL dependency.

## Alternatives considered

**Ship only `choruz` and `choruz-server`.** This loses the two backend
children that the server owns, so startup cannot reach its readiness handshake.

**Require a source checkout on the remote host.** This makes the binary bundle
dependent on Cargo and the repository layout, defeating the low-resource host
installation path.

**Keep native TLS for embedded PostgreSQL.** It requires a target-compatible
OpenSSL toolchain and dynamic libraries, which conflicts with the static musl
artifact contract.

## Consequences

The bundle is larger than two files, but it is an executable deployment unit
with no glibc or OpenSSL runtime dependency. The server continues to download
its embedded PostgreSQL runtime on first launch, so that initial setup still
requires outbound network access.

## Testing

`crates/choruz-supervisor/src/supervisor.rs` pins complete-bundle working-directory
resolution and rejects incomplete bundles before backend startup.
