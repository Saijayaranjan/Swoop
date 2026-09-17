#!/usr/bin/env bash
# Builds Osprey.app without Xcode: Rust FFI static library → UniFFI Swift bindings → SwiftPM →
# app bundle → ad-hoc codesign. Usage: scripts/build-macos.sh [--release|--debug] [--universal] [--dmg]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"

PROFILE=release
UNIVERSAL=0
MAKE_DMG=0
for arg in "$@"; do
  case "$arg" in
    --debug) PROFILE=debug ;;
    --release) PROFILE=release ;;
    --universal) UNIVERSAL=1 ;;
    --dmg) MAKE_DMG=1 ;;
    *) echo "unknown arg $arg" >&2; exit 2 ;;
  esac
done

VERSION="$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)"/\1/')"
BUILD_DIR="$ROOT/build"
APP="$BUILD_DIR/Osprey.app"
CARGO_FLAGS=()
[[ "$PROFILE" == release ]] && CARGO_FLAGS+=(--release)

echo "▸ Rust FFI library ($PROFILE)"
TARGETS=(aarch64-apple-darwin)
if [[ $UNIVERSAL == 1 ]]; then
  if rustup target list --installed | grep -q x86_64-apple-darwin; then
    TARGETS+=(x86_64-apple-darwin)
  else
    echo "  x86_64-apple-darwin target not installed; building arm64 only" >&2
  fi
fi
LIBS=()
for t in "${TARGETS[@]}"; do
  cargo build -p osprey-ffi -p osprey-cli --target "$t" "${CARGO_FLAGS[@]}"
  LIBS+=("target/$t/$PROFILE/libosprey_ffi.a")
done
mkdir -p "$BUILD_DIR/lib"
if [[ ${#LIBS[@]} -gt 1 ]]; then
  lipo -create "${LIBS[@]}" -output "$BUILD_DIR/lib/libosprey_ffi.a"
  lipo -create $(for t in "${TARGETS[@]}"; do echo "target/$t/$PROFILE/osprey"; done) -output "$BUILD_DIR/osprey"
else
  cp "${LIBS[0]}" "$BUILD_DIR/lib/libosprey_ffi.a"
  cp "target/${TARGETS[0]}/$PROFILE/osprey" "$BUILD_DIR/osprey"
fi

echo "▸ UniFFI Swift bindings"
BINDINGS="$ROOT/apps/macos/Sources/OspreyFFI"
mkdir -p "$BINDINGS/include"
cargo run -q -p osprey-ffi --bin uniffi-bindgen -- generate \
  --library "$BUILD_DIR/lib/libosprey_ffi.a" --language swift --out-dir "$BUILD_DIR/bindings"
cp "$BUILD_DIR/bindings/osprey_ffi.swift" "$ROOT/apps/macos/Sources/OspreyKit/Generated/OspreyFFI.swift" 2>/dev/null || {
  mkdir -p "$ROOT/apps/macos/Sources/OspreyKit/Generated"; cp "$BUILD_DIR/bindings/osprey_ffi.swift" "$ROOT/apps/macos/Sources/OspreyKit/Generated/OspreyFFI.swift"; }
cp "$BUILD_DIR/bindings/osprey_ffiFFI.h" "$BINDINGS/include/osprey_ffiFFI.h"
cp "$BUILD_DIR/bindings/osprey_ffiFFI.modulemap" "$BINDINGS/include/module.modulemap"

echo "▸ Swift package ($PROFILE)"
SWIFT_CONF=$([[ "$PROFILE" == release ]] && echo release || echo debug)
SWIFT_ARCHS=()
if [[ ${#TARGETS[@]} -gt 1 ]]; then SWIFT_ARCHS=(--arch arm64 --arch x86_64); fi
( cd apps/macos && swift build -c "$SWIFT_CONF" "${SWIFT_ARCHS[@]}" -Xlinker -L"$BUILD_DIR/lib" -Xlinker -losprey_ffi )
SWIFT_BIN="$(cd apps/macos && swift build -c "$SWIFT_CONF" "${SWIFT_ARCHS[@]}" --show-bin-path)"

echo "▸ Bundle"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$SWIFT_BIN/OspreyApp" "$APP/Contents/MacOS/Osprey"
cp "$BUILD_DIR/osprey" "$APP/Contents/MacOS/osprey"
cp apps/macos/Resources/Osprey.icns "$APP/Contents/Resources/"
if [[ -d "$SWIFT_BIN/OspreyApp_OspreyApp.bundle" ]]; then cp -R "$SWIFT_BIN/OspreyApp_OspreyApp.bundle" "$APP/Contents/Resources/"; fi
if [[ -d "$SWIFT_BIN/OspreyKit_OspreyKit.bundle" ]]; then cp -R "$SWIFT_BIN/OspreyKit_OspreyKit.bundle" "$APP/Contents/Resources/"; fi
sed -e "s/__VERSION__/$VERSION/g" -e "s/__BUILD__/$(date +%Y%m%d%H%M)/g" apps/macos/Resources/Info.plist > "$APP/Contents/Info.plist"
echo -n "APPL????" > "$APP/Contents/PkgInfo"

echo "▸ Codesign (ad-hoc unless OSPREY_SIGN_IDENTITY is set)"
IDENTITY="${OSPREY_SIGN_IDENTITY:--}"
codesign --force --deep --options runtime --entitlements apps/macos/Resources/Osprey.entitlements --sign "$IDENTITY" "$APP"
codesign --verify --deep --strict "$APP" && echo "  signed: $APP"

if [[ $MAKE_DMG == 1 ]]; then
  echo "▸ DMG"
  scripts/make-dmg.sh "$APP" "$BUILD_DIR/Osprey-$VERSION.dmg"
fi
echo "✓ $APP"
