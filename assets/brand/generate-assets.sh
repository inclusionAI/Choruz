#!/bin/zsh
set -euo pipefail

script_dir=${0:A:h}
repository_root=${script_dir:h:h}
light_source_svg="$script_dir/signal-chorus-mark.svg"
light_lockup_svg="$script_dir/signal-chorus-lockup.svg"
dark_lockup_svg="$script_dir/signal-chorus-lockup-dark.svg"
output_dir="$script_dir/generated"
work_dir=$(mktemp -d "${TMPDIR:-/tmp}/choruz-brand.XXXXXX")
trap 'rm -rf "$work_dir"' EXIT

apply=false
if (( $# > 1 )) || { (( $# == 1 )) && [[ $1 != --apply ]]; }; then
  print -u2 "Usage: $0 [--apply]"
  exit 2
fi
(( $# == 1 )) && apply=true

for required_command in qlmanage sips node; do
  command -v "$required_command" >/dev/null || {
    print -u2 "Missing required command: $required_command"
    exit 1
  }
done

rm -rf "$output_dir"
mkdir -p "$output_dir/web"

# Quick Look writes eXIf/XMP payloads into PNGs. Retain only PNG's critical
# display chunks so source paths and stale raster metadata never reach assets.
strip_png_metadata() {
  local image_path=$1
  node --input-type=module - "$image_path" <<'NODE'
import { readFileSync, renameSync, writeFileSync } from 'node:fs';
const [imagePath] = process.argv.slice(2);
const input = readFileSync(imagePath);
const signature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
if (!input.subarray(0, 8).equals(signature)) throw new Error(`Not a PNG: ${imagePath}`);
const chunks = [signature];
let offset = 8;
while (offset < input.length) {
  const length = input.readUInt32BE(offset);
  const end = offset + 12 + length;
  if (end > input.length) throw new Error(`Malformed PNG: ${imagePath}`);
  if ((input[offset + 4] & 0x20) === 0) chunks.push(input.subarray(offset, end));
  offset = end;
}
const temporaryPath = `${imagePath}.stripped`;
writeFileSync(temporaryPath, Buffer.concat(chunks));
renameSync(temporaryPath, imagePath);
NODE
}

rasterize_source() {
  local source_svg=$1
  local destination_dir=$2
  mkdir -p "$destination_dir"
  qlmanage -t -s 1024 -o "$destination_dir" "$source_svg" >/dev/null
  local raster_source="$destination_dir/${source_svg:t}.png"
  [[ -f "$raster_source" ]] || { print -u2 "SVG rasterization failed: $source_svg"; exit 1; }
  strip_png_metadata "$raster_source"
  print -r -- "$raster_source"
}

light_raster_source=$(rasterize_source "$light_source_svg" "$work_dir/light")

resize() {
  local raster_source=$1
  local pixels=$2
  local destination=$3
  sips -z "$pixels" "$pixels" "$raster_source" --out "$destination" >/dev/null
  strip_png_metadata "$destination"
}

for pixels in 16 32 180; do
  resize "$light_raster_source" "$pixels" "$output_dir/web/icon-${pixels}.png"
done

print "Generated Signal Chorus assets in $output_dir"

if $apply; then
  web_public_dir="$repository_root/apps/web/public"
  mkdir -p "$web_public_dir/brand"

  cp "$light_source_svg" "$repository_root/apps/web/app/icon.svg"
  cp "$output_dir/web/icon-16.png" "$web_public_dir/favicon-16x16.png"
  cp "$output_dir/web/icon-32.png" "$web_public_dir/favicon-32x32.png"
  cp "$output_dir/web/icon-180.png" "$web_public_dir/apple-touch-icon.png"
  cp "$light_lockup_svg" "$web_public_dir/brand/choruz-lockup.svg"
  cp "$dark_lockup_svg" "$web_public_dir/brand/choruz-lockup-dark.svg"
  print "Applied Signal Chorus assets to the web target"
fi
