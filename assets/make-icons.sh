#!/usr/bin/env bash
# Builds every icon from assets/logo.svg (64 px and up) and
# assets/logo-small.svg (below 64 px). Needs rsvg-convert, python3 + Pillow,
# and on macOS iconutil. Outputs go to assets/icons/ and web/.
set -euo pipefail
cd "$(dirname "$0")"
out=icons
mkdir -p "$out"
png() { # size file
  if [ "$1" -lt 64 ]; then src=logo-small.svg; else src=logo.svg; fi
  rsvg-convert -w "$1" -h "$1" "$src" -o "$2"
}
for s in 16 24 32 48 64 128 256 512 1024; do png "$s" "$out/everywhere-$s.png"; done

# Windows .ico (all sizes in one file) and the raw 64 px pixels the app
# window uses as its icon.
python3 - "$out" <<'PY'
import sys
from PIL import Image
d = sys.argv[1]
imgs = [Image.open(f"{d}/everywhere-{s}.png").convert("RGBA") for s in (16, 24, 32, 48, 64, 128, 256)]
imgs[-1].save(f"{d}/everywhere.ico", sizes=[i.size for i in imgs], append_images=imgs[:-1])
open(f"{d}/window-64.rgba", "wb").write(Image.open(f"{d}/everywhere-64.png").convert("RGBA").tobytes())
PY

# macOS .icns
if command -v iconutil >/dev/null; then
  set_dir="$out/Everywhere.iconset"
  rm -rf "$set_dir"; mkdir -p "$set_dir"
  for s in 16 32 128 256 512; do
    png "$s" "$set_dir/icon_${s}x${s}.png"
    png "$((s * 2))" "$set_dir/icon_${s}x${s}@2x.png"
  done
  iconutil -c icns "$set_dir" -o "$out/Everywhere.icns"
  rm -rf "$set_dir"
fi

# Website
cp logo-small.svg ../web/favicon.svg
png 32 ../web/favicon-32.png
png 180 ../web/apple-touch-icon.png
echo "icons written"
