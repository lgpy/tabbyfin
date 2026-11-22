#!/usr/bin/env bash
# Regenerate the website's images (site/assets/) from their sources: the README's screenshots
# as WebP at two widths, and the icons from the app's. Run it after replacing a screenshot in
# docs/screenshots/ or changing the icon.
#
#   bash dev/site-assets.sh
#
# Needs ImageMagick (`magick`) on PATH. The social card (site/assets/og.jpg) isn't made here:
# see dev/README.md.
set -euo pipefail

cd "$(dirname -- "$0")/.."
out=site/assets
mkdir -p "$out/screenshots"

for shot in docs/screenshots/*.jpg; do
  name=$(basename "$shot" .jpg)
  magick "$shot" -quality 82 -define webp:method=6 "$out/screenshots/$name.webp"
  magick "$shot" -resize 640x -quality 80 -define webp:method=6 "$out/screenshots/$name-640.webp"
done

cp packaging/tabbyfin.svg "$out/icon.svg"
magick packaging/tabbyfin.png -resize 32x32 "$out/icon-32.png"
# iOS draws the touch icon on black where it's transparent, so it gets the backdrop's colour.
magick packaging/tabbyfin.png -resize 148x148 -background '#1d1633' -gravity center -extent 180x180 "$out/apple-touch-icon.png"
