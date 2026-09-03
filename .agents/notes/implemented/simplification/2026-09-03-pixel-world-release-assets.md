# Agent Note: Keep only shipped Pixel World assets

Status: implemented

## Problem

Pixel World carried several sprite collections and loaders from rendering systems that the Phaser scene does not consume. Shipping those files increased browser requests and made the public asset provenance larger than the actual product surface. The active AI-assisted scenes and characters also lacked a single release inventory.

## Decision

Pixel World keeps the two floor backgrounds and masks, the packed legacy agent atlas, the 20 roster sheets and the two CC0 Ninja Adventure tilesets used by the procedural floor fallback. Legacy agent descriptors address atlas frame names rather than paths to unpacked source sheets. Initialization loads only the fallback tilesets; Phaser preloads the active floor and character textures itself.

`assets/THIRD_PARTY.md` is the source of truth for redistributed third-party artwork. Repository-generated visual assets require contributor authorization before public release.

## Alternatives considered

**Regenerate both floor scenes.** Rejected because the backgrounds, walkability masks, elevator triggers and spawn coordinates share an exact 2158 x 1984 coordinate system; stochastic regeneration would require rebuilding and retesting the navigation surface.

**Retain every design-stage asset.** Rejected because unconsumed source sheets, emotes, creatures, props and debug overlays add provenance obligations without providing runtime behaviour.

**Publish the images under the repository-wide MIT license without an asset record.** Rejected because visual artwork has different provenance and attribution needs from software, and the current release still requires contributor authorization for its AI-assisted outputs.

## Consequences

The browser avoids loading sprite registries and furniture images that have no renderer. The release tree contains only the visual assets needed by the current Phaser path and its procedural floor fallback. Public cutover remains blocked until the AI asset contributors provide durable authorization.
