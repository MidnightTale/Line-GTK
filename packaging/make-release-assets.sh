#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VER="$(grep -m1 '^version' "$ROOT/Cargo.toml" | cut -d'"' -f2)"
ARCH="$(uname -m)"

env DENO="${DENO:-deno}" "$ROOT/packaging/make-prebuild.sh"
env DENO="${DENO:-deno}" "$ROOT/packaging/make-flatpak.sh"

PREBUILD="/tmp/line-gtk-${VER}-x86_64.tar.gz"
FLATPAK="/tmp/line-gtk-${VER}-${ARCH}.flatpak"

echo
echo "Release assets are ready:"
sha256sum "$PREBUILD" "$FLATPAK"
echo
echo "After committing, pushing, and tagging, publish with:"
echo "  gh release create v${VER} \"$PREBUILD\" \"$FLATPAK\" --title \"v${VER}\" --generate-notes"
