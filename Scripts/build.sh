#!/bin/bash
# Build FrisyDisk with swiftc (no Xcode or SwiftPM needed).
#   Scripts/build.sh          build everything into build/
#   Scripts/build.sh test     build and run the unit tests
#   Scripts/build.sh app      also assemble build/FrisyDisk.app
#   Scripts/build.sh install  also copy the app to /Applications
set -euo pipefail
cd "$(dirname "$0")/.."
OUT=build
mkdir -p "$OUT"
FLAGS=(-O -wmo -swift-version 5 -target arm64-apple-macos14.0)
[ "$(uname -m)" = x86_64 ] && FLAGS=(-O -wmo -swift-version 5 -target x86_64-apple-macos14.0)

echo "==> FrisyCore"
swiftc "${FLAGS[@]}" -parse-as-library -module-name FrisyCore \
  -emit-module -emit-module-path "$OUT/FrisyCore.swiftmodule" \
  -emit-object -o "$OUT/FrisyCore.o" Sources/FrisyCore/*.swift

echo "==> frisyscan"
swiftc "${FLAGS[@]}" -I "$OUT" "$OUT/FrisyCore.o" Sources/frisyscan/*.swift -o "$OUT/frisyscan"

echo "==> frisytests"
swiftc "${FLAGS[@]}" -I "$OUT" "$OUT/FrisyCore.o" Tests/frisytests/*.swift -o "$OUT/frisytests"
if [ "${1:-}" = test ]; then
  "$OUT/frisytests"
  exit $?
fi

echo "==> FrisyDisk"
swiftc "${FLAGS[@]}" -parse-as-library -I "$OUT" "$OUT/FrisyCore.o" Sources/FrisyDisk/*.swift -o "$OUT/FrisyDisk"

if [ "${1:-}" = app ] || [ "${1:-}" = install ]; then
  APP="$OUT/FrisyDisk.app"
  rm -rf "$APP"
  mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
  cp "$OUT/FrisyDisk" "$APP/Contents/MacOS/FrisyDisk"
  cp "$OUT/frisyscan" "$APP/Contents/MacOS/frisyscan"
  cp Resources/Info.plist "$APP/Contents/Info.plist"
  if [ ! -f "$OUT/AppIcon.icns" ]; then
    echo "==> icon"
    swiftc -O Scripts/make-icon.swift -o "$OUT/make-icon"
    "$OUT/make-icon" "$OUT/AppIcon.iconset"
    iconutil -c icns "$OUT/AppIcon.iconset" -o "$OUT/AppIcon.icns"
  fi
  cp "$OUT/AppIcon.icns" "$APP/Contents/Resources/AppIcon.icns"
  codesign --force --sign - "$APP" >/dev/null
  echo "built $APP"
fi

if [ "${1:-}" = install ]; then
  rm -rf /Applications/FrisyDisk.app
  cp -R "$OUT/FrisyDisk.app" /Applications/FrisyDisk.app
  echo "installed /Applications/FrisyDisk.app"
fi
