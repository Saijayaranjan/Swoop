#!/usr/bin/env bash
# Creates a compressed DMG with an Applications symlink. Usage: make-dmg.sh <App.app> <out.dmg>
set -euo pipefail
APP="$1"; OUT="$2"
STAGE="$(mktemp -d)"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
rm -f "$OUT"
hdiutil create -volname "Osprey" -srcfolder "$STAGE" -ov -format UDZO -imagekey zlib-level=9 "$OUT" >/dev/null
rm -rf "$STAGE"
echo "  $OUT"
