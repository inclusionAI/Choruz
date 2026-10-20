# Agent Note: Pin embedded PostgreSQL and preserve incompatible data

Status: implemented

## Problem

An unbounded upstream version requirement lets identical Choruz packages install different PostgreSQL major versions. Losing an installation cache while retaining data can then select a server that cannot read that data.

## Decision

The supervisor selects the exact 16.15.0 distribution, rather than the latest upstream release. Before downloading or starting it, the supervisor reads `pgdata/PG_VERSION`. Major version 16 is accepted; another value or unreadable metadata stops startup without changing the data. An absent version file permits first initialization. The error requests a backup using the matching server and a compatible restore plan, never deletion or automatic downgrade.

The [headless bundle](../architecture/2026-09-02-headless-linux-bundle.md) and [desktop runtime owner](../architecture/2026-09-29-desktop-runtime-owner.md) decisions remain active: they own packaging and lifecycle, not distribution selection. Their first-launch network requirement still applies.

## Alternatives considered

**Select the newest release in a supported major range.** This preserves the data format but still changes native distribution requirements between identical installs. A reviewed exact pin makes that dependency explicit.

**Use a matching major found in existing data.** This silently expands the supported runtime set and leaves first installation dependent on upstream defaults.

**Reinitialize incompatible data automatically.** A startup retry must not destroy user history or treat an unsupported downgrade as a migration.

## Consequences

Updating the pin is a deliberate release change. The installation test starts an isolated downloaded PostgreSQL distribution, writes a row and verifies it after restart; it requires network access for its private cold-install directory. Unit tests independently reject upstream drift and incompatible data without modifying the version file. The pin does not make the installer offline-complete or certify every target operating system's native-library baseline.
