#!/bin/bash
# Build, sign with Developer ID, notarise and staple FrisyDisk, then wrap it in a
# signed, notarised, stapled DMG. Same steps and credentials as StudioRig's
# scripts/sign-macos.mjs.
#
# Credentials come from an env file (default ~/.studiorig-signing/macos.env) that sets:
#   APPLE_SIGNING_IDENTITY     "Developer ID Application: Name (TEAMID)"
#   APPLE_KEYCHAIN_PROFILE     notarytool profile from `notarytool store-credentials`
#   STUDIORIG_KEYCHAIN         keychain holding the identity and the profile
#   STUDIORIG_KEYCHAIN_PASSWORD
#
#   Scripts/release-macos.sh            build, sign, notarise, staple, make DMG
#   Scripts/release-macos.sh install    also copy the stapled app to /Applications
set -euo pipefail
cd "$(dirname "$0")/.."

ENV_FILE="${FRISY_SIGNING_ENV:-$HOME/.studiorig-signing/macos.env}"
[ -f "$ENV_FILE" ] || { echo "missing $ENV_FILE (see header of this script)"; exit 1; }
set -a; . "$ENV_FILE"; set +a
: "${APPLE_SIGNING_IDENTITY:?}" "${APPLE_KEYCHAIN_PROFILE:?}" "${STUDIORIG_KEYCHAIN:?}" "${STUDIORIG_KEYCHAIN_PASSWORD:?}"
KEYCHAIN="$STUDIORIG_KEYCHAIN"

step() { printf '\n==> %s\n' "$*"; }

step "Build"
Scripts/build.sh app
APP=build/FrisyDisk.app
VERSION=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP/Contents/Info.plist")
DMG="build/FrisyDisk-$VERSION.dmg"

step "Unlock signing keychain"
security unlock-keychain -p "$STUDIORIG_KEYCHAIN_PASSWORD" "$KEYCHAIN"

step "Strip extended attributes"
xattr -cr "$APP"

sign() {
  codesign --force --sign "$APPLE_SIGNING_IDENTITY" --keychain "$KEYCHAIN" \
    --options runtime --timestamp --entitlements Resources/entitlements.plist "$@"
}

step "Sign (inside out, hardened runtime, secure timestamp)"
sign "$APP/Contents/MacOS/frisyscan"
sign "$APP"

step "Verify signature"
codesign --verify --deep --strict --verbose=2 "$APP"
DETAILS=$(codesign -dvv "$APP" 2>&1)
grep -E "Authority=Developer ID|TeamIdentifier|Timestamp|flags=" <<<"$DETAILS"
grep -q "flags=.*runtime" <<<"$DETAILS" || { echo "hardened runtime is off"; exit 1; }

notarize() {
  xcrun notarytool submit "$1" --keychain-profile "$APPLE_KEYCHAIN_PROFILE" --keychain "$KEYCHAIN" --wait \
    | tee build/notary.log
  grep -q "status: Accepted" build/notary.log || {
    id=$(awk '/^ *id:/{print $2; exit}' build/notary.log)
    [ -n "$id" ] && xcrun notarytool log "$id" --keychain-profile "$APPLE_KEYCHAIN_PROFILE" --keychain "$KEYCHAIN" || true
    echo "notarisation was not accepted"; exit 1
  }
}

step "Notarise the app"
rm -f build/FrisyDisk-notarize.zip
ditto -c -k --keepParent "$APP" build/FrisyDisk-notarize.zip
notarize build/FrisyDisk-notarize.zip
rm -f build/FrisyDisk-notarize.zip

step "Staple the app"
xcrun stapler staple "$APP"
xcrun stapler validate "$APP"
spctl --assess --type execute --verbose=2 "$APP"

step "Build, sign, notarise and staple the DMG"
rm -rf build/dmg "$DMG"
mkdir -p build/dmg
cp -R "$APP" build/dmg/
ln -s /Applications build/dmg/Applications
hdiutil create -volname "FrisyDisk" -srcfolder build/dmg -ov -format UDZO "$DMG" >/dev/null
rm -rf build/dmg
codesign --force --sign "$APPLE_SIGNING_IDENTITY" --keychain "$KEYCHAIN" --timestamp "$DMG"
notarize "$DMG"
xcrun stapler staple "$DMG"
spctl --assess --type open --context context:primary-signature --verbose=2 "$DMG"

if [ "${1:-}" = install ]; then
  step "Install"
  rm -rf /Applications/FrisyDisk.app
  ditto "$APP" /Applications/FrisyDisk.app
  echo "installed /Applications/FrisyDisk.app"
fi
echo
echo "done: $APP and $DMG"
