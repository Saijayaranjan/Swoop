#!/usr/bin/env bash
# Builds Swoop.app without Xcode: Rust FFI static library → UniFFI Swift bindings → SwiftPM →
# app bundle → ad-hoc codesign. Usage: scripts/build-macos.sh [--release|--debug] [--universal] [--dmg]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
# Replace a file atomically with a NEW inode. Never `cp` over an executable that may be running:
# macOS kills a process whose mapped code pages change underneath it (CODESIGNING "Invalid Page").
replace_file() { local src="$1" dst="$2"; local tmp="$dst.tmp.$$"; cp "$src" "$tmp" && mv -f "$tmp" "$dst"; }
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
# Match the app's minimum macOS (Package.swift) so C code compiled into the Rust library links cleanly.
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-14.0}"

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
APP="$BUILD_DIR/Swoop.app"
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
  cargo build -p swoop-ffi -p swoop-cli --target "$t" ${CARGO_FLAGS[@]+"${CARGO_FLAGS[@]}"}
  LIBS+=("target/$t/$PROFILE/libswoop_ffi.a")
done
mkdir -p "$BUILD_DIR/lib"
if [[ ${#LIBS[@]} -gt 1 ]]; then
  lipo -create "${LIBS[@]}" -output "$BUILD_DIR/lib/libswoop_ffi.a"
  lipo -create $(for t in "${TARGETS[@]}"; do echo "target/$t/$PROFILE/swoop"; done) -output "$BUILD_DIR/swoop"
else
  cp "${LIBS[0]}" "$BUILD_DIR/lib/libswoop_ffi.a"
  cp "target/${TARGETS[0]}/$PROFILE/swoop" "$BUILD_DIR/swoop"
fi

echo "▸ UniFFI Swift bindings"
BINDINGS="$ROOT/apps/macos/Sources/SwoopFFI"
GENERATED="$ROOT/apps/macos/Sources/SwoopKit/Generated"
mkdir -p "$BINDINGS/include" "$GENERATED"
rm -rf "$BUILD_DIR/bindings"
# The bindgen binary is built alongside the library by `cargo build -p swoop-ffi`; reuse it
# instead of compiling the whole workspace again for the host profile.
BINDGEN="target/${TARGETS[0]}/$PROFILE/uniffi-bindgen"
if [[ ! -x "$BINDGEN" ]]; then
  cargo build -p swoop-ffi --bin uniffi-bindgen --target "${TARGETS[0]}" ${CARGO_FLAGS[@]+"${CARGO_FLAGS[@]}"}
fi
"$BINDGEN" generate --library "target/${TARGETS[0]}/$PROFILE/libswoop_ffi.a" --language swift --out-dir "$BUILD_DIR/bindings"
cp "$BUILD_DIR/bindings/swoop_ffi.swift" "$GENERATED/SwoopFFI.swift"
cp "$BUILD_DIR/bindings/swoop_ffiFFI.h" "$BINDINGS/include/swoop_ffiFFI.h"
cp "$BUILD_DIR/bindings/swoop_ffiFFI.modulemap" "$BINDINGS/include/module.modulemap"

echo "▸ Swift package ($PROFILE)"
SWIFT_CONF=$([[ "$PROFILE" == release ]] && echo release || echo debug)
SWIFT_ARCHS=()
if [[ ${#TARGETS[@]} -gt 1 ]]; then SWIFT_ARCHS=(--arch arm64 --arch x86_64); fi
# Link flags for libswoop_ffi.a (and the system frameworks it needs) live in Package.swift, which
# only enables the engine once the artefacts above exist.
( cd apps/macos && swift build -c "$SWIFT_CONF" ${SWIFT_ARCHS[@]+"${SWIFT_ARCHS[@]}"} --product SwoopApp )
SWIFT_BIN="$(cd apps/macos && swift build -c "$SWIFT_CONF" ${SWIFT_ARCHS[@]+"${SWIFT_ARCHS[@]}"} --show-bin-path)"

echo "▸ Bundle"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Helpers" "$APP/Contents/Resources"
replace_file "$SWIFT_BIN/SwoopApp" "$APP/Contents/MacOS/Swoop"
# The CLI / native-messaging host lives in Contents/Helpers: on the default case-insensitive APFS
# volume `MacOS/swoop` and `MacOS/Swoop` would be the same file.
replace_file "$BUILD_DIR/swoop" "$APP/Contents/Helpers/swoop"
cp apps/macos/Resources/Swoop.icns "$APP/Contents/Resources/"
for lproj in apps/macos/Resources/Localization/*.lproj; do
  cp -R "$lproj" "$APP/Contents/Resources/"
done
if [[ -d "$SWIFT_BIN/SwoopApp_SwoopApp.bundle" ]]; then cp -R "$SWIFT_BIN/SwoopApp_SwoopApp.bundle" "$APP/Contents/Resources/"; fi
if [[ -d "$SWIFT_BIN/SwoopKit_SwoopKit.bundle" ]]; then cp -R "$SWIFT_BIN/SwoopKit_SwoopKit.bundle" "$APP/Contents/Resources/"; fi
sed -e "s/__VERSION__/$VERSION/g" -e "s/__BUILD__/$(date +%Y%m%d%H%M)/g" apps/macos/Resources/Info.plist > "$APP/Contents/Info.plist"
echo -n "APPL????" > "$APP/Contents/PkgInfo"

echo "▸ Codesign (ad-hoc unless SWOOP_SIGN_IDENTITY is set)"
IDENTITY="${SWOOP_SIGN_IDENTITY:--}"
codesign --force --options runtime --sign "$IDENTITY" "$APP/Contents/Helpers/swoop"
codesign --force --options runtime --entitlements apps/macos/Resources/Swoop.entitlements --sign "$IDENTITY" "$APP"
codesign --verify --deep --strict "$APP" && echo "  signed: $APP"

if [[ $MAKE_DMG == 1 ]]; then
  echo "▸ DMG"
  scripts/make-dmg.sh "$APP" "$BUILD_DIR/Swoop-$VERSION.dmg"
fi
echo "✓ $APP"
