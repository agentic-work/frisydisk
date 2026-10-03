#!/bin/sh
# Install the FrisyDisk terminal app (frisy) for this user, on macOS or Linux:
#   curl -fsSL https://raw.githubusercontent.com/agentic-work/frisydisk/main/scripts/install.sh | sh
# Installs into ~/.local/share/frisydisk and links ~/.local/bin/frisy. Nothing needs root.
set -eu

repo=agentic-work/frisydisk
case "$(uname -s)" in
  Darwin) platform=macos-universal ;;
  Linux)
    case "$(uname -m)" in
      x86_64 | amd64) platform=linux-x64 ;;
      aarch64 | arm64) platform=linux-arm64 ;;
      *) echo "frisy: no build for $(uname -m) yet" >&2; exit 1 ;;
    esac ;;
  *) echo "frisy: use install.ps1 on Windows" >&2; exit 1 ;;
esac

fetch() { if command -v curl >/dev/null 2>&1; then curl -fsSL "$1"; else wget -qO- "$1"; fi; }

url=$(fetch "https://api.github.com/repos/$repo/releases/latest" |
  grep -o "\"browser_download_url\": *\"[^\"]*frisy-cli-[^\"]*-$platform\.tar\.gz\"" | head -1 | sed 's/.*"\(https[^"]*\)"/\1/')
[ -n "$url" ] || { echo "frisy: the latest release has no $platform build" >&2; exit 1; }

dest=${FRISY_HOME:-$HOME/.local/share/frisydisk}
bin=${FRISY_BIN:-$HOME/.local/bin}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
echo "Downloading $(basename "$url")"
fetch "$url" > "$tmp/frisy.tar.gz"
tar -xzf "$tmp/frisy.tar.gz" -C "$tmp"
rm -rf "$dest"
mkdir -p "$dest" "$bin"
cp -R "$tmp/frisy/." "$dest/"
ln -sf "$dest/frisy" "$bin/frisy"
echo "Installed frisy to $dest"

case ":$PATH:" in
  *":$bin:"*) ;;
  *) echo "Add $bin to your PATH, for example:  echo 'export PATH=\"$bin:\$PATH\"' >> ~/.profile" ;;
esac
command -v node >/dev/null 2>&1 || echo "Install Node.js 18 or newer for the interactive view (https://nodejs.org); without it frisy prints a summary."
echo "Try:  frisy ~      frisy clean      frisy du -c ~/*"
