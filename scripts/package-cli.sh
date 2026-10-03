#!/bin/bash
# Put the terminal app together into one archive:
#   scripts/package-cli.sh PLATFORM VERSION SCANNER
# PLATFORM is macos-universal, linux-x64, linux-arm64 or windows-x64; SCANNER is
# the built frisyscan binary. Writes build/frisy-cli-VERSION-PLATFORM.(tar.gz|zip).
set -euo pipefail
cd "$(dirname "$0")/.."
platform=$1 version=$2 scanner=$3

(cd cli && npm ci --silent && npm run --silent build)
out=build/cli/frisy
rm -rf build/cli
mkdir -p "$out"
cp cli/dist/frisy.mjs LICENSE "$out/"
if [[ $platform == windows-* ]]; then
  cp "$scanner" "$out/frisyscan.exe"
  cp cli/bin/frisy.cmd "$out/"
  archive=build/frisy-cli-$version-$platform.zip
  rm -f "$archive"
  (cd build/cli && powershell -NoProfile -Command "Compress-Archive -Path frisy -DestinationPath ../$(basename "$archive")")
else
  cp "$scanner" "$out/frisyscan"
  cp cli/bin/frisy "$out/"
  chmod +x "$out/frisy" "$out/frisyscan"
  archive=build/frisy-cli-$version-$platform.tar.gz
  tar -czf "$archive" -C build/cli frisy
fi
echo "$archive"
