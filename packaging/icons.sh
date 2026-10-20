#!/usr/bin/env bash
# Regenerate every derived app icon from assets/app-icon/lumen.svg (the master; a 512-unit
# full-bleed tile with rx=112) and its bolder variants for small sizes: lumen-small.svg (32-40
# px) and lumen-tiny.svg (16-24 px).
#
# Needs: resvg (brew install resvg / cargo install resvg). On macOS, iconutil also writes the
# .icns. The outputs are committed, so builds and packaging never need these tools.
#
#   packaging/icons.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIR="$ROOT/assets/app-icon"
SVG="$DIR/lumen.svg"
SMALL="$DIR/lumen-small.svg"
TINY="$DIR/lumen-tiny.svg"
APP_ID="net.sifg.lumen"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

command -v resvg >/dev/null || { echo "error: resvg not found (brew install resvg)" >&2; exit 1; }

# macOS icons follow Apple's grid: the tile body is 824 of 1024 px with a transparent margin, so
# the Dock shows it at the same size as other apps. 512 * 1024 / 824 = 636.27 units, margin 62.14.
mac() {
  sed 's/viewBox="0 0 512 512"/viewBox="-62.14 -62.14 636.27 636.27"/' "$1" >"$2"
  grep -q 'viewBox="-62.14' "$2" || { echo "error: unexpected viewBox in $1" >&2; exit 1; }
}
MAC="$TMP/mac.svg"
mac "$SVG" "$MAC"
mac "$SMALL" "$TMP/mac-small.svg"
mac "$TINY" "$TMP/mac-tiny.svg"

render() { resvg -w "$2" -h "$2" "$1" "$3" </dev/null; }
# The artwork for a size: the tiny variant up to 24 px, the small one up to 40 px, else the master.
pick() { if [ "$1" -le 24 ]; then echo "$TINY"; elif [ "$1" -le 40 ]; then echo "$SMALL"; else echo "$SVG"; fi; }
pick_mac() { if [ "$1" -le 24 ]; then echo "$TMP/mac-tiny.svg"; elif [ "$1" -le 40 ]; then echo "$TMP/mac-small.svg"; else echo "$MAC"; fi; }

render "$SVG" 1024 "$DIR/lumen-1024.png"
# Runtime window/Dock icon on macOS (apps/lumen/src/main.rs embeds it).
render "$MAC" 512 "$DIR/lumen-macos-512.png"

# Linux hicolor theme. Windows and Linux use the full-bleed tile.
for s in 16 24 32 48 64 128 256 512; do
  mkdir -p "$DIR/hicolor/${s}x${s}/apps"
  render "$(pick "$s")" "$s" "$DIR/hicolor/${s}x${s}/apps/$APP_ID.png"
done
mkdir -p "$DIR/hicolor/scalable/apps"
cp "$SVG" "$DIR/hicolor/scalable/apps/$APP_ID.svg"

# Web app favicon and loading image.
render "$SVG" 128 "$ROOT/apps/lumen-web/web/favicon.png"

# Windows .ico.
ICO_PNGS=()
for s in 16 20 24 32 40 48 64 128 256; do
  render "$(pick "$s")" "$s" "$TMP/ico-$s.png"
  ICO_PNGS+=("$TMP/ico-$s.png")
done
(cd "$ROOT" && cargo run -q -p xtask -- ico "$DIR/lumen.ico" "${ICO_PNGS[@]}")

# macOS .icns.
if command -v iconutil >/dev/null; then
  SET="$TMP/lumen.iconset"
  mkdir -p "$SET"
  for s in 16 32 128 256 512; do
    render "$(pick_mac "$s")" "$s" "$SET/icon_${s}x${s}.png"
    render "$(pick_mac $((s * 2)))" $((s * 2)) "$SET/icon_${s}x${s}@2x.png"
  done
  iconutil -c icns -o "$DIR/lumen.icns" "$SET"
else
  echo "warning: iconutil not found (macOS only); lumen.icns not regenerated" >&2
fi
echo "icons written to $DIR"
