# Signal Chorus brand assets

This directory contains the original, editable SVG source artwork for Choruz's
Signal Chorus identity. The mark is three parallel, open C-like outward
signal strokes that read as one chorus silhouette. The strokes use exactly the
approved navy (`#102A6B`), mint (`#14B8A6`), and sky (`#38BDF8`) palette.
The monochrome source preserves the same geometry for one-ink and small-size
uses.

## Files

- `signal-chorus-mark.svg` — canonical transparent mark for light surfaces.
- `signal-chorus-mark-dark.svg` — navy-field variant for dark surfaces and app icons.
- `signal-chorus-mark-mono.svg` — transparent one-ink fallback.
- `signal-chorus-lockup.svg` — primary mark and custom outlined `CHORUZ` lettering.
- `signal-chorus-lockup-dark.svg` — bright-lettered lockup for dark product surfaces.
- `generate-assets.sh` — deterministic macOS generator for the derivative files.
- `generated/` — generated derivatives; do not edit by hand.

## Production sizes

The generator creates the web PNG sizes `16`, `32`, and `180` pixels.

```sh
# Refresh generated preview artifacts only.
./assets/brand/generate-assets.sh

# Refresh and deploy the checked-in application icon targets.
./assets/brand/generate-assets.sh --apply
```

`--apply` is the only supported way to update the web favicon/Apple-touch
variants and dark-surface product lockup. Those files
are generated outputs and must not be edited by hand.

The script requires macOS's built-in `qlmanage`, `sips`, and Node
18+; it uses no downloadable renderer or design package. Its editable vector
inputs are `signal-chorus-mark.svg`, `signal-chorus-mark-dark.svg`,
`signal-chorus-lockup.svg`, and `signal-chorus-lockup-dark.svg`. It rasterizes
`signal-chorus-mark-dark.svg` because the navy rounded-square field is required
for standalone app icons.

## Light and dark usage

Use the canonical light variant on cloud or other light backgrounds. Use the
dark variant on navy or other dark backgrounds. The dark variant uses
`#5170B8`, a documented brighter night-mode navy tint derived from the primary
navy, for the outer signal so all three strokes remain visibly distinct;
mint and sky are not adjusted. The canonical light mark retains the exact
approved navy, mint, and sky values.

## Ownership and license posture

The SVG artwork and the generation script in this directory are original
repository-authored work. They do not contain third-party logos, font files,
or raster source artwork. As authorized for the Signal Chorus identity, these
brand source assets and their generated derivatives are dedicated to the public
domain under [CC0 1.0 Universal](https://creativecommons.org/publicdomain/zero/1.0/).
This dedication applies to the artwork files in this directory; it does not
change licensing for other repository code or third-party dependencies.
