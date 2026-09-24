#!/usr/bin/env bash
# Builds Osprey.app without Xcode: Rust FFI static library → UniFFI Swift bindings → SwiftPM →
# app bundle → ad-hoc codesign. Usage: scripts/build-macos.sh [--release|--debug] [--universal] [--dmg]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
# Replace a file atomically with a NEW inode. Never `cp` over an executable that may be running:
# macOS kills a process whose mapped code pages change underneath it (CODESIGNING "Invalid Page").
replace_file() { local src="$1" dst="$2"; local tmp="$dst.tmp.$$"; cp "$src" "$tmp" && mv -f "$tmp" "$dst"; }
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
  cargo build -p osprey-ffi -p osprey-cli --target "$t" ${CARGO_FLAGS[@]+"${CARGO_FLAGS[@]}"}
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
GENERATED="$ROOT/apps/macos/Sources/OspreyKit/Generated"
mkdir -p "$BINDINGS/include" "$GENERATED"
rm -rf "$BUILD_DIR/bindings"
# The bindgen binary is built alongside the library by `cargo build -p osprey-ffi`; reuse it
# instead of compiling the whole workspace again for the host profile.
BINDGEN="target/${TARGETS[0]}/$PROFILE/uniffi-bindgen"
if [[ ! -x "$BINDGEN" ]]; then
  cargo build -p osprey-ffi --bin uniffi-bindgen --target "${TARGETS[0]}" ${CARGO_FLAGS[@]+"${CARGO_FLAGS[@]}"}
fi
"$BINDGEN" generate --library "target/${TARGETS[0]}/$PROFILE/libosprey_ffi.a" --language swift --out-dir "$BUILD_DIR/bindings"
cp "$BUILD_DIR/bindings/osprey_ffi.swift" "$GENERATED/OspreyFFI.swift"
cp "$BUILD_DIR/bindings/osprey_ffiFFI.h" "$BINDINGS/include/osprey_ffiFFI.h"
cp "$BUILD_DIR/bindings/osprey_ffiFFI.modulemap" "$BINDINGS/include/module.modulemap"

echo "▸ Swift package ($PROFILE)"
SWIFT_CONF=$([[ "$PROFILE" == release ]] && echo release || echo debug)
SWIFT_ARCHS=()
if [[ ${#TARGETS[@]} -gt 1 ]]; then SWIFT_ARCHS=(--arch arm64 --arch x86_64); fi
# Link flags for libosprey_ffi.a (and the system frameworks it needs) live in Package.swift, which
# only enables the engine once the artefacts above exist.
( cd apps/macos && swift build -c "$SWIFT_CONF" ${SWIFT_ARCHS[@]+"${SWIFT_ARCHS[@]}"} --product OspreyApp )
SWIFT_BIN="$(cd apps/macos && swift build -c "$SWIFT_CONF" ${SWIFT_ARCHS[@]+"${SWIFT_ARCHS[@]}"} --show-bin-path)"

echo "▸ Bundle"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Helpers" "$APP/Contents/Resources"
replace_file "$SWIFT_BIN/OspreyApp" "$APP/Contents/MacOS/Osprey"
# The CLI / native-messaging host lives in Contents/Helpers: on the default case-insensitive APFS
# volume `MacOS/osprey` and `MacOS/Osprey` would be the same file.
replace_file "$BUILD_DIR/osprey" "$APP/Contents/Helpers/osprey"
cp apps/macos/Resources/Osprey.icns "$APP/Contents/Resources/"
for lproj in apps/macos/Resources/Localization/*.lproj; do
  cp -R "$lproj" "$APP/Contents/Resources/"
done
if [[ -d "$SWIFT_BIN/OspreyApp_OspreyApp.bundle" ]]; then cp -R "$SWIFT_BIN/OspreyApp_OspreyApp.bundle" "$APP/Contents/Resources/"; fi
if [[ -d "$SWIFT_BIN/OspreyKit_OspreyKit.bundle" ]]; then cp -R "$SWIFT_BIN/OspreyKit_OspreyKit.bundle" "$APP/Contents/Resources/"; fi
sed -e "s/__VERSION__/$VERSION/g" -e "s/__BUILD__/$(date +%Y%m%d%H%M)/g" apps/macos/Resources/Info.plist > "$APP/Contents/Info.plist"
echo -n "APPL????" > "$APP/Contents/PkgInfo"

echo "▸ Codesign (ad-hoc unless OSPREY_SIGN_IDENTITY is set)"
IDENTITY="${OSPREY_SIGN_IDENTITY:--}"
codesign --force --options runtime --sign "$IDENTITY" "$APP/Contents/Helpers/osprey"
codesign --force --options runtime --entitlements apps/macos/Resources/Osprey.entitlements --sign "$IDENTITY" "$APP"
codesign --verify --deep --strict "$APP" && echo "  signed: $APP"

if [[ $MAKE_DMG == 1 ]]; then
  echo "▸ DMG"
  scripts/make-dmg.sh "$APP" "$BUILD_DIR/Osprey-$VERSION.dmg"
fi
echo "✓ $APP"
