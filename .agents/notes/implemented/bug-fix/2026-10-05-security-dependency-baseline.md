# Agent Note: Keep the runtime dependency baseline patched

Status: implemented

## Problem

The workspace's locked Next.js, Axios and Fastify releases have published high or critical advisories that the required security scan rejects. Application behavior tests cannot detect those library defects.

## Decision

The web package pins Next.js 16.3.6, the shared Axios override pins 1.20.0, and the bridge requires Fastify 5.12.2 or a compatible later patched release. The workspace lockfile fixes the resolved graph. Verification uses the existing security scan, web build and browser acceptance, and bridge tests.

Patch selection follows the [Next.js advisory](https://github.com/advisories/GHSA-vcvr-r3jv-pc5j), [Axios release](https://github.com/axios/axios/releases/tag/v1.20.0) and [Fastify security release](https://github.com/fastify/fastify/releases/tag/v5.12.2).

## Alternatives considered

**Ignore advisories because a particular application path is unused.** This weakens the shared dependency gate and leaves consumers exposed when their application paths change.

**Upgrade every dependency to its latest release.** Unrelated changes expand the verification scope without resolving an identified additional defect.

## Consequences

The affected runtime graph is tested on its patched releases. No new advisory exception or scan bypass is introduced. Future advisories require a new scan and an appropriate dependency correction rather than treating this baseline as permanently secure.
