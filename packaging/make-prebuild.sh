#!/usr/bin/env bash
# Build a portable prebuilt tarball for GitHub Releases.
# After this finishes, run the printed gh command yourself.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

VER="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
STAGE="line-gtk-${VER}-x86_64"
OUT_DIR="/tmp/${STAGE}"
OUT_TAR="/tmp/${STAGE}.tar.gz"

echo "==> building release binary"
cargo build --release

echo "==> staging ${OUT_DIR}"
rm -rf "$OUT_DIR" "$OUT_TAR"
mkdir -p "$OUT_DIR"
cp -a target/release/line-gtk "$OUT_DIR/"
cp -a protocol assets "$OUT_DIR/"
echo "==> compiling offline protocol sidecar"
./scripts/build-sidecar.sh "$OUT_DIR/protocol/line-gtk-protocol"
cp -a packaging/dev.linegtk.LineGtk.desktop "$OUT_DIR/" 2>/dev/null || true
cp -a README.md LICENSE "$OUT_DIR/" 2>/dev/null || true
rm -rf "$OUT_DIR/protocol/node_modules" "$OUT_DIR/protocol/.deno" 2>/dev/null || true

echo "==> packing ${OUT_TAR}"
tar -C /tmp -czf "$OUT_TAR" "$STAGE"
sha256sum "$OUT_TAR"
./packaging/check-package.sh "$OUT_TAR"

echo
echo "Done. Asset ready:"
echo "  $OUT_TAR"
echo
echo "If release v${VER} does not exist yet:"
echo "  gh release create v${VER} \"$OUT_TAR\" --title \"v${VER}\" --notes \"Prebuilt x86_64 Linux bundle.\""
echo
echo "If release already exists:"
echo "  gh release upload v${VER} \"$OUT_TAR\" --clobber"
