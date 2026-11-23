#!/usr/bin/env bash
# Render web/public/og.svg to web/public/og.png (1200x630), the Open Graph / social card image.
# Needs rsvg-convert (brew install librsvg) or macOS Quick Look (qlmanage). Run it once, then add the
# og:image and twitter:image tags (see docs/todo.md, section 2).
set -euo pipefail
cd "$(dirname "$0")/../web/public"
if command -v rsvg-convert >/dev/null; then
  rsvg-convert -w 1200 -h 630 og.svg -o og.png
elif command -v qlmanage >/dev/null; then
  tmp="$(mktemp -d)"
  qlmanage -t -s 1200 -o "$tmp" og.svg >/dev/null
  mv "$tmp/og.svg.png" og.png
else
  echo "install librsvg (rsvg-convert) or run this on macOS" >&2
  exit 1
fi
echo "wrote web/public/og.png"
