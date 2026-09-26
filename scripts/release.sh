#!/usr/bin/env bash
# Cuts a Swoop release for GitHub Releases, which is where the in-app updater looks.
#
#   scripts/release.sh <X.Y.Z> [--publish] [--no-build]
#
# 1. Sets the workspace version in Cargo.toml (Info.plist and the CLI take it from there).
# 2. Builds the universal app and DMG: build/Swoop-X.Y.Z.dmg.
# 3. Signs the DMG with the release key (~/.config/swoop/update-signing-key, or
#    $SWOOP_UPDATE_SIGNING_KEY): build/Swoop-X.Y.Z.dmg.sig, a base64 Ed25519 signature over the
#    DMG's SHA-256. Then verifies it with the key compiled into the app just built.
# 4. Writes build/release-notes-X.Y.Z.md from scripts/release-notes-template.md (edit it).
# 5. Prints the git and `gh release create` commands. With --publish it runs `gh release create`
#    (the tag must already be pushed); without it, nothing leaves this machine.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"

die() { echo "release: $*" >&2; exit 1; }

VERSION=""
PUBLISH=0
BUILD=1
for arg in "$@"; do
  case "$arg" in
    --publish) PUBLISH=1 ;;
    --no-build) BUILD=0 ;;
    -*) die "unknown option $arg" ;;
    *) [[ -z "$VERSION" ]] || die "one version only"; VERSION="${arg#v}" ;;
  esac
done
[[ -n "$VERSION" ]] || die "usage: scripts/release.sh <X.Y.Z> [--publish] [--no-build]"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || die "$VERSION isn't a semver version (X.Y.Z or X.Y.Z-beta.N)"

# The owner/repo the updater queries lives in one Rust constant; read it from there.
REPO="$(sed -n 's/^pub const GITHUB_REPOSITORY: &str = "\(.*\)";/\1/p' crates/swoop-update/src/github.rs)"
[[ -n "$REPO" ]] || die "couldn't read GITHUB_REPOSITORY from crates/swoop-update/src/github.rs"

KEY="${SWOOP_UPDATE_SIGNING_KEY:-$HOME/.config/swoop/update-signing-key}"
[[ -f "$KEY" ]] || die "release signing key not found at $KEY (see the Releasing section of CONTRIBUTING.md)"
if [[ "$(stat -f %Lp "$KEY")" != "600" ]]; then
  echo "release: warning: $KEY should be chmod 600" >&2
fi

TAG="v$VERSION"
DMG="build/Swoop-$VERSION.dmg"
SIG="$DMG.sig"
NOTES="build/release-notes-$VERSION.md"
CLI="build/Swoop.app/Contents/Helpers/swoop"

# 1. version
CURRENT="$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)"/\1/')"
if [[ "$CURRENT" != "$VERSION" ]]; then
  echo "▸ Version $CURRENT → $VERSION"
  # Only the first `version = ` line: the [workspace.package] one.
  sed -i '' "1,/^version = \".*\"/s/^version = \".*\"/version = \"$VERSION\"/" Cargo.toml
  [[ "$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)"/\1/')" == "$VERSION" ]] || die "couldn't set the version in Cargo.toml"
else
  echo "▸ Version $VERSION (unchanged)"
fi

# 2. build
if [[ $BUILD == 1 ]]; then
  if pgrep -f "scripts/build-macos.sh" >/dev/null; then die "another build-macos.sh is running; try again when it finishes"; fi
  scripts/build-macos.sh --release --universal --dmg
fi
[[ -f "$DMG" ]] || die "$DMG not found (build first, or drop --no-build)"
[[ -x "$CLI" ]] || die "$CLI not found"
BUILT="$(/usr/bin/plutil -extract CFBundleShortVersionString raw -o - build/Swoop.app/Contents/Info.plist)"
[[ "$BUILT" == "$VERSION" ]] || die "build/Swoop.app is version $BUILT, not $VERSION"

# 3. sign + verify with the key the app will use
echo "▸ Sign"
"$CLI" update sign --key "$KEY" "$DMG" --out "$SIG" >/dev/null
"$CLI" update verify "$DMG" --sig "$SIG"

# 4. notes
if [[ ! -f "$NOTES" ]]; then
  sed -e "s/__VERSION__/$VERSION/g" -e "s#__REPO__#$REPO#g" scripts/release-notes-template.md > "$NOTES"
  echo "▸ Wrote $NOTES: edit it before publishing"
else
  echo "▸ Keeping existing $NOTES"
fi

PRERELEASE=()
[[ "$VERSION" == *-* ]] && PRERELEASE=(--prerelease)
GH=(gh release create "$TAG" "$DMG" "$SIG" --repo "$REPO" --title "Swoop $VERSION" --notes-file "$NOTES" --verify-tag ${PRERELEASE[@]+"${PRERELEASE[@]}"})

echo
echo "✓ $DMG"
echo "✓ $SIG"
shasum -a 256 "$DMG"
echo
echo "Next steps:"
echo "  git add Cargo.toml Cargo.lock && git commit -m \"Release $VERSION\"   # if the version changed"
echo "  git tag -a $TAG -m \"Swoop $VERSION\""
echo "  git push origin main $TAG"
printf '  '; printf '%q ' "${GH[@]}"; echo

if [[ $PUBLISH == 1 ]]; then
  command -v gh >/dev/null || die "gh isn't installed"
  echo
  echo "▸ Publishing $TAG to $REPO"
  "${GH[@]}"
fi
