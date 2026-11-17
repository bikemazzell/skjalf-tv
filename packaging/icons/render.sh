#!/bin/sh
# Render the committed PNG assets from the SVG sources. Needs rsvg-convert and the Ubuntu font.
set -eu
cd "$(dirname "$0")"
for size in 32 48 64 128 256 512; do
    rsvg-convert -w "$size" -h "$size" skjalf.svg -o "skjalf-$size.png"
done
for art in capsule header hero logo; do
    rsvg-convert "$art.svg" -o "$art.png"
done
