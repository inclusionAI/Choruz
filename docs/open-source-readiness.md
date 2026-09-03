# Choruz Open-Source Readiness Checklist

This checklist records launch blockers separately from the product rename. Completing the rename does not make the repository ready to publish.

## Blockers requiring an owner decision

- [x] Root `LICENSE`: MIT, matching `[workspace.package] license` in `Cargo.toml` and the `license` field of every `package.json`; GitHub detects the repository license as MIT.
- [ ] Obtain the repository owner’s trademark/name-collision review for Choruz.
- [x] Screenshots and documentation reviewed (2026-09-03): the nine `apps/web/public/docs-img` screenshots show only the seeded `operator` principal and fictional conversations. Re-run the review before the external cutover.
- [x] Third-party Pixel World assets: the two runtime tilesets are exact matches to CC0 Ninja Adventure files and are recorded in `assets/THIRD_PARTY.md`; unconsumed sprite collections with unknown or unnecessary provenance are absent from the release tree.
- [ ] AI asset authorization: obtain and retain contributor confirmations for every repository-generated visual asset before the external cutover.
- [ ] Define a security-reporting policy and a supported disclosure contact.
- [ ] Define contribution, governance, and release-owner policies appropriate to the chosen license and publication model.

## Repository and release preparation

- [ ] Complete the Phase 8 external cutover: rename the live GitHub repository, then update the canonical clone URL, repository description, topics, social preview, and verified links.
- [ ] Confirm package names and registry metadata before any publication; this repository currently makes no package-availability promise.
- [ ] Add release notes, versioning, and support expectations only after they are approved by the repository owner.
- [ ] Run a clean-clone installation and all required checks from the final repository URL after the external cutover.

## Phase 7 status

The public repository target and canonical clone URL remain an owner decision. Phase 8 must update clone and remote instructions only after the target exists and its links are verified. This checklist is not authorization to rename the repository, publish packages, reserve names, or add a license.
