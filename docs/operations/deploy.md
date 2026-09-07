# Verified deployment

Choruz delivers the artifact tested by CI, not a rebuild of a moving branch. Cloud Gateway deployment is automatic in the designated repository; device upgrades remain an explicit operator action because restarting the pipeline interrupts running Agents.

## CI artifacts and publication

The `Release packaging` job in [ci.yml](../../.github/workflows/ci.yml) builds the five host binaries, migrations and standalone Next.js application. It runs on `main` and on PRs changing CI or operations. It extracts the archive and starts its API, pipeline and web processes against a disposable PostgreSQL instance before retaining `release-<commit>` for 30 days.

[cd.yml](../../.github/workflows/cd.yml) accepts only a successful same-repository `push` to `main`, with successful required CI and packaging jobs. It verifies the archive checksum, file manifest and source revision, then publishes a `build-<commit>` prerelease. Existing assets must match byte-for-byte; delivery never replaces them. A manual dispatch takes the successful CI run ID and enforces the same checks. PR and fork artifacts are not deployable.

The hosted build targets Linux x86_64 on Ubuntu 24.04. It requires Node 24 and an operator-managed PostgreSQL installation. It is not a portable glibc 2.32 or NAS bundle and does not include PostgreSQL executables. The included `choruz-server` uses its own embedded-PostgreSQL bootstrap; use the managed API/pipeline services for an external database.

## Cloud Gateway delivery

Only the repository with `CHORUZ_CLOUD_DEPLOY_ENABLED=true` deploys the shared gateway. Configure the `cloud-production` GitHub environment with:

- Secret `CLOUDFLARE_API_TOKEN`: Workers Scripts write and D1 read permissions for the gateway's account. Do not use a developer's interactive OAuth credential.
- Variable `CLOUDFLARE_ACCOUNT_ID`: the account owning the Worker.
- Variable `CHORUZ_GATEWAY_URL`: its HTTPS origin.

Restrict the environment's deployment branches to `main`. Mirrored repositories can publish their own artifacts without receiving cloud credentials or the enablement variable. Rotate an expiring API token before its expiry; authorization failures stop delivery rather than falling back to another identity.

The deployment checks that the Durable Object migration tag and applied D1 migration filenames match the checked-out release. A mismatch requires a separately reviewed storage rollout; CD does not apply or undo database migrations. It uploads a uniquely tagged Worker version, verifies that the deployed baseline did not change, then promotes that version. Health checks require the exact Cloudflare version ID and a working anonymous Online session route. A failed promotion restores and checks the previous version, unless another deployment has taken ownership. Concurrent CD activations are serialized and are not cancelled by newer pushes.

The `gateway-delivery-<run>` artifact records the source revision, previous/target version IDs and final recovery phase. A failed or superseded deployment is a failed GitHub job, including when recovery succeeds. Use that job and Cloudflare deployment history for diagnosis. This is deployment verification, not a substitute for ongoing availability alerts or a database backup.

## Managed device upgrades

Download the archive and checksum from the desired prerelease. Keep writable state outside the extracted release: database storage, attachments, account credentials and Agent workspaces must survive a version change. The [systemd](../../infra/ops/systemd/) and [launchd](../../infra/ops/launchd/) files are templates for an already configured service account, persistent working directory, environment and log directories. Supply production secrets and the correct database URL before starting those services.

The release helper requires Python 3.12 or newer. From the repository root, its entry points are:

```bash
python3 infra/ops/release.py --help
python3 infra/ops/verify-archive.py /path/to/choruz.tar.gz COMMIT_SHA
python3 infra/ops/release.py verify /path/to/extracted-release
```

For a managed installation using the template ports, activate an extracted release with all three readiness URLs:

```bash
python3 infra/ops/release.py deploy /path/to/extracted-release \
  --releases /srv/choruz/releases \
  --health-url http://127.0.0.1:3000/readyz \
  --health-url http://127.0.0.1:3020/readyz \
  --health-url http://127.0.0.1:3100/docs
```

Use your actual ports and release directory, including `/Users/Shared/choruz/releases` for the macOS templates. The helper verifies hashes, executable permissions, platform and libc baseline before atomically replacing `current`. It restarts the managed services and waits for readiness. On failure it restores and checks the known-good release; on first-install failure it stops services because no rollback target exists. A host-wide activation lock rejects overlapping upgrades. A manual `rollback` uses the same options and the verified `previous` release.

Back up and check schema compatibility before activation. Binary rollback does not undo migrations, and the helper does not promise recovery from power loss, a killed deployment process or an incompatible database change. Preserve the failed run's service logs and use [the incident runbook](runbook.md) when automatic restoration fails.

For local packaging, `pnpm release:package` requires a clean tracked checkout, Rust and pnpm. It writes a commit-addressed release, manifest, archive and checksum without changing `current` or `previous`. Build without production secrets; environment files are excluded and rejected by verification.
