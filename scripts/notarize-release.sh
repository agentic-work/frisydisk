#!/bin/bash
# Notarise and staple the macOS DMG that CI attached to a GitHub release, then
# put it back on the release. CI has already Developer ID signed the app.
#   scripts/notarize-release.sh v0.2.0
# Uses the same credentials file as release-macos.sh.
set -euo pipefail
cd "$(dirname "$0")/.."
TAG="${1:?usage: notarize-release.sh TAG}"
ENV_FILE="${FRISY_SIGNING_ENV:-$HOME/.studiorig-signing/macos.env}"
set -a; . "$ENV_FILE"; set +a
KC="$STUDIORIG_KEYCHAIN"
security unlock-keychain -p "$STUDIORIG_KEYCHAIN_PASSWORD" "$KC"
step() { printf '\n==> %s\n' "$*"; }
notarize() {
  xcrun notarytool submit "$1" --keychain-profile "$APPLE_KEYCHAIN_PROFILE" --keychain "$KC" --wait | tee build/notary.log
  grep -q "status: Accepted" build/notary.log || { echo "notarisation was not accepted"; exit 1; }
}

W=build/notarize-$TAG
rm -rf "$W"; mkdir -p "$W/dmg"
step "Download the CI-built DMG"
gh release download "$TAG" -p '*macos*.dmg' -D "$W"
DMG=$(ls "$W"/*.dmg)
MNT=$(hdiutil attach -nobrowse -readonly "$DMG" | tail -1 | awk -F'\t' '{print $NF}')
ditto "$MNT/FrisyDisk.app" "$W/dmg/FrisyDisk.app"
hdiutil detach -quiet "$MNT"
APP="$W/dmg/FrisyDisk.app"

step "Check the CI signature"
codesign --verify --deep --strict "$APP"
codesign -dvv "$APP" 2>&1 | grep -E "Authority=Developer ID Application|flags=.*runtime" | head -2
lipo -archs "$APP/Contents/MacOS/frisydisk"

step "Notarise and staple the app"
ditto -c -k --keepParent "$APP" "$W/app.zip"
notarize "$W/app.zip"
xcrun stapler staple "$APP"

step "Rebuild, sign, notarise and staple the DMG"
ln -s /Applications "$W/dmg/Applications"
OUT="$W/out/$(basename "$DMG")"
mkdir -p "$W/out"
hdiutil create -volname FrisyDisk -srcfolder "$W/dmg" -ov -format UDZO "$OUT" >/dev/null
codesign --force --sign "$APPLE_SIGNING_IDENTITY" --keychain "$KC" --timestamp "$OUT"
notarize "$OUT"
xcrun stapler staple "$OUT"
spctl --assess --type open --context context:primary-signature -vv "$OUT"
spctl --assess --type execute -vv "$APP"

step "Replace the DMG on the release"
gh release upload "$TAG" "$OUT" --clobber
echo "done: $OUT"
