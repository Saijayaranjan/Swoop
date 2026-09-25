#!/usr/bin/env bash
# Creates the installer DMG: a styled window (rendered background, icon positions, volume icon)
# with an Applications link. Usage: make-dmg.sh <App.app> <out.dmg>
#
# The styled layout is written with dmgbuild (`pip install dmgbuild`; found on PATH or as a python3
# module). Without it the script falls back to a plain compressed image with the same contents.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="$1"; OUT="$2"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
rm -f "$OUT"

swift "$ROOT/scripts/render-dmg-background.swift" "$WORK/background" >/dev/null
tiffutil -cathidpicheck "$WORK/background.png" "$WORK/background@2x.png" -out "$WORK/background.tiff" >/dev/null 2>&1

ICON="$APP/Contents/Resources/Swoop.icns"
if command -v dmgbuild >/dev/null 2>&1; then
  DMGBUILD=(dmgbuild)
elif python3 -c "import dmgbuild" >/dev/null 2>&1; then
  DMGBUILD=(python3 -m dmgbuild)
else
  DMGBUILD=()
fi

if [[ ${#DMGBUILD[@]} -gt 0 ]]; then
  "${DMGBUILD[@]}" -s "$ROOT/scripts/dmg-settings.py" \
    -D app="$APP" -D background="$WORK/background.tiff" -D icon="$ICON" \
    "Swoop" "$OUT" >/dev/null
else
  echo "  dmgbuild not found: building an unstyled DMG" >&2
  STAGE="$WORK/stage"
  mkdir -p "$STAGE"
  cp -R "$APP" "$STAGE/"
  ln -s /Applications "$STAGE/Applications"
  cp "$ICON" "$STAGE/.VolumeIcon.icns"
  hdiutil create -volname "Swoop" -srcfolder "$STAGE" -ov -format UDZO -imagekey zlib-level=9 "$OUT" >/dev/null
fi
echo "  $OUT"
